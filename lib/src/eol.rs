// Copyright 2025 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::error::Error;
use std::io::Cursor;
use std::io::Read;
use std::io::Write;
use std::pin::Pin;

use bstr::ByteSlice as _;
use thiserror::Error;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;

use crate::config::ConfigGetError;
use crate::config::ConfigValue;
use crate::file_util::copy_async_to_sync;
use crate::local_working_copy::TreeStateSettings;
use crate::settings::UserSettings;

#[derive(Error, Debug)]
#[error("{message}")]
struct EolError {
    message: String,
    #[source]
    source: Option<Box<dyn Error + Send + Sync>>,
}

pub(crate) fn create_target_eol_strategy(
    tree_state_settings: &TreeStateSettings,
) -> TargetEolStrategy {
    TargetEolStrategy {
        eol_conversion_settings: tree_state_settings.eol_conversion_settings,
    }
}

fn is_binary(bytes: &[u8]) -> bool {
    // TODO(06393993): align the algorithm with git so that the git config autocrlf
    // users won't see different decisions on whether a file is binary and needs to
    // perform EOL conversion.
    bytes.contains(&b'\0')
}

pub(crate) struct TargetEolStrategy {
    eol_conversion_settings: EolConversionSettings,
}

impl TargetEolStrategy {
    const PROBE_LIMIT: u64 = 8 << 10;

    pub(crate) fn convert_eol_for_snapshot(
        &self,
        mut contents: impl Read,
        writer: impl Write,
    ) -> Result<(), impl Error + Send + Sync + 'static> {
        let (contents, target_eol) = match self.eol_conversion_settings {
            EolConversionSettings::None => {
                (Box::new(contents) as Box<dyn Read>, TargetEol::PassThrough)
            }
            EolConversionSettings::Input | EolConversionSettings::InputOutput => {
                let mut peek = vec![];
                Read::by_ref(&mut contents)
                    .take(Self::PROBE_LIMIT)
                    .read_to_end(&mut peek)
                    .map_err(|source| EolError {
                        message: "failed to read the contents".to_string(),
                        source: Some(Box::new(source)),
                    })?;
                let target_eol = if is_binary(&peek) {
                    TargetEol::PassThrough
                } else {
                    TargetEol::Lf
                };
                let peek = Cursor::new(peek);
                let contents = Read::chain(peek, contents);
                (Box::new(contents) as Box<dyn Read>, target_eol)
            }
        };
        convert_eol(contents, target_eol, writer).map_err(|source| EolError {
            message: format!("failed to call convert_eol with {target_eol:?}"),
            source: Some(source.into()),
        })
    }

    pub(crate) async fn convert_eol_for_update(
        &self,
        mut contents: impl AsyncRead + Send + Unpin,
        writer: impl Write + Send + Unpin,
    ) -> Result<usize, impl Error + Send + Sync + 'static> {
        let (contents, target_eol) = match self.eol_conversion_settings {
            EolConversionSettings::None | EolConversionSettings::Input => (
                Box::pin(contents) as Pin<Box<dyn AsyncRead + Send + Unpin>>,
                TargetEol::PassThrough,
            ),
            EolConversionSettings::InputOutput => {
                let mut peek = vec![];
                (&mut contents)
                    .take(Self::PROBE_LIMIT)
                    .read_to_end(&mut peek)
                    .await
                    .map_err(|source| EolError {
                        message: "failed to read the file from the store".to_string(),
                        source: Some(Box::new(source)),
                    })?;
                let target_eol = if is_binary(&peek) {
                    TargetEol::PassThrough
                } else {
                    TargetEol::Crlf
                };
                let contents = AsyncReadExt::chain(Cursor::new(peek), contents);
                (Box::pin(contents) as _, target_eol)
            }
        };
        convert_eol_async(contents, target_eol, writer)
            .await
            .map_err(|source| EolError {
                message: format!("failed to call convert_eol_async with {target_eol:?}"),
                source: Some(source.into()),
            })
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[non_exhaustive]
/// Configuring auto-converting CRLF line endings into LF when you add a file to
/// the backend, and vice versa when it checks out code onto your filesystem.
pub enum EolConversionSettings {
    /// Do not perform EOL conversion.
    None,
    /// Only perform the CRLF to LF EOL conversion when writing to the backend
    /// store from the file system.
    Input,
    /// Perform CRLF to LF EOL conversion when writing to the backend store from
    /// the file system and LF to CRLF EOL conversion when writing to the file
    /// system from the backend store.
    InputOutput,
}

impl EolConversionSettings {
    fn try_from_config_value(value: ConfigValue) -> Result<Self, impl Error + Send + Sync> {
        let value = value.as_str().ok_or_else(|| EolError {
            message: "the working-copy.eol-conversion setting can't be casted to a string"
                .to_string(),
            source: None,
        })?;
        match value {
            "none" => Ok(Self::None),
            "input" => Ok(Self::Input),
            "input-output" => Ok(Self::InputOutput),
            other => Err(EolError {
                message: format!("unrecognized working-copy.eol-conversion value: {other}"),
                source: None,
            }),
        }
    }

