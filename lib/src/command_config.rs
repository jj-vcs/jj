// Copyright 2022 The Jujutsu Authors
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

//! Types for specifying external command names and arguments via config.
//!
//! [`CommandNameAndArgs`] is used throughout jj for tool configuration such
//! as diff tools, merge tools, fix tools, and gitattributes filter drivers.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::process::Command;
use std::sync::LazyLock;

use regex::Captures;
use regex::Regex;

/// Command name and arguments specified by config.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(untagged)]
pub enum CommandNameAndArgs {
    /// A single string that will be split on spaces (or with shell quoting).
    String(String),
    /// An array of command name followed by arguments.
    Vec(NonEmptyCommandArgsVec),
    /// A structured form with optional environment variables.
    Structured {
        /// Environment variables to set for the command.
        env: HashMap<String, String>,
        /// The command and its arguments.
        command: NonEmptyCommandArgsVec,
    },
}

impl CommandNameAndArgs {
    /// Returns command name without arguments.
    pub fn split_name(&self) -> Cow<'_, str> {
        let (name, _) = self.split_name_and_args();
        name
    }

    /// Returns command name and arguments.
    ///
    /// The command name may be an empty string (as well as each argument.)
    pub fn split_name_and_args(&self) -> (Cow<'_, str>, Cow<'_, [String]>) {
        match self {
            Self::String(s) => {
                if s.contains('"') || s.contains('\'') {
                    let mut parts = shlex::Shlex::new(s);
                    let res = (
                        parts.next().unwrap_or_default().into(),
                        parts.by_ref().collect(),
                    );
                    if !parts.had_error {
                        return res;
                    }
                }
                let mut args = s.split(' ').map(|s| s.to_owned());
                (args.next().unwrap().into(), args.collect())
            }
            Self::Vec(NonEmptyCommandArgsVec(a)) => (Cow::Borrowed(&a[0]), Cow::Borrowed(&a[1..])),
            Self::Structured {
                env: _,
                command: cmd,
            } => (Cow::Borrowed(&cmd.0[0]), Cow::Borrowed(&cmd.0[1..])),
        }
    }

    /// Returns command string only if the underlying type is a string.
    ///
    /// Use this to parse enum strings such as `":builtin"`, which can be
    /// escaped as `[":builtin"]`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            Self::Vec(_) | Self::Structured { .. } => None,
        }
    }

    /// Returns process builder configured with this.
    pub fn to_command(&self) -> Command {
        let empty: HashMap<&str, &str> = HashMap::new();
        self.to_command_with_variables(&empty)
    }

    /// Returns process builder configured with this after interpolating
    /// variables into the arguments.
    pub fn to_command_with_variables<V: AsRef<str>>(
        &self,
        variables: &HashMap<&str, V>,
    ) -> Command {
        let (name, args) = self.split_name_and_args();
        let mut cmd = Command::new(interpolate_variables_single(name.as_ref(), variables));
        if let Self::Structured { env, .. } = self {
            cmd.envs(env);
        }
        cmd.args(interpolate_variables(&args, variables));
        cmd
    }
}

impl<T: AsRef<str> + ?Sized> From<&T> for CommandNameAndArgs {
    fn from(s: &T) -> Self {
        Self::String(s.as_ref().to_owned())
    }
}

impl fmt::Display for CommandNameAndArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(s) => write!(f, "{s}"),
            // TODO: format with shell escapes
            Self::Vec(a) => write!(f, "{}", a.0.join(" ")),
            Self::Structured { env, command } => {
                for (k, v) in env {
                    write!(f, "{k}={v} ")?;
                }
                write!(f, "{}", command.0.join(" "))
            }
        }
    }
}

// Not interested in $UPPER_CASE_VARIABLES
static VARIABLE_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$([a-z0-9_]+)\b").unwrap());

/// Substitutes `$variable` patterns in `args` using the provided map.
pub fn interpolate_variables<V: AsRef<str>>(
    args: &[String],
    variables: &HashMap<&str, V>,
) -> Vec<String> {
    args.iter()
        .map(|arg| interpolate_variables_single(arg, variables))
        .collect()
}

