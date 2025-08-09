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

use std::io::Cursor;

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
        eol_conversion_mode: tree_state_settings.eol_conversion_mode,
    }
}

fn is_binary(bytes: &[u8]) -> bool {
    // TODO(06393993): align the algorithm with git so that the git config autocrlf
    // users won't see different decisions on whether a file is binary and needs to
    // perform EOL conversion.
    let mut bytes = bytes.iter().peekable();
    while let Some(byte) = bytes.next() {
        match *byte {
            b'\0' => return true,
            b'\r' => {
                if bytes.peek() != Some(&&b'\n') {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

#[derive(Clone)]
pub(crate) struct TargetEolStrategy {
    eol_conversion_mode: EolConversionMode,
}

impl TargetEolStrategy {
    /// The limit is to probe whether the file is binary is 8KB.
    const PROBE_LIMIT: u64 = 8 << 10;

    pub(crate) async fn convert_eol_for_snapshot<'a>(
        &self,
        mut contents: impl AsyncRead + Send + Unpin + 'a,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin + 'a>, std::io::Error> {
        match self.eol_conversion_mode {
            EolConversionMode::None => Ok(Box::new(contents)),
            EolConversionMode::Input | EolConversionMode::InputOutput => {
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
    ) -> Result<Box<dyn AsyncRead + Send + Unpin + 'a>, std::io::Error> {
        match self.eol_conversion_mode {
            EolConversionMode::None | EolConversionMode::Input => Ok(Box::new(contents)),
            EolConversionMode::InputOutput => {
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

/// Configuring auto-converting CRLF line endings into LF when you add a file to
/// the backend, and vice versa when it checks out code onto your filesystem.
#[derive(Debug, PartialEq, Eq, Copy, Clone, serde::Deserialize, Default)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub enum EolConversionMode {
    /// Do not perform EOL conversion.
    #[default]
    None,
    /// Only perform the CRLF to LF EOL conversion when writing to the backend
    /// store from the file system.
    Input,
    /// Perform CRLF to LF EOL conversion when writing to the backend store from
    /// the file system and LF to CRLF EOL conversion when writing to the file
    /// system from the backend store.
    InputOutput,
}

impl EolConversionMode {
    /// Try to create the [`EolConversionMode`] based on the
    /// `working-copy.eol-conversion` setting in the [`UserSettings`].
    pub fn try_from_settings(user_settings: &UserSettings) -> Result<Self, ConfigGetError> {
        user_settings.get("working-copy.eol-conversion")
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
) -> Result<Box<dyn AsyncRead + Send + Unpin + 'a>, std::io::Error> {
    let eol = match target_eol {
        TargetEol::PassThrough => {
            return Ok(Box::new(input));
        }
        TargetEol::Lf => b"\n".as_slice(),
        TargetEol::Crlf => b"\r\n".as_slice(),
    };

    let mut contents = vec![];
    input.read_to_end(&mut contents).await?;
    let lines = contents.lines_with_terminator();
    let mut res = Vec::<u8>::with_capacity(contents.len());
    fn trim_last_eol(input: &[u8]) -> Option<&[u8]> {
        input
            .strip_suffix(b"\r\n")
            .or_else(|| input.strip_suffix(b"\n"))
    }
    for line in lines {
        if let Some(line) = trim_last_eol(line) {
            res.extend_from_slice(line);
            // If the line ends with an EOL, we should append the target EOL.
            res.extend_from_slice(eol);
        } else {
            // If the line doesn't end with an EOL, we don't append the EOL. This can happen
            // on the last line.
            res.extend_from_slice(line);
        }
    }
    Ok(Box::new(Cursor::new(res)))
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::pin::Pin;
    use std::task::Poll;

    use test_case::test_case;

    use super::*;

    #[tokio::main(flavor = "current_thread")]
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
    async fn test_eol_conversion(input: &[u8], target_eol: TargetEol, expected_output: &[u8]) {
        let mut input = input;
        let mut output = vec![];
        convert_eol(&mut input, target_eol)
            .await
            .expect("Failed to call convert_eol")
            .read_to_end(&mut output)
            .await
            .expect("Failed to read from the result");
        assert_eq!(output, expected_output);
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

    #[tokio::main(flavor = "current_thread")]
    #[test_case(TargetEol::PassThrough; "no EOL conversion")]
    #[test_case(TargetEol::Lf; "LF EOL conversion")]
    #[test_case(TargetEol::Crlf; "CRLF EOL conversion")]
    async fn test_eol_convert_eol_read_error(target_eol: TargetEol) {
        let message = "test error";
        let error_reader = ErrorReader::new(std::io::Error::other(message));
        let mut output = vec![];
        // TODO: use TryFutureExt::and_then and async closure after we upgrade to 1.85.0
        // or later.
        let err = match convert_eol(error_reader, target_eol).await {
            Ok(mut reader) => reader.read_to_end(&mut output).await,
            Err(e) => Err(e),
        }
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

    #[tokio::main(flavor = "current_thread")]
    #[test_case(TargetEolStrategy {
          eol_conversion_mode: EolConversionMode::None,
      }, b"\r\n", b"\r\n"; "none settings")]
    #[test_case(TargetEolStrategy {
          eol_conversion_mode: EolConversionMode::Input,
      }, b"\r\n", b"\n"; "input settings text input")]
    #[test_case(TargetEolStrategy {
          eol_conversion_mode: EolConversionMode::InputOutput,
      }, b"\r\n", b"\n"; "input output settings text input")]
    #[test_case(TargetEolStrategy {
          eol_conversion_mode: EolConversionMode::Input,
      }, b"\0\r\n", b"\0\r\n"; "input settings binary input")]
    #[test_case(TargetEolStrategy {
          eol_conversion_mode: EolConversionMode::InputOutput,
      }, b"\0\r\n", b"\0\r\n"; "input output settings binary input with NUL")]
    #[test_case(TargetEolStrategy {
          eol_conversion_mode: EolConversionMode::InputOutput,
      }, b"\r\r\n", b"\r\r\n"; "input output settings binary input with lone CR")]
    #[test_case(TargetEolStrategy {
          eol_conversion_mode: EolConversionMode::Input,
      }, &[0; 20 << 10], &[0; 20 << 10]; "input settings long binary input")]
    async fn test_eol_strategy_convert_eol_for_snapshot(
        strategy: TargetEolStrategy,
        contents: &[u8],
        expected_output: &[u8],
    ) {
        let mut actual_output = vec![];
        strategy
            .convert_eol_for_snapshot(contents)
            .await
            .unwrap()
            .read_to_end(&mut actual_output)
            .await
            .unwrap();
        assert_eq!(actual_output, expected_output);
    }

    #[tokio::main(flavor = "current_thread")]
    #[test_case(TargetEolStrategy {
          eol_conversion_mode: EolConversionMode::None,
      }, b"\n", b"\n"; "none settings")]
    #[test_case(TargetEolStrategy {
          eol_conversion_mode: EolConversionMode::Input,
      }, b"\n", b"\n"; "input settings")]
    #[test_case(TargetEolStrategy {
          eol_conversion_mode: EolConversionMode::InputOutput,
      }, b"\n", b"\r\n"; "input output settings text input")]
    #[test_case(TargetEolStrategy {
          eol_conversion_mode: EolConversionMode::InputOutput,
      }, b"\0\n", b"\0\n"; "input output settings binary input")]
    #[test_case(TargetEolStrategy {
          eol_conversion_mode: EolConversionMode::Input,
      }, &[0; 20 << 10], &[0; 20 << 10]; "input output settings long binary input")]
    async fn test_eol_strategy_convert_eol_for_update(
        strategy: TargetEolStrategy,
        contents: &[u8],
        expected_output: &[u8],
    ) {
        let mut actual_output = vec![];
        strategy
            .convert_eol_for_update(contents)
            .await
            .unwrap()
            .read_to_end(&mut actual_output)
            .await
            .unwrap();
        assert_eq!(actual_output, expected_output);
    }
}
