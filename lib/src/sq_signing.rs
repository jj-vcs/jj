#![expect(missing_docs)]

use std::ffi::OsString;
use std::io::Write as _;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::LazyLock;

use bstr::ByteSlice as _;
use regex::Regex;
use thiserror::Error;

use crate::config::ConfigGetError;
use crate::settings::UserSettings;
use crate::signing::SigStatus;
use crate::signing::SignError;
use crate::signing::SignResult;
use crate::signing::SigningBackend;
use crate::signing::Verification;

/// Signing backend using the Sequoia PGP CLI for sigining & verifying.
#[derive(Debug)]
pub struct SqBackend {
    program: OsString,
    default_key: String,
}

#[derive(Debug, Error)]
pub enum SqError {
    #[error("sq failed with {exit_status}:\n{stderr}")]
    Command {
        exit_status: ExitStatus,
        stderr: String,
    },
    #[error("Failed to run sq")]
    Io(#[from] std::io::Error),
}

impl From<SqError> for SignError {
    fn from(e: SqError) -> Self {
        Self::Backend(Box::new(e))
    }
}

impl SqBackend {
    pub fn new(program: OsString, default_key: String) -> Self {
        Self {
            program,
            default_key,
        }
    }

    pub fn from_settings(settings: &UserSettings) -> Result<Self, ConfigGetError> {
        let program = settings.get_string("signing.backends.sq.program")?;
        let default_key = settings.user_email().to_owned();

        Ok(Self::new(program.into(), default_key))
    }

    fn create_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        // Hide console window on Windows (https://stackoverflow.com/a/60958956)
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        command.stdin(Stdio::piped()).stdout(Stdio::piped());
        command
    }
}

impl SigningBackend for SqBackend {
    fn name(&self) -> &'static str {
        "sq"
    }

    fn can_read(&self, signature: &[u8]) -> bool {
        signature.starts_with(b"-----BEGIN PGP SIGNATURE-----")
    }

    fn sign(&self, data: &[u8], key: Option<&str>) -> SignResult<Vec<u8>> {
        let signer_id = key.unwrap_or(&self.default_key);

        // `sq sign` has different options for using email as the key lookup vs key
        // fingerprint.
        //
        // We make a (perhaps hasty) generalization here that if the key contains `@`,
        // we should treat it as an email and otherwise as a fingerprint/key-id.
        let sign_result = if signer_id.contains("@") {
            run_sign_command(
                self.create_command().args([
                    "sign",
                    "--signer-email",
                    signer_id,
                    "--signature-file",
                    "-",
                    "-",
                ]),
                data,
            )?
        } else {
            run_sign_command(
                self.create_command().args([
                    "sign",
                    "--signer",
                    signer_id,
                    "--signature-file",
                    "-",
                    "-",
                ]),
                data,
            )?
        };

        Ok(sign_result)
    }

    fn verify(&self, data: &[u8], signature: &[u8]) -> SignResult<Verification> {
        let mut signature_file = tempfile::Builder::new()
            .prefix(".jj-sq-sig-tmp-")
            .tempfile()
            .map_err(SqError::Io)?;
        signature_file.write_all(signature).map_err(SqError::Io)?;
        signature_file.flush().map_err(SqError::Io)?;

        let sig_path = signature_file.into_temp_path();

        let cmd_output = run_verify_command(
            self.create_command()
                .args(["verify", "--signature-file"])
                .arg(&sig_path)
                .arg("-"),
            data,
        );

        let verification = cmd_output
            .map(|(success, output)| parse_verify_output(success, &output.to_str_lossy()))
            .map_err(SqError::from)?;

        Ok(verification)
    }
}

fn run_sign_command(command: &mut Command, input: &[u8]) -> Result<Vec<u8>, SqError> {
    tracing::info!(?command, "running sq sign command");
    let process = command.stderr(Stdio::piped()).spawn()?;
    let write_result = process.stdin.as_ref().unwrap().write_all(input);
    let output = process.wait_with_output()?;
    tracing::info!(?command, ?output.status, "sq sign command exited");
    if output.status.success() {
        write_result?;
        Ok(output.stdout)
    } else {
        Err(SqError::Command {
            exit_status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim_end().into(),
        })
    }
}

fn run_verify_command(
    command: &mut Command,
    input: &[u8],
) -> Result<(bool, Vec<u8>), std::io::Error> {
    tracing::info!(?command, "running sq verify command");
    let process = command.stderr(Stdio::piped()).spawn()?;
    let write_result = process.stdin.as_ref().unwrap().write_all(input);
    let output = process.wait_with_output()?;
    tracing::info!(?command, ?output.status, "sq verify command exited");

    // `sq verify` exits with non-zero exit codes when e.g., signature is bad, the
    // certificate can't be verified, etc.
    //
    // Regardless of success or failure, the output is written to stderr, so we
    // consider the result of running the command a success as long as an
    // io::Error doesn't occur.
    write_result?;
    Ok((output.status.success(), output.stderr))
}

