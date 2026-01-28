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
use crate::gitattributes::GitAttributes;
use crate::gitattributes::State;
use crate::repo_path::RepoPath;
use crate::settings::UserSettings;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The gitattributes related to eol conversion.
#[derive(PartialEq, Eq, Debug, Clone)]
pub(crate) struct EolGitAttributes {
    /// The state of the `eol` gitattribute.
    ///
    /// See https://git-scm.com/docs/gitattributes#_eol.
    pub eol: State,
    /// The state of the `text` gitattribute.
    ///
    /// See https://git-scm.com/docs/gitattributes#_text.
    pub text: State,
    /// The state of the `crlf` gitattribute.
    ///
    /// See https://git-scm.com/docs/gitattributes#_backwards_compatibility_with_crlf_attribute.
    pub crlf: State,
}

impl EolGitAttributes {
    /// Apply the automatic state conversion based on the states of other
    /// attributes.
    ///
    /// Currently, we handle 2 types of conversion:
    ///
    /// * Specifying `eol` automatically sets `text` if `text` was left
    ///   unspecified.
    /// * The `crlf` backwards compatibility is converted to proper `text` and
    ///   `eol` states.
    fn normalize(mut self) -> Self {
        // If the text and eol attributes are not one of the defined states, they behave
        // as if they are unspecified.
        let text_unspecified = match &self.text {
            State::Unspecified => true,
            State::Set | State::Unset => false,
            State::Value(value) => value != b"auto",
        };

        let eol_unspecified = match &self.eol {
            State::Unspecified | State::Set | State::Unset => true,
            State::Value(value) => value != b"lf" && value != b"crlf",
        };

        if text_unspecified && !eol_unspecified {
            // Specifying eol automatically sets text if text was left unspecified.
            self.text = State::Set;
        }

        if text_unspecified && eol_unspecified {
            // crlf exists for backwards compatibility, so it only has effect when both text
            // and eol are unspecified.
            match &self.crlf {
                State::Set => self.text = State::Set,
                State::Unset => self.text = State::Unset,
                State::Value(value) if value == b"input" => {
                    // While the gitattributes doc only mentions that crlf=input is equivalent to
                    // eol=lf, the doc also mentions that specifying eol automatically sets text if
                    // text was left unspecified.
                    self.text = State::Set;
                    self.eol = State::Value(b"lf".to_vec());
                }
                _ => {}
            }
        }

        self
    }
}

pub(crate) trait GitAttributesExt {
    async fn search_eol(&self, path: &RepoPath) -> Result<EolGitAttributes, BoxError>;
}