    /// Try to create the [`EolConversionSettings`] based on the
    /// `working-copy.eol-conversion` setting in the [`UserSettings`].
    pub fn try_get_from_settings(
        user_settings: &UserSettings,
    ) -> Result<Self, impl Error + Send + Sync> {
        match user_settings
            .get_value_with("working-copy.eol-conversion", Self::try_from_config_value)
        {
            Ok(value) => Ok(value),
            Err(ConfigGetError::NotFound { .. }) => Ok(Self::None),
            Err(source) => Err(EolError {
                message: "failed to retrieve the working-copy.eol-conversion setting".to_string(),
                source: Some(Box::new(source)),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetEol {
    Lf,
    Crlf,
    PassThrough,
}

fn convert_eol_inner(input: &[u8], eol: &[u8]) -> Vec<u8> {
    let mut lines = input.lines().peekable();
    let mut res = Vec::<u8>::with_capacity(input.len());
    while let Some(line) = lines.next() {
        res.extend_from_slice(line);
        if lines.peek().is_some() || input.last() == Some(&b'\n') {
            // If we are not the last line, we should append the EOL, because this line must
            // ends with an EOL. If we are the last line, we only append the EOL when the
            // last line ends with EOL.
            res.extend_from_slice(eol);
        }
    }
    res
}

fn convert_eol(
    mut input: impl Read,
    target_eol: TargetEol,
    mut writer: impl Write,
) -> Result<(), impl Error + Send + Sync + 'static> {
    let eol = match target_eol {
        TargetEol::PassThrough => {
            std::io::copy(&mut input, &mut writer).map_err(|source| EolError {
                message: "failed to write input to writer without EOL conversion".to_string(),
                source: Some(source.into()),
            })?;
            return Ok(());
        }
        TargetEol::Lf => b"\n".as_slice(),
        TargetEol::Crlf => b"\r\n".as_slice(),
    };
    let mut contents = vec![];
    input
        .read_to_end(&mut contents)
        .map_err(|source| EolError {
            message: "failed to read from the input before EOL conversion".to_string(),
            source: Some(source.into()),
        })?;
    let res = convert_eol_inner(&contents, eol);
    writer.write_all(&res).map_err(|source| EolError {
        message: "failed to write the contents to the writer after EOL conversion".to_string(),
        source: Some(source.into()),
    })
}

async fn convert_eol_async(
    mut input: impl AsyncRead + Send + Unpin,
    target_eol: TargetEol,
    mut writer: impl Write + Send + Unpin,
) -> Result<usize, impl Error + Send + Sync + 'static> {
    let eol = match target_eol {
        TargetEol::PassThrough => {
            return copy_async_to_sync(input, &mut writer)
                .await
                .map_err(|source| EolError {
                    message: "failed to write input to writer without EOL conversion".to_string(),
                    source: Some(source.into()),
                });
        }
        TargetEol::Lf => b"\n".as_slice(),
        TargetEol::Crlf => b"\r\n".as_slice(),
    };
    let mut contents = vec![];
    input
        .read_to_end(&mut contents)
        .await
        .map_err(|source| EolError {
            message: "failed to read from the input before EOL conversion".to_string(),
            source: Some(source.into()),
        })?;
    let res = convert_eol_inner(&contents, eol);
    writer.write_all(&res).map_err(|source| EolError {
        message: "failed to write the contents to the writer after EOL conversion".to_string(),
        source: Some(source.into()),
    })?;
    Ok(res.len())
}

#[cfg(test)]
mod tests {
    use std::task::Poll;

    use pollster::FutureExt as _;
    use test_case::test_case;

    use super::*;
    use crate::config::ConfigLayer;
    use crate::config::ConfigSource;
    use crate::config::StackedConfig;

    #[test_case(b"a\n", TargetEol::PassThrough, b"a\n"; "LF text with no EOL conversion")]
    #[test_case(b"a\r\n", TargetEol::PassThrough, b"a\r\n"; "CRLF text with no EOL conversion")]
    #[test_case(b"a", TargetEol::PassThrough, b"a"; "no EOL text with no EOL conversion")]
    #[test_case(b"a\n", TargetEol::Crlf, b"a\r\n"; "LF text with CRLF EOL conversion")]
    #[test_case(b"a\r\n", TargetEol::Crlf, b"a\r\n"; "CRLF text with CRLF EOL conversion")]
    #[test_case(b"a", TargetEol::Crlf, b"a"; "no EOL text with CRLF conversion")]
    #[test_case(b"", TargetEol::Crlf, b""; "empty text with CRLF EOL conversion")]
    #[test_case(b"a\nb", TargetEol::Crlf, b"a\r\nb"; "text ends without EOL with CRLF EOL conversion")]
    #[test_case(b"a\n", TargetEol::Lf, b"a\n"; "LF text with LF EOL conversion")]
    #[test_case(b"a\r\n", TargetEol::Lf, b"a\n"; "CRLF text with LF EOL conversion")]
    #[test_case(b"a", TargetEol::Lf, b"a"; "no EOL text with LF conversion")]
    #[test_case(b"", TargetEol::Lf, b""; "empty text with LF EOL conversion")]
    #[test_case(b"a\r\nb", TargetEol::Lf, b"a\nb"; "text ends without EOL with LF EOL conversion")]
    fn test_eol_conversion(input: &[u8], target_eol: TargetEol, expected_output: &[u8]) {
        {
            let mut input = input;
            let mut output = vec![];
            convert_eol(&mut input, target_eol, &mut output)
                .expect("failed to read the output to end");
            assert_eq!(output, expected_output);
        }

        async {
            let mut input = input;
            let mut output = vec![];
            convert_eol_async(&mut input, target_eol, &mut output)
                .await
                .expect("failed to read the output to end");
            assert_eq!(output, expected_output);
        }
        .block_on();
    }

    struct ErrorReader(Option<std::io::Error>);

    impl ErrorReader {
        fn new(error: std::io::Error) -> Self {
            Self(Some(error))
        }
    }

    impl Read for ErrorReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            if let Some(e) = self.0.take() {
                return Err(e);
            }
            Ok(0)
        }
    }

    impl AsyncRead for ErrorReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if let Some(e) = self.0.take() {
                return Poll::Ready(Err(e));
            }
            Poll::Ready(Ok(()))
        }
    }

    #[test_case(TargetEol::PassThrough; "no EOL conversion")]
    #[test_case(TargetEol::Lf; "LF EOL conversion")]
    #[test_case(TargetEol::Crlf; "CRLF EOL conversion")]
    fn test_eol_convert_eol_read_error(target_eol: TargetEol) {
        let message = "test error";
        let error_reader = ErrorReader::new(std::io::Error::other(message));
        let mut output = vec![];
        let err = convert_eol(error_reader, target_eol, &mut output).expect_err("should fail");
        let has_expected_error_message = (0..)
            .scan(Some(&err as &(dyn Error + 'static)), |err, _| {
                let current_err = err.take()?;
                *err = current_err.source();
                Some(current_err)
            })
            .any(|e| e.to_string() == message);
        assert!(
            has_expected_error_message,
            "should have expected error message: {message}"
        );
    }

    #[test_case(TargetEol::PassThrough; "no EOL conversion")]
    #[test_case(TargetEol::Lf; "LF EOL conversion")]
    #[test_case(TargetEol::Crlf; "CRLF EOL conversion")]
    fn test_eol_convert_eol_async_read_error(target_eol: TargetEol) {
        async {
            let message = "test error";
            let error_reader = ErrorReader::new(std::io::Error::other(message));
            let mut output = vec![];
            let err = convert_eol_async(error_reader, target_eol, &mut output)
                .await
                .expect_err("should fail");
            let has_expected_error_message = (0..)
                .scan(Some(&err as &(dyn Error + 'static)), |err, _| {
                    let current_err = err.take()?;
                    *err = current_err.source();
                    Some(current_err)
                })
                .any(|e| e.to_string() == message);
            assert!(
                has_expected_error_message,
                "should have expected error message: {message}"
            );
        }
        .block_on();
    }

    fn user_settings_from_toml_text(config_text: &str) -> UserSettings {
        let mut config = StackedConfig::with_defaults();
        let default_config_text = r#"
            user.name = "Test User"
            user.email = "test.user@example.com"
            operation.username = "test-username"
            operation.hostname = "host.example.com"
            debug.randomness-seed = 42
        "#;
        config.add_layer(ConfigLayer::parse(ConfigSource::User, default_config_text).unwrap());
        config.add_layer(
            ConfigLayer::parse(ConfigSource::User, config_text)
                .expect("failed to parse the config text"),
        );
        UserSettings::from_config(config).expect("failed to create user settings from the config")
    }

    #[test]
    fn test_eol_conversion_setting_parse_should_default_to_none() {
        let user_settings = user_settings_from_toml_text("");
        let setting = EolConversionSettings::try_get_from_settings(&user_settings)
            .expect("should parse successfully");
        assert_eq!(setting, EolConversionSettings::None);
    }

    #[test_case(r#"working-copy.eol-conversion = "none""# => EolConversionSettings::None)]
    #[test_case(r#"working-copy.eol-conversion = "input""# => EolConversionSettings::Input)]
    #[test_case(r#"working-copy.eol-conversion = "input-output""# => EolConversionSettings::InputOutput)]
    fn test_eol_conversion_setting_parse_should_parse_correct_values_successfully(
        config_text: &str,
    ) -> EolConversionSettings {
        let user_settings = user_settings_from_toml_text(config_text);
        EolConversionSettings::try_get_from_settings(&user_settings)
            .expect("should parse successfully")
    }

    #[test_case("working-copy.eol-conversion = true"; "not string")]
    #[test_case(r#"working-copy.eol-conversion = "invalid-value-42""#; "invalid string")]
    fn test_eol_conversion_setting_parse_should_fail_on_invalid_values(config_text: &str) {
        let user_settings = user_settings_from_toml_text(config_text);
        EolConversionSettings::try_get_from_settings(&user_settings)
            .expect_err("should fail the parsing");
    }
}