fn interpolate_variables_single<V: AsRef<str>>(arg: &str, variables: &HashMap<&str, V>) -> String {
    VARIABLE_REGEX
        .replace_all(arg, |caps: &Captures| {
            let name = &caps[1];
            if let Some(subst) = variables.get(name) {
                subst.as_ref().to_owned()
            } else {
                caps[0].to_owned()
            }
        })
        .into_owned()
}

/// Return all variable names found in the args, without the dollar sign
pub fn find_all_variables(args: &[String]) -> impl Iterator<Item = &str> {
    let regex = &*VARIABLE_REGEX;
    args.iter()
        .flat_map(|arg| regex.find_iter(arg))
        .map(|single_match| {
            let s = single_match.as_str();
            &s[1..]
        })
}

/// Wrapper to reject an array without command name.
// Based on https://github.com/serde-rs/serde/issues/939
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Deserialize)]
#[serde(try_from = "Vec<String>")]
pub struct NonEmptyCommandArgsVec(Vec<String>);

impl TryFrom<Vec<String>> for NonEmptyCommandArgsVec {
    type Error = &'static str;

    fn try_from(args: Vec<String>) -> Result<Self, Self::Error> {
        if args.is_empty() {
            Err("command arguments should not be empty")
        } else {
            Ok(Self(args))
        }
    }
}

#[cfg(test)]
mod tests {
    use maplit::hashmap;

    use super::*;

    #[test]
    fn test_string_split_name_and_args() {
        let cmd = CommandNameAndArgs::String("emacs -nw".to_owned());
        let (name, args) = cmd.split_name_and_args();
        assert_eq!(name, "emacs");
        assert_eq!(args.as_ref(), ["-nw"]);
    }

    #[test]
    fn test_string_quoted_split_name_and_args() {
        let cmd = CommandNameAndArgs::String(r#""spaced path/emacs" -nw"#.to_owned());
        let (name, args) = cmd.split_name_and_args();
        assert_eq!(name, "spaced path/emacs");
        assert_eq!(args.as_ref(), ["-nw"]);
    }

    #[test]
    fn test_vec_split_name_and_args() {
        let cmd = CommandNameAndArgs::Vec(NonEmptyCommandArgsVec(
            ["emacs", "-nw"].map(|s| s.to_owned()).to_vec(),
        ));
        let (name, args) = cmd.split_name_and_args();
        assert_eq!(name, "emacs");
        assert_eq!(args.as_ref(), ["-nw"]);
    }

    #[test]
    fn test_structured_split_name_and_args() {
        let cmd = CommandNameAndArgs::Structured {
            env: hashmap! { "KEY".to_owned() => "val".to_owned() },
            command: NonEmptyCommandArgsVec(["emacs", "-nw"].map(|s| s.to_owned()).to_vec()),
        };
        let (name, args) = cmd.split_name_and_args();
        assert_eq!(name, "emacs");
        assert_eq!(args.as_ref(), ["-nw"]);
    }

    #[test]
    fn test_nonempty_args_vec_rejects_empty() {
        let err = NonEmptyCommandArgsVec::try_from(vec![]);
        assert!(err.is_err());
    }

    #[test]
    fn test_interpolate_variables() {
        let vars: HashMap<&str, &str> = hashmap! {
            "path" => "my file.txt",
        };
        let args = vec!["--out=$path".to_owned(), "$path".to_owned()];
        let result = interpolate_variables(&args, &vars);
        assert_eq!(result, vec!["--out=my file.txt", "my file.txt"]);
    }

    #[test]
    fn test_uppercase_variable_not_interpolated() {
        let vars: HashMap<&str, &str> = HashMap::new();
        let args = vec!["$UPPER".to_owned()];
        // $UPPER is all-caps; regex only matches $[a-z0-9_]+ so unchanged
        assert_eq!(interpolate_variables(&args, &vars), vec!["$UPPER"]);
    }

    #[test]
    fn test_find_all_variables() {
        let args = vec![
            "$path".to_owned(),
            "--flag=$dir".to_owned(),
            "$UPPER".to_owned(),
        ];
        let vars: Vec<&str> = find_all_variables(&args).collect();
        // $UPPER not matched (uppercase)
        assert_eq!(vars, vec!["path", "dir"]);
    }
}