impl GitAttributesExt for GitAttributes {
    async fn search_eol(&self, path: &RepoPath) -> Result<EolGitAttributes, BoxError> {
        let mut attributes = self.search(path, &["text", "eol", "crlf"]).await?;
        Ok(EolGitAttributes {
            text: attributes.remove("text").unwrap_or(State::Unspecified),
            eol: attributes.remove("eol").unwrap_or(State::Unspecified),
            crlf: attributes.remove("crlf").unwrap_or(State::Unspecified),
        })
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

/// Indicate whether EOL conversion needs to be applied on update. Part of
/// [`EffectiveEolMode`].
enum ConvertMode {
    /// Apply EOL conversion on both snapshot and update. Equivalent to
    /// `eol=crlf`.
    InputOutput,
    /// Apply EOL conversion only on snapshot. Equivalent to `eol=lf`.
    Input,
}

/// A type that is resolved from [`EolConversionSettings`], and
/// [`EolGitAttributes`]` with fewer variants.
enum EffectiveEolMode {
    /// The file is a text file, and may apply EOL conversion. Equivalent to set
    /// text.
    Text(ConvertMode),
    /// Use heuristics to detect if EOL conversion should be applied. Equivalent
    /// to `text=auto`.
    Auto(ConvertMode),
    /// The file is a binary file, and we should not apply EOL conversion.
    /// Equivalent to `-text`.
    Binary,
}

impl EffectiveEolMode {
    /// Resolve the attributes and the settings to [`EffectiveEolMode`] which
    /// has much less variants.
    fn new(attr: EolGitAttributes, settings: &EolConversionSettings) -> Self {
        let attr = attr.normalize();

        fn resolve_convert_mode(
            attr: &EolGitAttributes,
            settings: &EolConversionSettings,
        ) -> ConvertMode {
            debug_assert_eq!(
                attr,
                &attr.clone().normalize(),
                "The passed in EolGitAttributes must be normalized, so that we don't have to \
                 consider the crlf attribute separately."
            );

            // If the eol attribute is set explicitly, ConvertMode is decide by the
            // attribute.
            if let State::Value(value) = &attr.eol {
                if value == b"crlf" {
                    return ConvertMode::InputOutput;
                }
                if value == b"lf" {
                    return ConvertMode::Input;
                }
            }

            // If the eol attribute is unspecified or not in a state defined in the
            // document, the ConvertMode is decided by the settings, following
            // https://git-scm.com/docs/gitattributes#Documentation/gitattributes.txt-Unspecified-1-1.
            //
            // > If the eol attribute is unspecified for a file, its line endings in the
            // > working directory are determined by the core.autocrlf or core.eol
            // > configuration variable.
            match settings.eol_conversion_mode {
                EolConversionMode::InputOutput => return ConvertMode::InputOutput,
                EolConversionMode::Input => return ConvertMode::Input,
                EolConversionMode::None => {}
            }
            // The working-copy.gitattributes-default-eol setting only takes effect when the
            // working-copy.eol-conversion setting is set to "none", following
            // https://git-scm.com/docs/git-config#Documentation/git-config.txt-coreeol.
            //
            // > Note that this value is ignored if core.autocrlf is set to true or input.
            match settings.default_eol_attributes {
                EolConversionMode::InputOutput => return ConvertMode::InputOutput,
                EolConversionMode::Input => return ConvertMode::Input,
                EolConversionMode::None => {}
            }
            // If the eol attribute is unspecified and the settings are none, ConvertMode is
            // decided based on the platform, following
            // https://git-scm.com/docs/gitattributes#Documentation/gitattributes.txt-Unspecified-1-1.
            //
            // > If text is set but neither of those variables is, the default is eol=crlf
            // > on Windows and eol=lf on all other platforms.
            if cfg!(windows) {
                ConvertMode::InputOutput
            } else {
                ConvertMode::Input
            }
        }

        match &attr.text {
            State::Set => Self::Text(resolve_convert_mode(&attr, settings)),
            State::Unset => Self::Binary,
            State::Value(value) if value == b"auto" => {
                Self::Auto(resolve_convert_mode(&attr, settings))
            }
            // If the text attributes is unspecified or not in a state defined in the doc, we use
            // the working-copy.eol-conversion setting, following
            // https://git-scm.com/docs/gitattributes#Documentation/gitattributes.txt-Unspecified-1.
            //
            // > If the text attribute is unspecified, Git uses the core.autocrlf configuration
            // > variable to determine if the file should be converted.
            _ => match settings.eol_conversion_mode {
                // If the setting is none, we don't apply EOL conversion.
                EolConversionMode::None => Self::Binary,
                // If the setting is not none, we probe the contents and decide whether to apply EOL
                // conversion, following
                // https://git-scm.com/docs/git-config#Documentation/git-config.txt-coreautocrlf.
                //
                // > Setting this variable to "true" is the same as setting the text attribute to
                // > "auto" on all files and core.eol to "crlf".
                //
                // > This variable can be set to input, in which case no output conversion is
                // > performed.
                _ => Self::Auto(resolve_convert_mode(&attr, settings)),
            },
        }
    }
}

#[derive(Clone)]
pub(crate) struct TargetEolStrategy {
    settings: EolConversionSettings,
}

impl TargetEolStrategy {
    pub(crate) fn new(settings: EolConversionSettings) -> Self {
        Self { settings }
    }

    /// The limit to probe for whether the file is binary is 8KB.
    /// All files strictly smaller than the limit are always
    /// evaluated correctly and in full.
    /// Files larger than the limit - or with ambiguous content at the limit -
    /// are potentially misclassified.
    const PROBE_LIMIT: u64 = 8 << 10;

    /// Peek into the first [`TargetEolStrategy::PROBE_LIMIT`] bytes of content
    /// to determine if it is binary data.
    ///
    /// Peeked data is stored in `peek`.
    async fn probe_for_binary(
        mut contents: impl AsyncRead + Unpin,
        peek: &mut Vec<u8>,
    ) -> Result<bool, std::io::Error> {
        (&mut contents)
            .take(Self::PROBE_LIMIT)
            .read_to_end(peek)
            .await?;

        // The probe limit may have sliced a CRLF sequence, which would cause
        // misclassification as binary.
        let slice_to_check = if peek.get(Self::PROBE_LIMIT as usize - 1) == Some(&b'\r') {
            &peek[0..Self::PROBE_LIMIT as usize - 1]
        } else {
            peek
        };

        Ok(is_binary(slice_to_check))
    }

    async fn get_effective_eol_mode<'a, F>(
        &self,
        get_git_attributes: F,
    ) -> Result<EffectiveEolMode, BoxError>
    where
        F: (AsyncFnOnce() -> Result<EolGitAttributes, BoxError>) + Send + Unpin + 'a,
    {
        let git_attributes = if self.settings.use_git_attributes {
            // We only read the gitattributes file if necessary to save the cost for users
            // who don't use gitattributes.
            get_git_attributes().await?
        } else {
            EolGitAttributes {
                eol: State::Unspecified,
                text: State::Unspecified,
                crlf: State::Unspecified,
            }
        };
        Ok(EffectiveEolMode::new(git_attributes, &self.settings))
    }

    pub(crate) async fn convert_eol_for_snapshot<'a, F>(
        &self,
        mut contents: impl AsyncRead + Send + Unpin + 'a,
        get_git_attributes: F,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin + 'a>, BoxError>
    where
        F: (AsyncFnOnce() -> Result<EolGitAttributes, BoxError>) + Send + Unpin + 'a,
    {
        // For snapshot, we don't care about ConvertMode, if EOL conversion is applied,
        // we always convert the EOL to LF.
        match self.get_effective_eol_mode(get_git_attributes).await? {
            EffectiveEolMode::Binary => Ok(Box::new(contents)),
            EffectiveEolMode::Text(_) => convert_eol(contents, TargetEol::Lf)
                .await
                .map_err(|e| Box::new(e) as _),
            EffectiveEolMode::Auto(_) => {
                let mut peek = vec![];
                let target_eol = if Self::probe_for_binary(&mut contents, &mut peek).await? {
                    TargetEol::PassThrough
                } else {
                    TargetEol::Lf
                };
                let peek = Cursor::new(peek);
                let contents = peek.chain(contents);
                convert_eol(contents, target_eol)
                    .await
                    .map_err(|e| Box::new(e) as _)
            }
        }
    }

    pub(crate) async fn convert_eol_for_update<'a, F>(
        &self,
        mut contents: impl AsyncRead + Send + Unpin + 'a,
        get_git_attributes: F,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin + 'a>, BoxError>
    where
        F: (AsyncFnOnce() -> Result<EolGitAttributes, BoxError>) + Send + Unpin + 'a,
    {
        match self.get_effective_eol_mode(get_git_attributes).await? {
            // For update, if EffectiveEolMode resolves to Binary or the ConvertMode is Input, we
            // don't need to apply any conversion.
            EffectiveEolMode::Binary
            | EffectiveEolMode::Text(ConvertMode::Input)
            | EffectiveEolMode::Auto(ConvertMode::Input) => Ok(Box::new(contents)),
            EffectiveEolMode::Text(ConvertMode::InputOutput) => {
                convert_eol(contents, TargetEol::Crlf)
                    .await
                    .map_err(|e| Box::new(e) as _)
            }
            EffectiveEolMode::Auto(ConvertMode::InputOutput) => {
                let mut peek = vec![];
                let target_eol = if Self::probe_for_binary(&mut contents, &mut peek).await? {
                    TargetEol::PassThrough
                } else {
                    TargetEol::Crlf
                };
                let peek = Cursor::new(peek);
                let contents = peek.chain(contents);
                convert_eol(contents, target_eol)
                    .await
                    .map_err(|e| Box::new(e) as _)
            }
        }
    }
}

/// Configuring auto-converting CRLF line endings into LF when you add a file to
/// the backend, and vice versa when it checks out code onto your filesystem.
#[derive(Debug, PartialEq, Eq, Copy, Clone, serde::Deserialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub enum EolConversionMode {
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

/// EOL conversion user settings.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct EolConversionSettings {
    /// A killer switch on whether EOL conversion should read gitattributes.
    /// When the value is `false`, the implementation shouldn't read any
    /// `gitattributes` files so that the user that doesn't need this feature
    /// pay little cost if not zero.
    pub use_git_attributes: bool,
    /// The equivalent of the git [`core.eol`] config. It has the same 3 valid
    /// values as [`Self::eol_conversion_mode`]:
    /// [`EolConversionMode::InputOutput`], [`EolConversionMode::Input`],
    /// [`EolConversionMode::None`]. When the [`Self::eol_conversion_mode`]
    /// setting is not [`EolConversionMode::None`], this setting is ignored
    /// following [the gitattributes doc]. Note that:
    /// - The names are different from the actual `eol` `gitattributes`, so it
    ///   can be confusing, but we do our best to document this divergence.
    /// - We don't have an equivalent for `native` in `core.eol`, because we
    ///   don't think it's necessary: the user should always specify `input` or
    ///   `input-output` explicitly, and the `jj` EOL settings are not supposed
    ///   to be shared across multiple machines.
    ///
    /// [`core.eol`]: https://git-scm.com/docs/git-config#Documentation/git-config.txt-coreeol
    /// [the gitattributes doc]:
    /// https://git-scm.com/docs/git-config#Documentation/git-config.txt-coreeol
    pub default_eol_attributes: EolConversionMode,
    /// The equivalent of the git [`core.autocrlf`] config.
    ///
    /// [`core.autocrlf`]:
    /// https://git-scm.com/docs/git-config#Documentation/git-config.txt-coreautocrlf
    pub eol_conversion_mode: EolConversionMode,
}