/// Parses a [`Verification`] from the command output when running `sq verify`
/// The output of this command can vary widely depending on the user's
/// trusted/linked certs in their PKI and is meant to be human readable
/// more than machine parseable.
///
/// This implementation uses regex to search for specific phrases in the output
/// and capture groups to parse out key fingerprint and user id information.
fn parse_verify_output(success: bool, cmd_output: &str) -> Verification {
    static AUTHENTICATED_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"Authenticated (?:signature|level \d+ notarization) made by (\w+) \((.+)\)")
            .unwrap()
    });
    static ERR_VERIFICATION_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Error verifying signature made by (\w+):").unwrap());
    static ERR_AUTHENTICATION_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"Can't authenticate signature (?:allegedly)? made by (.+):").unwrap()
    });

    if success && let Some(captures) = AUTHENTICATED_RE.captures(cmd_output) {
        // Signature was verified and the key's cert was authenticated by web of trust.
        let fingerprint = captures[1].to_owned();
        let user_id = captures[2].to_owned();

        return Verification::new(SigStatus::Good, Some(fingerprint), Some(user_id));
    } else if success {
        // Signature was verified and authenticated, but the command output looks
        // different than expected/documented.
        //
        // This shouldn't happen, but we should still mark the verification as `Good`
        // since we received a zero exit code.
        return Verification::new(SigStatus::Good, None, None);
    }

    let fingerprint = ERR_VERIFICATION_RE
        .captures(cmd_output)
        .or_else(|| ERR_AUTHENTICATION_RE.captures(cmd_output))
        .map(|c| c[1].to_owned());

    // Extra parts of output to check for to make sure we're not marking a known
    // bad signature as `SigStatus::Unknown`
    const BAD_SIG_REASONS: &[&str] = &["bad signature", "bad key", "broken signature"];

    if ERR_VERIFICATION_RE.is_match(cmd_output)
        || BAD_SIG_REASONS
            .iter()
            .any(|pattern| cmd_output.contains(pattern))
    {
        return Verification::new(SigStatus::Bad, fingerprint, None);
    }

    // Most common case here is the signature is correct but the user hasn't
    // added/linked the key fingerprint in their PKI web of trust.
    Verification::new(SigStatus::Unknown, fingerprint, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_parsed_verification(successful: bool, output: &str, expected: Verification) {
        let actual = parse_verify_output(successful, output);
        assert_eq!(expected, actual);
    }

    #[test]
    fn parse_verify_output_success() {
        let successful = true;
        let output = r#"
            Authenticating B535B0D4736F809892B42F4A388344D1DEAA4483 (Alice) using the web of trust:
              Fully authenticated (120 of 120) B535B0D4736F809892B42F4A388344D1DEAA4483, <alice@example.com>
              ◯─┬ E100FB15031CA14658A60ED0299FFA52B2D9174A
              │ └ (Local Trust Root)
              │
              │  certified the following certificate on 2026‑01‑08 as a meta-introducer (depth: unconstrained)
              │
              ├─┬ B535B0D4736F809892B42F4A388344D1DEAA4483
              │ └ (<alice@example.com>)
              │
              │  certified the following binding on 2026‑01‑13
              │
              └─┬ B535B0D4736F809892B42F4A388344D1DEAA4483
                └ <alice@example.com>

              Fully authenticated (120 of 120) B535B0D4736F809892B42F4A388344D1DEAA4483, Alice
                ◯─┬ E100FB15031CA14658A60ED0299FFA52B2D9174A
                │ └ (Local Trust Root)
                │
                │  certified the following binding on 2026‑01‑08 as a meta-introducer (depth: unconstrained)
                │
                └─┬ B535B0D4736F809892B42F4A388344D1DEAA4483
                  └ Alice

              Authenticated signature made by B535B0D4736F809892B42F4A388344D1DEAA4483 (<alice@example.com>)

            1 authenticated signature.
        "#;

        let expected = Verification::new(
            SigStatus::Good,
            Some("B535B0D4736F809892B42F4A388344D1DEAA4483".to_owned()),
            Some("<alice@example.com>".to_owned()),
        );
        assert_parsed_verification(successful, output, expected);
    }

    #[test]
    fn parse_verify_output_success_unexpected_output() {
        let successful = true;
        let output = "";
        let expected = Verification::new(SigStatus::Good, None, None);
        assert_parsed_verification(successful, output, expected);
    }

    #[test]
    fn parse_verify_output_err_bad_signature() {
        let successful = false;
        let output = r#"
            Error verifying signature made by B535B0D4736F809892B42F4A388344D1DEAA4483:

              Error: Message has been manipulated
            0 authenticated signatures, 1 bad signature.

              Error: Verification failed: could not authenticate any signatures
        "#;
        let expected = Verification::new(
            SigStatus::Bad,
            Some("B535B0D4736F809892B42F4A388344D1DEAA4483".to_owned()),
            None,
        );
        assert_parsed_verification(successful, output, expected);
    }

    #[test]
    fn parse_verify_output_err_unauthenticated() {
        let successful = false;
        let output = r#"
            Can't authenticate signature allegedly made by B535B0D4736F809892B42F4A388344D1DEAA4483: missing certificate.

            Hint: Consider searching for the certificate using:

              $ sq network search B535B0D4736F809892B42F4A388344D1DEAA4483
            0 authenticated signatures, 1 uncheckable signature.

              Error: Verification failed: could not authenticate any signatures
        "#;
        let expected = Verification::new(
            SigStatus::Unknown,
            Some("B535B0D4736F809892B42F4A388344D1DEAA4483".to_owned()),
            None,
        );
        assert_parsed_verification(successful, output, expected);
    }
}
