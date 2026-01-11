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

fn run_verify_command(command: &mut Command, input: &[u8]) -> Result<Vec<u8>, SqError> {
    tracing::info!(?command, "running sq verify command");
    let process = command.stderr(Stdio::piped()).spawn()?;
    let write_result = process.stdin.as_ref().unwrap().write_all(input);
    let output = process.wait_with_output()?;
    tracing::info!(?command, ?output.status, "sq verify command exited");

    match write_result {
        Ok(()) => Ok(output.stderr),
        Err(err) => Err(err.into()),
    }
}

fn parse_verify_output(cmd_output: Result<Vec<u8>, SqError>) -> Result<Verification, SignError> {
    let verification = match cmd_output {
        Ok(output) => {
            let output = output.as_slice().to_str_lossy();

            // Regex for capturing key label and user ID from output when the
            // signature was verified & authenticated using a trusted cert
            static AUTHENTICATED_RE: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r"Authenticated signature made by (.+) \((.+)\)").unwrap()
            });

            // Regex for capturing key label and user ID from output when the
            // signature was verified & authenticated via 1 or more trusted introducer certs
            static NOTARIZED_RE: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r"Authenticated level \d+ notarization made by (.+) \((.+)\)").unwrap()
            });

            let Some(captures) = AUTHENTICATED_RE
                .captures(&output)
                .or_else(|| NOTARIZED_RE.captures(&output))
            else {
                // `sq verify` succeeded, but the stderr output couldn't be parsed to get the
                // key & user id.
                //
                // This shouldn't happen unless sq makes breaking changes to its verify output,
                // but we'll still show the signature status as "good".
                return Ok(Verification::new(SigStatus::Good, None, None));
            };

            let fingerprint = &captures[1];
            let user_id = &captures[2];

            Verification::new(
                SigStatus::Good,
                Some(fingerprint.to_owned()),
                Some(user_id.to_owned()),
            )
        }
        Err(SqError::Command { stderr, .. }) => {
            if ["bad signature", "bad key", "broken signature"]
                .into_iter()
                .any(|pattern| stderr.contains(pattern))
            {
                Verification::new(SigStatus::Bad, None, None)
            } else {
                Verification::new(SigStatus::Unknown, None, None)
            }
        }
        Err(e) => return Err(e.into()),
    };

    Ok(verification)
}

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

        let output = run_verify_command(
            self.create_command()
                .args(["verify", "--signature-file"])
                .arg(&sig_path)
                .arg("-"),
            data,
        );

        parse_verify_output(output)
    }
}