impl EolConversionSettings {
    /// Try to create the [`EolConversionSettings`] based on the
    /// [`UserSettings`].
    pub fn try_from_settings(user_settings: &UserSettings) -> Result<Self, ConfigGetError> {
        Ok(Self {
            use_git_attributes: user_settings
                .get_bool("working-copy.eol-conversion-use-gitattributes")?,
            default_eol_attributes: user_settings.get("working-copy.gitattributes-default-eol")?,
            eol_conversion_mode: user_settings.get("working-copy.eol-conversion")?,
        })
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
    use test_case::test_matrix;

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

    fn test_probe_limit_input_crlf() -> [u8; TargetEolStrategy::PROBE_LIMIT as usize + 1] {
        let mut arr = [b'a'; TargetEolStrategy::PROBE_LIMIT as usize + 1];
        let crlf = b"\r\n";
        arr[100..102].copy_from_slice(crlf);
        arr[500..502].copy_from_slice(crlf);
        arr[1000..1002].copy_from_slice(crlf);
        arr[4090..4092].copy_from_slice(crlf);
        arr[TargetEolStrategy::PROBE_LIMIT as usize - 1
            ..TargetEolStrategy::PROBE_LIMIT as usize + 1]
            .copy_from_slice(crlf);
        arr
    }

    fn test_probe_limit_input_lf() -> Vec<u8> {
        test_probe_limit_input_crlf().replace(b"\r\n", b"\n")
    }

    #[tokio::main(flavor = "current_thread")]
    #[test_case(EolConversionMode::None, b"\r\n", b"\r\n"; "none settings")]
    #[test_case(EolConversionMode::Input, b"\r\n", b"\n"; "input settings text input")]
    #[test_case(EolConversionMode::InputOutput, b"\r\n", b"\n"; "input output settings text input")]
    #[test_case(EolConversionMode::Input, b"\0\r\n", b"\0\r\n"; "input settings binary input")]
    #[test_case(
        EolConversionMode::InputOutput,
        b"\0\r\n",
        b"\0\r\n";
        "input output settings binary input with NUL"
    )]
    #[test_case(
        EolConversionMode::InputOutput,
        b"\r\r\n",
        b"\r\r\n";
        "input output settings binary input with lone CR"
    )]
    #[test_case(
        EolConversionMode::Input,
        &[0; 20 << 10],
        &[0; 20 << 10];
        "input settings long binary input"
    )]
    #[test_case(
        EolConversionMode::Input,
        &test_probe_limit_input_crlf(),
        &test_probe_limit_input_lf();
        "input settings with CRLF on probe boundary"
    )]
    async fn test_eol_strategy_convert_eol_for_snapshot(
        eol_conversion_mode: EolConversionMode,
        contents: &[u8],
        expected_output: &[u8],
    ) {
        let settings = EolConversionSettings {
            use_git_attributes: false,
            default_eol_attributes: EolConversionMode::None,
            eol_conversion_mode,
        };
        let strategy = TargetEolStrategy::new(settings);
        let mut actual_output = vec![];
        strategy
            .convert_eol_for_snapshot(contents, async || panic!("should not read gitattributes"))
            .await
            .unwrap()
            .read_to_end(&mut actual_output)
            .await
            .unwrap();
        assert_eq!(actual_output, expected_output);
    }

    #[tokio::main(flavor = "current_thread")]
    #[test_case(EolConversionMode::None, b"\n", b"\n"; "none settings")]
    #[test_case(EolConversionMode::Input, b"\n", b"\n"; "input settings")]
    #[test_case(EolConversionMode::InputOutput, b"\n", b"\r\n"; "input output settings text input")]
    #[test_case(
        EolConversionMode::InputOutput,
        b"\0\n",
        b"\0\n";
        "input output settings binary input"
    )]
    #[test_case(
        EolConversionMode::Input,
        &[0; 20 << 10],
        &[0; 20 << 10];
        "input output settings long binary input"
    )]
    async fn test_eol_strategy_convert_eol_for_update(
        eol_conversion_mode: EolConversionMode,
        contents: &[u8],
        expected_output: &[u8],
    ) {
        let settings = EolConversionSettings {
            use_git_attributes: false,
            default_eol_attributes: EolConversionMode::None,
            eol_conversion_mode,
        };
        let strategy = TargetEolStrategy::new(settings);
        let mut actual_output = vec![];
        strategy
            .convert_eol_for_update(contents, async || panic!("should not read gitattributes"))
            .await
            .unwrap()
            .read_to_end(&mut actual_output)
            .await
            .unwrap();
        assert_eq!(actual_output, expected_output);
    }

    #[tokio::main(flavor = "current_thread")]
    #[test_matrix(
        [EolConversionMode::None, EolConversionMode::Input, EolConversionMode::InputOutput],
        [EolConversionMode::None, EolConversionMode::Input, EolConversionMode::InputOutput],
        [b"\0", b"a\r\n", b"a\n"]
    )]
    async fn test_eol_gitattr_not_used(
        default_eol_attributes: EolConversionMode,
        eol_conversion_mode: EolConversionMode,
        contents: &[u8],
    ) {
        let settings = EolConversionSettings {
            use_git_attributes: false,
            default_eol_attributes,
            eol_conversion_mode,
        };
        let strategy = TargetEolStrategy::new(settings);
        let mut output = vec![];
        strategy
            .convert_eol_for_update(contents, async || panic!("should not read gitattributes"))
            .await
            .unwrap()
            .read_to_end(&mut output)
            .await
            .unwrap();
        let mut output = vec![];
        strategy
            .convert_eol_for_snapshot(contents, async || panic!("should not read gitattributes"))
            .await
            .unwrap()
            .read_to_end(&mut output)
            .await
            .unwrap();
    }

    #[tokio::main(flavor = "current_thread")]
    #[test_matrix(
        State::Set,
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"lf".to_vec()),
            State::Value(b"crlf".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [(b"\0a\r\n", b"\0a\n"), (b"a\r\n", b"a\n")];
        "text set"
    )]
    #[test_matrix(
        State::Value(b"auto".to_vec()),
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"lf".to_vec()),
            State::Value(b"crlf".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [(b"a\r\n", b"a\n")];
        "text=auto text contents"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Value(b"lf".to_vec()),
            State::Value(b"crlf".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [(b"\0a\r\n", b"\0a\n"), (b"a\r\n", b"a\n")];
        "!text eol=lf or eol=crlf"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [(b"\0a\r\n", b"\0a\n"), (b"a\r\n", b"a\n")];
        "eol=crlf or eol=input"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [(b"a\r\n", b"a\n")];
        "eol-conversion is input or input-output"
    )]
    async fn test_eol_snapshot_should_convert_eol(
        text: State,
        eol: State,
        crlf: State,
        default_eol_attributes: EolConversionMode,
        eol_conversion_mode: EolConversionMode,
        (contents, expect): (&[u8], &[u8]),
    ) {
        let settings = EolConversionSettings {
            use_git_attributes: true,
            default_eol_attributes,
            eol_conversion_mode,
        };
        let strategy = TargetEolStrategy::new(settings);
        let mut output = vec![];
        strategy
            .convert_eol_for_snapshot(contents, async || Ok(EolGitAttributes { eol, text, crlf }))
            .await
            .unwrap()
            .read_to_end(&mut output)
            .await
            .unwrap();
        assert_eq!(output, expect);
    }

    #[tokio::main(flavor = "current_thread")]
    #[test_matrix(
        State::Value(b"auto".to_vec()),
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"lf".to_vec()),
            State::Value(b"crlf".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        b"\0a\r\n";
        "text=auto binary contents"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        b"\0a\r\n";
        "eol-conversion is input or input-output binary contents"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Unset,
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [b"\0a\r\n", b"a\r\n"];
        "-crlf"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        EolConversionMode::None,
        [b"\0a\r\n", b"a\r\n"];
        "eol-conversion=none"
    )]
    async fn test_eol_snapshot_should_not_convert_eol(
        text: State,
        eol: State,
        crlf: State,
        default_eol_attributes: EolConversionMode,
        eol_conversion_mode: EolConversionMode,
        contents: &[u8],
    ) {
        let settings = EolConversionSettings {
            use_git_attributes: true,
            default_eol_attributes,
            eol_conversion_mode,
        };
        let strategy = TargetEolStrategy::new(settings);
        let mut output = vec![];
        strategy
            .convert_eol_for_snapshot(contents, async || Ok(EolGitAttributes { eol, text, crlf }))
            .await
            .unwrap()
            .read_to_end(&mut output)
            .await
            .unwrap();
        assert_eq!(output, contents);
    }

    #[tokio::main(flavor = "current_thread")]
    #[test_matrix(
        [
            State::Set,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Value(b"crlf".to_vec()),
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [(b"\0a\n", b"\0a\r\n"), (b"a\n", b"a\r\n")];
        "eol=crlf"
    )]
    #[test_matrix(
        State::Set,
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        EolConversionMode::InputOutput,
        EolConversionMode::None,
        [(b"\0a\n", b"\0a\r\n"), (b"a\n", b"a\r\n")];
        "text set gitattributes-default-eol=input-output"
    )]
    #[test_matrix(
        State::Set,
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        EolConversionMode::InputOutput,
        [(b"\0a\n", b"\0a\r\n"), (b"a\n", b"a\r\n")];
        "text set eol-conversion=input-output"
    )]
    #[test_matrix(
        State::Value(b"auto".to_vec()),
        State::Value(b"crlf".to_vec()),
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [(b"a\n", b"a\r\n")];
        "text=auto eol=crlf text contents"
    )]
    #[test_matrix(
        State::Value(b"auto".to_vec()),
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        EolConversionMode::InputOutput,
        EolConversionMode::None,
        [(b"a\n", b"a\r\n")];
        "text=auto gitattributes-default-eol=input-output text contents"
    )]
    #[test_matrix(
        State::Value(b"auto".to_vec()),
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        EolConversionMode::InputOutput,
        [(b"a\n", b"a\r\n")];
        "text=auto eol-conversion=input-output text contents"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Set,
        EolConversionMode::InputOutput,
        EolConversionMode::None,
        [(b"\0a\n", b"\0a\r\n"), (b"a\n", b"a\r\n")];
        "crlf set gitattributes-default-eol=input-output"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Set,
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        EolConversionMode::InputOutput,
        [(b"\0a\n", b"\0a\r\n"), (b"a\n", b"a\r\n")];
        "crlf set eol-conversion=input-output"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        EolConversionMode::InputOutput,
        [(b"a\n", b"a\r\n")];
        "eol-conversion=input-output text contents"
    )]
    async fn test_eol_update_should_convert_eol(
        text: State,
        eol: State,
        crlf: State,
        default_eol_attributes: EolConversionMode,
        eol_conversion_mode: EolConversionMode,
        (contents, expect): (&[u8], &[u8]),
    ) {
        let settings = EolConversionSettings {
            use_git_attributes: true,
            default_eol_attributes,
            eol_conversion_mode,
        };
        let strategy = TargetEolStrategy::new(settings);
        let mut output = vec![];
        strategy
            .convert_eol_for_update(contents, async || Ok(EolGitAttributes { eol, text, crlf }))
            .await
            .unwrap()
            .read_to_end(&mut output)
            .await
            .unwrap();
        assert_eq!(output, expect);
    }

    #[tokio::main(flavor = "current_thread")]
    #[test_matrix(
        [
            State::Set,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"auto".to_vec()),
        ],
        State::Value(b"lf".to_vec()),
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [b"a\n", b"\0a\n"];
        "eol=lf"
    )]
    #[test_matrix(
        [
            State::Set,
            State::Value(b"auto".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        EolConversionMode::Input,
        EolConversionMode::None,
        [b"a\n", b"\0a\n"];
        "gitattributes-default-eol=input"
    )]
    #[test_matrix(
        [
            State::Set,
            State::Value(b"auto".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        EolConversionMode::Input,
        [b"a\n", b"\0a\n"];
        "eol-conversion=input"
    )]
    #[test_matrix(
        State::Unset,
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"lf".to_vec()),
            State::Value(b"crlf".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [b"a\n", b"\0a\n"];
        "-text"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Unset,
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [b"a\n", b"\0a\n"];
        "crlf=input or -crlf"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
        ],
        [b"a\n", b"\0a\n"];
        "eol-conversion is none or input"
    )]
    #[test_matrix(
        State::Value(b"auto".to_vec()),
        State::Value(b"crlf".to_vec()),
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        b"\0a\n";
        "text=auto eol=crlf binary contents"
    )]
    #[test_matrix(
        State::Value(b"auto".to_vec()),
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::InputOutput,
        ],
        EolConversionMode::None,
        b"\0a\n";
        "text=auto gitattributes-default-eol is none or input-output binary contents"
    )]
    #[test_matrix(
        State::Value(b"auto".to_vec()),
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput,
        ],
        EolConversionMode::InputOutput,
        b"\0a\n";
        "text=auto eol-conversion=input-ouput binary contents"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput,
        ],
        EolConversionMode::InputOutput,
        b"\0a\n";
        "eol-conversion=input-ouput binary contents"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Set,
        EolConversionMode::Input,
        EolConversionMode::None,
        [b"a\n", b"\0a\n"];
        "crlf set gitattributes-default-eol=input"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Set,
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput,
        ],
        EolConversionMode::Input,
        [b"a\n", b"\0a\n"];
        "crlf set eol-conversion=input"
    )]
    async fn test_eol_update_should_not_convert_eol(
        text: State,
        eol: State,
        crlf: State,
        default_eol_attributes: EolConversionMode,
        eol_conversion_mode: EolConversionMode,
        contents: &[u8],
    ) {
        let settings = EolConversionSettings {
            use_git_attributes: true,
            default_eol_attributes,
            eol_conversion_mode,
        };
        let strategy = TargetEolStrategy::new(settings);
        let mut output = vec![];
        strategy
            .convert_eol_for_update(contents, async || Ok(EolGitAttributes { eol, text, crlf }))
            .await
            .unwrap()
            .read_to_end(&mut output)
            .await
            .unwrap();
        assert_eq!(output, contents);
    }

    #[cfg(windows)]
    const NATIVE_EOL: &str = "\r\n";
    #[cfg(not(windows))]
    const NATIVE_EOL: &str = "\n";

    #[tokio::main(flavor = "current_thread")]
    #[test_matrix(
        State::Value(b"auto".to_vec()),
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [(b"a\n", format!("a{NATIVE_EOL}"))];
        "text=auto"
    )]
    #[test_matrix(
        State::Set,
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [(b"a\n", format!("a{NATIVE_EOL}")), (b"\0a\n", format!("\0a{NATIVE_EOL}"))];
        "text is set"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Set,
        [(b"a\n", format!("a{NATIVE_EOL}")), (b"\0a\n", format!("\0a{NATIVE_EOL}"))];
        "crlf is set"
    )]
    async fn test_eol_update_eol_conversion_platform_specific(
        text: State,
        eol: State,
        crlf: State,
        (contents, expect): (&[u8], String),
    ) {
        let settings = EolConversionSettings {
            use_git_attributes: true,
            default_eol_attributes: EolConversionMode::None,
            eol_conversion_mode: EolConversionMode::None,
        };
        let strategy = TargetEolStrategy::new(settings);
        let mut output = vec![];
        strategy
            .convert_eol_for_update(contents, async || Ok(EolGitAttributes { eol, text, crlf }))
            .await
            .unwrap()
            .read_to_end(&mut output)
            .await
            .unwrap();
        assert_eq!(output, expect.as_bytes());
    }

    struct UnreachableReader;

    impl AsyncRead for UnreachableReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            unreachable!()
        }
    }

    #[cfg(not(windows))]
    #[tokio::main(flavor = "current_thread")]
    #[test_matrix(
        State::Set,
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ];
        "text is set"
    )]
    #[test_matrix(
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Set;
        "crlf is set"
    )]
    async fn test_eol_update_wont_read_platform_specific(text: State, eol: State, crlf: State) {
        let settings = EolConversionSettings {
            use_git_attributes: true,
            default_eol_attributes: EolConversionMode::None,
            eol_conversion_mode: EolConversionMode::None,
        };
        let strategy = TargetEolStrategy::new(settings);
        strategy
            .convert_eol_for_update(UnreachableReader, async || {
                Ok(EolGitAttributes { eol, text, crlf })
            })
            .await
            .unwrap();
    }

    async fn convert_eol_for_snapshot<'a>(
        target_eol_strategy: &'a TargetEolStrategy,
        contents: &'a mut (dyn AsyncRead + Send + Unpin),
        git_attributes: EolGitAttributes,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin + 'a>, BoxError> {
        target_eol_strategy
            .convert_eol_for_snapshot(contents, async || Ok(git_attributes))
            .await
    }

    async fn convert_eol_for_update<'a>(
        target_eol_strategy: &'a TargetEolStrategy,
        contents: &'a mut (dyn AsyncRead + Send + Unpin),
        git_attributes: EolGitAttributes,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin + 'a>, BoxError> {
        target_eol_strategy
            .convert_eol_for_update(contents, async || Ok(git_attributes))
            .await
    }

    #[tokio::main(flavor = "current_thread")]
    #[test_matrix(
        [
            convert_eol_for_snapshot,
            convert_eol_for_update,
        ],
        State::Unset,
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"lf".to_vec()),
            State::Value(b"crlf".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ];
        "-text"
    )]
    #[test_matrix(
        [
            convert_eol_for_snapshot,
            convert_eol_for_update,
        ],
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Unset,
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ];
        "-crlf"
    )]
    #[test_matrix(
        [
            convert_eol_for_snapshot,
            convert_eol_for_update,
        ],
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        EolConversionMode::None;
        "eol-conversion=none"
    )]
    #[test_matrix(
        convert_eol_for_update,
        [
            State::Set,
            State::Value(b"auto".to_vec()),
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Value(b"lf".to_vec()),
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput
        ];
        "eol=lf on update"
    )]
    #[test_matrix(
        convert_eol_for_update,
        [
            State::Set,
            State::Value(b"auto".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        EolConversionMode::Input,
        EolConversionMode::None;
        "gitattributes-default-eol=input on update"
    )]
    #[test_matrix(
        convert_eol_for_update,
        [
            State::Set,
            State::Value(b"auto".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
            State::Value(b"input".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput,
        ],
        EolConversionMode::Input;
        "text set or text=auto eol-conversion=input on update"
    )]
    #[test_matrix(
        convert_eol_for_update,
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Value(b"input".to_vec()),
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput,
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput,
        ];
        "crlf=input on update"
    )]
    #[test_matrix(
        convert_eol_for_update,
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput,
        ],
        EolConversionMode::Input;
        "not gitattr eol-conversion=input on update"
    )]
    #[test_matrix(
        convert_eol_for_update,
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Set,
        EolConversionMode::Input,
        EolConversionMode::None;
        "crlf set gitattributes-default-eol=input on update"
    )]
    #[test_matrix(
        convert_eol_for_update,
        [
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        [
            State::Set,
            State::Unset,
            State::Unspecified,
            State::Value(b"wrong_value".to_vec()),
        ],
        State::Set,
        [
            EolConversionMode::None,
            EolConversionMode::Input,
            EolConversionMode::InputOutput,
        ],
        EolConversionMode::Input;
        "crlf set eol-conversion=input on update"
    )]
    async fn test_eol_wont_read(
        operation: impl for<'a> AsyncFn(
            &'a TargetEolStrategy,
            &'a mut (dyn AsyncRead + Send + Unpin),
            EolGitAttributes,
        )
            -> Result<Box<dyn AsyncRead + Send + Unpin + 'a>, BoxError>,
        text: State,
        eol: State,
        crlf: State,
        default_eol_attributes: EolConversionMode,
        eol_conversion_mode: EolConversionMode,
    ) {
        let settings = EolConversionSettings {
            use_git_attributes: true,
            default_eol_attributes,
            eol_conversion_mode,
        };
        let strategy = TargetEolStrategy::new(settings);
        operation(
            &strategy,
            &mut UnreachableReader,
            EolGitAttributes { eol, text, crlf },
        )
        .await
        .unwrap();
    }
}
