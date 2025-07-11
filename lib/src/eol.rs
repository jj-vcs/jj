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
use std::pin::Pin;

use bstr::ByteSlice as _;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt as _;

use crate::config::ConfigGetError;
use crate::local_working_copy::TreeStateSettings;
use crate::settings::UserSettings;

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

    pub(crate) async fn convert_eol_for_snapshot<'a>(
        &self,
        mut contents: impl AsyncRead + Send + Unpin + 'a,
    ) -> Result<impl AsyncRead + Send + Unpin + 'a, std::io::Error> {
        match self.eol_conversion_settings {
            EolConversionSettings::None => {
                Ok(Box::pin(contents) as Pin<Box<dyn AsyncRead + Send + Unpin>>)
            }
            EolConversionSettings::Input | EolConversionSettings::InputOutput => {
                let mut peek = vec![];
                (&mut contents)
                    .take(Self::PROBE_LIMIT)
                    .read_to_end(&mut peek)
                    .await?;
                let target_eol = if is_binary(&peek) {
                    TargetEol::PassThrough
                } else {
                    TargetEol::Lf
                };
                let peek = Cursor::new(peek);
                let contents = peek.chain(contents);
                convert_eol(contents, target_eol).await
            }
        }
    }

    pub(crate) async fn convert_eol_for_update<'a>(
        &self,
        mut contents: impl AsyncRead + Send + Unpin + 'a,
    ) -> Result<impl AsyncRead + Send + Unpin + 'a, std::io::Error> {
        match self.eol_conversion_settings {
            EolConversionSettings::None | EolConversionSettings::Input => {
                Ok(Box::pin(contents) as Pin<Box<dyn AsyncRead + Send + Unpin>>)
            }
            EolConversionSettings::InputOutput => {
                let mut peek = vec![];
                (&mut contents)
                    .take(Self::PROBE_LIMIT)
                    .read_to_end(&mut peek)
                    .await?;
                let target_eol = if is_binary(&peek) {
                    TargetEol::PassThrough
                } else {
                    TargetEol::Crlf
                };
                let peek = Cursor::new(peek);
                let contents = peek.chain(contents);
                convert_eol(contents, target_eol).await
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone, serde::Deserialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
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
    /// Try to create the [`EolConversionSettings`] based on the
    /// `working-copy.eol-conversion` setting in the [`UserSettings`].
    pub fn try_get_from_settings(
        user_settings: &UserSettings,
    ) -> Result<Self, impl Error + Send + Sync> {
        match user_settings.get("working-copy.eol-conversion") {
            Ok(value) => Ok(value),
            Err(ConfigGetError::NotFound { .. }) => Ok(Self::None),
            Err(err) => Err(err),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetEol {
    Lf,
    Crlf,
    PassThrough,
}

async fn convert_eol<'a>(
    mut input: impl AsyncRead + Send + Unpin + 'a,
    target_eol: TargetEol,
) -> Result<Pin<Box<dyn AsyncRead + Send + Unpin + 'a>>, std::io::Error> {
    let eol = match target_eol {
        TargetEol::PassThrough => {
            return Ok(Box::pin(input) as Pin<Box<dyn AsyncRead + Send + Unpin>>);
        }
        TargetEol::Lf => b"\n".as_slice(),
        TargetEol::Crlf => b"\r\n".as_slice(),
    };

    let mut contents = vec![];
    input.read_to_end(&mut contents).await?;
    let mut lines = contents.lines().peekable();
    let mut res = Vec::<u8>::with_capacity(contents.len());
    while let Some(line) = lines.next() {
        res.extend_from_slice(line);
        if lines.peek().is_some() || contents.last() == Some(&b'\n') {
            // If we are not the last line, we should append the EOL, because this line must
            // ends with an EOL. If we are the last line, we only append the EOL when the
            // last line ends with EOL.
            res.extend_from_slice(eol);
        }
    }
    Ok(Box::pin(Cursor::new(res)) as Pin<Box<dyn AsyncRead + Send + Unpin>>)
}

#[cfg(test)]
mod tests {
    use std::task::Poll;

    use futures::TryFutureExt as _;
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
        async {
            let mut input = input;
            let mut output = vec![];
            convert_eol(&mut input, target_eol)
                .await
                .expect("failed to call convert_eol")
                .read_to_end(&mut output)
                .await
                .expect("failed to read frm the result");
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
        let err = async {
            convert_eol(error_reader, target_eol)
                .and_then(async |mut reader| reader.read_to_end(&mut output).await)
                .await
                .expect_err("should fail")
        }
        .block_on();
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

    #[test]
    fn test_eol_conversion_setting_parse_should_default_to_none() {
        let mut config = StackedConfig::with_defaults();
        let default_config_text = r#"
            user.name = "Test User"
            user.email = "test.user@example.com"
            operation.username = "test-username"
            operation.hostname = "host.example.com"
            debug.randomness-seed = 42
        "#;
        config.add_layer(ConfigLayer::parse(ConfigSource::User, default_config_text).unwrap());
        let user_settings = UserSettings::from_config(config).expect("failed to create user settings from the config");
        let setting = EolConversionSettings::try_get_from_settings(&user_settings)
            .expect("should parse successfully");
        assert_eq!(setting, EolConversionSettings::None);
    }
}
