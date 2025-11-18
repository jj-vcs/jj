// Copyright 2024 The Jujutsu Authors
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

use std::collections::HashSet;
use std::io::BufRead as _;
use std::path::Path;

use clap::FromArgMatches as _;
use clap::builder::StyledStr;
use clap_complete::CompletionCandidate;
use indoc::indoc;
use itertools::Itertools as _;
use jj_lib::config::ConfigNamePathBuf;
use jj_lib::file_util::normalize_path;
use jj_lib::file_util::slash_path;
use jj_lib::settings::UserSettings;
use jj_lib::workspace::DefaultWorkspaceLoaderFactory;
use jj_lib::workspace::WorkspaceLoaderFactory as _;

use crate::cli_util::GlobalArgs;
use crate::cli_util::expand_args;
use crate::cli_util::find_workspace_dir;
use crate::cli_util::load_template_aliases;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::config::CONFIG_SCHEMA;
use crate::config::ConfigArgKind;
use crate::config::ConfigEnv;
use crate::config::config_from_environment;
use crate::config::default_config_layers;
use crate::merge_tools::ExternalMergeTool;
use crate::merge_tools::configured_merge_tools;
use crate::merge_tools::get_external_tool_config;
use crate::revset_util::load_revset_aliases;
use crate::ui::Ui;

const BOOKMARK_HELP_TEMPLATE: &str = r#"template-aliases.'bookmark_help()'='''
" " ++
if(normal_target,
    if(normal_target.description(),
        normal_target.description().first_line(),
        "(no description set)",
    ),
    "(conflicted bookmark)",
)
'''"#;
const TAG_HELP_TEMPLATE: &str = r#"template-aliases.'tag_help()'='''
" " ++
if(normal_target,
    if(normal_target.description(),
        normal_target.description().first_line(),
        "(no description set)",
    ),
    "(conflicted tag)",
)
'''"#;

/// A helper function for various completer functions. It returns
/// (candidate, help) assuming they are separated by a space.
fn split_help_text(line: &str) -> (&str, Option<StyledStr>) {
    match line.split_once(' ') {
        Some((name, help)) => (name, Some(help.to_string().into())),
        None => (line, None),
    }
}

pub fn local_bookmarks() -> Vec<CompletionCandidate> {
    with_jj(|jj, _| {
        let output = jj
            .build()
            .arg("bookmark")
            .arg("list")
            .arg("--config")
            .arg(BOOKMARK_HELP_TEMPLATE)
            .arg("--template")
            .arg(r#"if(!remote, name ++ bookmark_help()) ++ "\n""#)
            .output()
            .map_err(user_error)?;

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(split_help_text)
            .map(|(name, help)| CompletionCandidate::new(name).help(help))
            .collect())
    })
}

pub fn tracked_bookmarks() -> Vec<CompletionCandidate> {
    with_jj(|jj, _| {
        let output = jj
            .build()
            .arg("bookmark")
            .arg("list")
            .arg("--tracked")
            .arg("--config")
            .arg(BOOKMARK_HELP_TEMPLATE)
            .arg("--template")
            .arg(r#"if(remote, name ++ '@' ++ remote ++ bookmark_help() ++ "\n")"#)
            .output()
            .map_err(user_error)?;

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(split_help_text)
            .map(|(name, help)| CompletionCandidate::new(name).help(help))
            .collect())
    })
}

pub fn untracked_bookmarks() -> Vec<CompletionCandidate> {
    with_jj(|jj, _settings| {
        let remotes = jj
            .build()
            .arg("git")
            .arg("remote")
            .arg("list")
            .output()
            .map_err(user_error)?;
        let remotes = String::from_utf8_lossy(&remotes.stdout);
        let remotes = remotes
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .collect_vec();

        let bookmark_table = jj
            .build()
            .arg("bookmark")
            .arg("list")
            .arg("--all-remotes")
            .arg("--config")
            .arg(BOOKMARK_HELP_TEMPLATE)
            .arg("--template")
            .arg(
                r#"
                if(remote != "git",
                    if(!remote, name) ++ "\t" ++
                    if(remote, name ++ "@" ++ remote) ++ "\t" ++
                    if(tracked, "tracked") ++ "\t" ++
                    bookmark_help() ++ "\n"
                )"#,
            )
            .output()
            .map_err(user_error)?;
        let bookmark_table = String::from_utf8_lossy(&bookmark_table.stdout);

        let mut possible_bookmarks_to_track = Vec::new();
        let mut already_tracked_bookmarks = HashSet::new();

        for line in bookmark_table.lines() {
            let [local, remote, tracked, help] =
                line.split('\t').collect_array().unwrap_or_default();

            if !local.is_empty() {
                possible_bookmarks_to_track.extend(
                    remotes
                        .iter()
                        .map(|remote| (format!("{local}@{remote}"), help)),
                );
            } else if tracked.is_empty() {
                possible_bookmarks_to_track.push((remote.to_owned(), help));
            } else {
                already_tracked_bookmarks.insert(remote);
            }
        }
        possible_bookmarks_to_track
            .retain(|(bookmark, _help)| !already_tracked_bookmarks.contains(&bookmark.as_str()));

        Ok(possible_bookmarks_to_track
            .into_iter()
            .map(|(bookmark, help)| {
                CompletionCandidate::new(bookmark).help(Some(help.to_string().into()))
            })
            .collect())
    })
}

pub fn bookmarks() -> Vec<CompletionCandidate> {
    with_jj(|jj, _settings| {
        let output = jj
            .build()
            .arg("bookmark")
            .arg("list")
            .arg("--all-remotes")
            .arg("--config")
            .arg(BOOKMARK_HELP_TEMPLATE)
            .arg("--template")
            .arg(
                // only provide help for local refs, remote could be ambiguous
                r#"name ++ if(remote, "@" ++ remote, bookmark_help()) ++ "\n""#,
            )
            .output()
            .map_err(user_error)?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        Ok((&stdout
            .lines()
            .map(split_help_text)
            .chunk_by(|(name, _)| name.split_once('@').map(|t| t.0).unwrap_or(name)))
            .into_iter()
            .map(|(bookmark, mut refs)| {
                let help = refs.find_map(|(_, help)| help);
                let local = help.is_some();
                let display_order = match local {
                    true => 0,
                    false => 1,
                };
                CompletionCandidate::new(bookmark)
                    .help(help)
                    .display_order(Some(display_order))
            })
            .collect())
    })
}

pub fn local_tags() -> Vec<CompletionCandidate> {
    with_jj(|jj, _| {
        let output = jj
            .build()
            .arg("tag")
            .arg("list")
            .arg("--config")
            .arg(TAG_HELP_TEMPLATE)
            .arg("--template")
            .arg(r#"if(!remote, name ++ tag_help()) ++ "\n""#)
            .output()
            .map_err(user_error)?;

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(split_help_text)
            .map(|(name, help)| CompletionCandidate::new(name).help(help))
            .collect())
    })
}

pub fn git_remotes() -> Vec<CompletionCandidate> {
    with_jj(|jj, _| {
        let output = jj
            .build()
            .arg("git")
            .arg("remote")
            .arg("list")
            .output()
            .map_err(user_error)?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        Ok(stdout
            .lines()
            .filter_map(|line| line.split_once(' ').map(|(name, _url)| name))
            .map(CompletionCandidate::new)
            .collect())
    })
}

pub fn template_aliases() -> Vec<CompletionCandidate> {
    with_jj(|_, settings| {
        let Ok(template_aliases) = load_template_aliases(&Ui::null(), settings.config()) else {
            return Ok(Vec::new());
        };
        Ok(template_aliases
            .symbol_names()
            .map(CompletionCandidate::new)
            .sorted()
            .collect())
    })
}

pub fn aliases() -> Vec<CompletionCandidate> {
    with_jj(|_, settings| {
        Ok(settings
            .table_keys("aliases")
            // This is opinionated, but many people probably have several
            // single- or two-letter aliases they use all the time. These
            // aliases don't need to be completed and they would only clutter
            // the output of `jj <TAB>`.
            .filter(|alias| alias.len() > 2)
            .map(CompletionCandidate::new)
            .collect())
    })
}

fn revisions(match_prefix: &str, revset_filter: Option<&str>) -> Vec<CompletionCandidate> {
    with_jj(|jj, settings| {
        // display order
        const LOCAL_BOOKMARK: usize = 0;
        const TAG: usize = 1;
        const CHANGE_ID: usize = 2;
        const REMOTE_BOOKMARK: usize = 3;
        const REVSET_ALIAS: usize = 4;

        let mut candidates = Vec::new();

        // bookmarks

        let mut cmd = jj.build();
        cmd.arg("bookmark")
            .arg("list")
            .arg("--all-remotes")
            .arg("--config")
            .arg(BOOKMARK_HELP_TEMPLATE)
            .arg("--template")
            .arg(
                r#"if(remote != "git", name ++ if(remote, "@" ++ remote) ++ bookmark_help() ++ "\n")"#,
            );
        if let Some(revs) = revset_filter {
            cmd.arg("--revisions").arg(revs);
        }
        let output = cmd.output().map_err(user_error)?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        candidates.extend(
            stdout
                .lines()
                .map(split_help_text)
                .filter(|(bookmark, _)| bookmark.starts_with(match_prefix))
                .map(|(bookmark, help)| {
                    let local = !bookmark.contains('@');
                    let display_order = match local {
                        true => LOCAL_BOOKMARK,
                        false => REMOTE_BOOKMARK,
                    };
                    CompletionCandidate::new(bookmark)
                        .help(help)
                        .display_order(Some(display_order))
                }),
        );

        // tags

        // Tags cannot be filtered by revisions. In order to avoid suggesting
        // immutable tags for mutable revision args, we skip tags entirely if
        // revset_filter is set. This is not a big loss, since tags usually point
        // to immutable revisions anyway.
        if revset_filter.is_none() {
            let output = jj
                .build()
                .arg("tag")
                .arg("list")
                .arg("--config")
                .arg(BOOKMARK_HELP_TEMPLATE)
                .arg("--template")
                .arg(r#"name ++ bookmark_help() ++ "\n""#)
                .arg(format!("glob:{}*", globset::escape(match_prefix)))
                .output()
                .map_err(user_error)?;
            let stdout = String::from_utf8_lossy(&output.stdout);

            candidates.extend(stdout.lines().map(|line| {
                let (name, desc) = split_help_text(line);
                CompletionCandidate::new(name)
                    .help(desc)
                    .display_order(Some(TAG))
            }));
        }

        // change IDs

        let revisions = revset_filter
            .map(String::from)
            .or_else(|| settings.get_string("revsets.short-prefixes").ok())
            .or_else(|| settings.get_string("revsets.log").ok())
            .unwrap_or_default();

        let output = jj
            .build()
            .arg("log")
            .arg("--no-graph")
            .arg("--limit")
            .arg("100")
            .arg("--revisions")
            .arg(revisions)
            .arg("--template")
            .arg(r#"change_id.shortest() ++ " " ++ if(description, description.first_line(), "(no description set)") ++ "\n""#)
            .output()
            .map_err(user_error)?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        candidates.extend(
            stdout
                .lines()
                .map(split_help_text)
                .filter(|(id, _)| id.starts_with(match_prefix))
                .map(|(id, desc)| {
                    CompletionCandidate::new(id)
                        .help(desc)
                        .display_order(Some(CHANGE_ID))
                }),
        );

        // revset aliases

        let revset_aliases = load_revset_aliases(&Ui::null(), settings.config())?;
        let mut symbol_names: Vec<_> = revset_aliases.symbol_names().collect();
        symbol_names.sort();
        candidates.extend(
            symbol_names
                .into_iter()
                .filter(|symbol| symbol.starts_with(match_prefix))
                .map(|symbol| {
                    let (_, defn) = revset_aliases.get_symbol(symbol).unwrap();
                    CompletionCandidate::new(symbol)
                        .help(Some(defn.into()))
                        .display_order(Some(REVSET_ALIAS))
                }),
        );

        Ok(candidates)
    })
}

fn revset_expression(
    current: &std::ffi::OsStr,
    revset_filter: Option<&str>,
) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let (prepend, match_prefix) = split_revset_trailing_name(current).unwrap_or(("", current));
    let candidates = revisions(match_prefix, revset_filter);
    if prepend.is_empty() {
        candidates
    } else {
        candidates
            .into_iter()
            .map(|candidate| candidate.add_prefix(prepend))
            .collect()
    }
}

pub fn revset_expression_all(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    revset_expression(current, None)
}

pub fn revset_expression_mutable(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    revset_expression(current, Some("mutable()"))
}

pub fn revset_expression_mutable_conflicts(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    revset_expression(current, Some("mutable() & conflicts()"))
}

/// Identifies if an incomplete expression ends with a name, or may be continued
/// with a name.
///
/// If the expression ends with an name or a partial name, returns a tuple that
/// splits the string at the point the name starts.
/// If the expression is empty or ends with a prefix or infix operator that
/// could plausibly be followed by a name, returns a tuple where the first
/// item is the entire input string, and the second item is empty.
/// Otherwise, returns `None`.
///
/// The input expression may be incomplete (e.g. missing closing parentheses),
/// and the ability to reject invalid expressions is limited.
fn split_revset_trailing_name(incomplete_revset_str: &str) -> Option<(&str, &str)> {
    let final_part = incomplete_revset_str
        .rsplit_once([':', '~', '|', '&', '(', ','])
        .map(|(_, rest)| rest)
        .unwrap_or(incomplete_revset_str);
    let final_part = final_part
        .rsplit_once("..")
        .map(|(_, rest)| rest)
        .unwrap_or(final_part)
        .trim_ascii_start();

    let re = regex::Regex::new(r"^(?:[\p{XID_CONTINUE}_/]+[@.+-])*[\p{XID_CONTINUE}_/]*$").unwrap();
    re.is_match(final_part)
        .then(|| incomplete_revset_str.split_at(incomplete_revset_str.len() - final_part.len()))
}

pub fn operations() -> Vec<CompletionCandidate> {
    with_jj(|jj, _| {
        let output = jj
            .build()
            .arg("operation")
            .arg("log")
            .arg("--no-graph")
            .arg("--limit")
            .arg("100")
            .arg("--template")
            .arg(
                r#"
                separate(" ",
                    id.short(),
                    "(" ++ format_timestamp(time.end()) ++ ")",
                    description.first_line(),
                ) ++ "\n""#,
            )
            .output()
            .map_err(user_error)?;

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| {
                let (id, help) = split_help_text(line);
                CompletionCandidate::new(id).help(help)
            })
            .collect())
    })
}

pub fn workspaces() -> Vec<CompletionCandidate> {
    let template = indoc! {r#"
        name ++ "\t" ++ if(
            target.description(),
            target.description().first_line(),
            "(no description set)"
        ) ++ "\n"
    "#};
    with_jj(|jj, _| {
        let output = jj
            .build()
            .arg("workspace")
            .arg("list")
            .arg("--template")
            .arg(template)
            .output()
            .map_err(user_error)?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        Ok(stdout
            .lines()
            .filter_map(|line| {
                let res = line.split_once("\t").map(|(name, desc)| {
                    CompletionCandidate::new(name).help(Some(desc.to_string().into()))
                });
                if res.is_none() {
                    eprintln!("Error parsing line {line}");
                }
                res
            })
            .collect())
    })
}

fn merge_tools_filtered_by(
    settings: &UserSettings,
    condition: impl Fn(ExternalMergeTool) -> bool,
) -> impl Iterator<Item = &str> {
    configured_merge_tools(settings).filter(move |name| {
        let Ok(Some(tool)) = get_external_tool_config(settings, name) else {
            return false;
        };
        condition(tool)
    })
}

pub fn merge_editors() -> Vec<CompletionCandidate> {
    with_jj(|_, settings| {
        Ok([":builtin", ":ours", ":theirs"]
            .into_iter()
            .chain(merge_tools_filtered_by(settings, |tool| {
                !tool.merge_args.is_empty()
            }))
            .map(CompletionCandidate::new)
            .collect())
    })
}

/// Approximate list of known diff editors
pub fn diff_editors() -> Vec<CompletionCandidate> {
    with_jj(|_, settings| {
        Ok(std::iter::once(":builtin")
            .chain(merge_tools_filtered_by(
                settings,
                // The args are empty only if `edit-args` are explicitly set to
                // `[]` in TOML. If they are not specified, the default
                // `["$left", "$right"]` value would be used.
                |tool| !tool.edit_args.is_empty(),
            ))
            .map(CompletionCandidate::new)
            .collect())
    })
}

/// Approximate list of known diff tools
pub fn diff_formatters() -> Vec<CompletionCandidate> {
    let builtin_format_kinds = crate::diff_util::all_builtin_diff_format_names();
    with_jj(|_, settings| {
        Ok(builtin_format_kinds
            .iter()
            .map(|s| s.as_str())
            .chain(merge_tools_filtered_by(
                settings,
                // The args are empty only if `diff-args` are explicitly set to
                // `[]` in TOML. If they are not specified, the default
                // `["$left", "$right"]` value would be used.
                |tool| !tool.diff_args.is_empty(),
            ))
            .map(CompletionCandidate::new)
            .collect())
    })
}

fn config_keys_rec(
    prefix: ConfigNamePathBuf,
    properties: &serde_json::Map<String, serde_json::Value>,
    acc: &mut Vec<CompletionCandidate>,
    only_leaves: bool,
    suffix: &str,
) {
    for (key, value) in properties {
        let mut prefix = prefix.clone();
        prefix.push(key);

        let value = value.as_object().unwrap();
        match value.get("type").and_then(|v| v.as_str()) {
            Some("object") => {
                if !only_leaves {
                    let help = value
                        .get("description")
                        .map(|desc| desc.as_str().unwrap().to_string().into());
                    let escaped_key = prefix.to_string();
                    acc.push(CompletionCandidate::new(escaped_key).help(help));
                }
                let Some(properties) = value.get("properties") else {
                    continue;
                };
                let properties = properties.as_object().unwrap();
                config_keys_rec(prefix, properties, acc, only_leaves, suffix);
            }
            _ => {
                let help = value
                    .get("description")
                    .map(|desc| desc.as_str().unwrap().to_string().into());
                let escaped_key = format!("{prefix}{suffix}");
                acc.push(CompletionCandidate::new(escaped_key).help(help));
            }
        }
    }
}

fn json_keypath<'a>(
    schema: &'a serde_json::Value,
    keypath: &str,
    separator: &str,
) -> Option<&'a serde_json::Value> {
    keypath
        .split(separator)
        .try_fold(schema, |value, step| value.get(step))
}
fn jsonschema_keypath<'a>(
    schema: &'a serde_json::Value,
    keypath: &ConfigNamePathBuf,
) -> Option<&'a serde_json::Value> {
    keypath.components().try_fold(schema, |value, step| {
        let value = value.as_object()?;
        if value.get("type")?.as_str()? != "object" {
            return None;
        }
        let properties = value.get("properties")?.as_object()?;
        properties.get(step.get())
    })
}

fn config_values(path: &ConfigNamePathBuf) -> Option<Vec<String>> {
    let schema: serde_json::Value = serde_json::from_str(CONFIG_SCHEMA).unwrap();

    let mut config_entry = jsonschema_keypath(&schema, path)?;
    if let Some(reference) = config_entry.get("$ref") {
        let reference = reference.as_str()?.strip_prefix("#/")?;
        config_entry = json_keypath(&schema, reference, "/")?;
    };

    if let Some(possible_values) = config_entry.get("enum") {
        return Some(
            possible_values
                .as_array()?
                .iter()
                .filter_map(|val| val.as_str())
                .map(ToOwned::to_owned)
                .collect(),
        );
    }

    Some(match config_entry.get("type")?.as_str()? {
        "boolean" => vec!["false".into(), "true".into()],
        _ => vec![],
    })
}

fn config_keys_impl(only_leaves: bool, suffix: &str) -> Vec<CompletionCandidate> {
    let schema: serde_json::Value = serde_json::from_str(CONFIG_SCHEMA).unwrap();
    let schema = schema.as_object().unwrap();
    let properties = schema["properties"].as_object().unwrap();

    let mut candidates = Vec::new();
    config_keys_rec(
        ConfigNamePathBuf::root(),
        properties,
        &mut candidates,
        only_leaves,
        suffix,
    );
    candidates
}

pub fn config_keys() -> Vec<CompletionCandidate> {
    config_keys_impl(false, "")
}

pub fn leaf_config_keys() -> Vec<CompletionCandidate> {
    config_keys_impl(true, "")
}

pub fn leaf_config_key_value(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };

    if let Some((key, current_val)) = current.split_once('=') {
        let Ok(key) = key.parse() else {
            return Vec::new();
        };
        let possible_values = config_values(&key).unwrap_or_default();

        possible_values
            .into_iter()
            .filter(|x| x.starts_with(current_val))
            .map(|x| CompletionCandidate::new(format!("{key}={x}")))
            .collect()
    } else {
        config_keys_impl(true, "=")
            .into_iter()
            .filter(|candidate| candidate.get_value().to_str().unwrap().starts_with(current))
            .collect()
    }
}

pub fn branch_name_equals_any_revision(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };

    let Some((branch_name, revision)) = current.split_once('=') else {
        // Don't complete branch names since we want to create a new branch
        return Vec::new();
    };
    revset_expression(revision.as_ref(), None)
        .into_iter()
        .map(|rev| rev.add_prefix(format!("{branch_name}=")))
        .collect()
}

fn path_completion_candidate_from(
    current_prefix: &str,
    normalized_prefix_path: &Path,
    path: &Path,
    mode: Option<clap::builder::StyledStr>,
) -> Option<CompletionCandidate> {
    let normalized_prefix = match normalized_prefix_path.to_str()? {
        "." => "", // `.` cannot be normalized further, but doesn't prefix `path`.
        normalized_prefix => normalized_prefix,
    };

    let path = slash_path(path);
    let mut remainder = path.to_str()?.strip_prefix(normalized_prefix)?;

    // Trailing slash might have been normalized away in which case we need to strip
    // the leading slash in the remainder away, or else the slash would appear
    // twice.
    if current_prefix.ends_with(std::path::is_separator) {
        remainder = remainder.strip_prefix('/').unwrap_or(remainder);
    }

    match remainder.split_inclusive('/').at_most_one() {
        // Completed component is the final component in `path`, so we're completing the file to
        // which `mode` refers.
        Ok(file_completion) => Some(
            CompletionCandidate::new(format!(
                "{current_prefix}{}",
                file_completion.unwrap_or_default()
            ))
            .help(mode),
        ),

        // Omit `mode` when completing only up to the next directory.
        Err(mut components) => Some(CompletionCandidate::new(format!(
            "{current_prefix}{}",
            components.next().unwrap()
        ))),
    }
}

fn current_prefix_to_fileset(current: &str) -> String {
    let cur_esc = globset::escape(current);
    let dir_pat = format!("{cur_esc}*/**");
    let path_pat = format!("{cur_esc}*");
    format!("glob:{dir_pat:?} | glob:{path_pat:?}")
}

fn all_files_from_rev(rev: String, current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };

    let normalized_prefix = normalize_path(Path::new(current));
    let normalized_prefix = slash_path(&normalized_prefix);

    with_jj(|jj, _| {
        let mut child = jj
            .build()
            .arg("file")
            .arg("list")
            .arg("--revision")
            .arg(rev)
            .arg("--template")
            .arg(r#"path.display() ++ "\n""#)
            .arg(current_prefix_to_fileset(current))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(user_error)?;
        let stdout = child.stdout.take().unwrap();

        Ok(std::io::BufReader::new(stdout)
            .lines()
            .take(1_000)
            .map_while(Result::ok)
            .filter_map(|path| {
                path_completion_candidate_from(current, &normalized_prefix, Path::new(&path), None)
            })
            .dedup() // directories may occur multiple times
            .collect())
    })
}

fn modified_files_from_rev_with_jj_cmd(
    rev: (String, Option<String>),
    mut cmd: std::process::Command,
    current: &std::ffi::OsStr,
) -> Result<Vec<CompletionCandidate>, CommandError> {
    let Some(current) = current.to_str() else {
        return Ok(Vec::new());
    };

    let normalized_prefix = normalize_path(Path::new(current));
    let normalized_prefix = slash_path(&normalized_prefix);

    // In case of a rename, one entry of `diff` results in two suggestions.
    let template = indoc! {r#"
        concat(
          status ++ ' ' ++ path.display() ++ "\n",
          if(status == 'renamed', 'renamed.source ' ++ source.path().display() ++ "\n"),
        )
    "#};
    cmd.arg("diff")
        .args(["--template", template])
        .arg(current_prefix_to_fileset(current));
    match rev {
        (rev, None) => cmd.arg("--revisions").arg(rev),
        (from, Some(to)) => cmd.arg("--from").arg(from).arg("--to").arg(to),
    };
    let output = cmd.output().map_err(user_error)?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut include_renames = false;
    let mut candidates: Vec<_> = stdout
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter_map(|(mode, path)| {
            let mode = match mode {
                "modified" => "Modified".into(),
                "removed" => "Deleted".into(),
                "added" => "Added".into(),
                "renamed" => "Renamed".into(),
                "renamed.source" => {
                    include_renames = true;
                    "Renamed".into()
                }
                "copied" => "Copied".into(),
                _ => format!("unknown mode: '{mode}'").into(),
            };
            path_completion_candidate_from(current, &normalized_prefix, Path::new(path), Some(mode))
        })
        .collect();

    if include_renames {
        candidates.sort_unstable_by(|a, b| Path::new(a.get_value()).cmp(Path::new(b.get_value())));
    }
    candidates.dedup();

    Ok(candidates)
}

fn modified_files_from_rev(
    rev: (String, Option<String>),
    current: &std::ffi::OsStr,
) -> Vec<CompletionCandidate> {
    with_jj(|jj, _| modified_files_from_rev_with_jj_cmd(rev, jj.build(), current))
}

fn conflicted_files_from_rev(rev: &str, current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };

    let normalized_prefix = normalize_path(Path::new(current));
    let normalized_prefix = slash_path(&normalized_prefix);

    with_jj(|jj, _| {
        let output = jj
            .build()
            .arg("resolve")
            .arg("--list")
            .arg("--revision")
            .arg(rev)
            .arg(current_prefix_to_fileset(current))
            .output()
            .map_err(user_error)?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        Ok(stdout
            .lines()
            .filter_map(|line| {
                let path = line
                    .split_whitespace()
                    .next()
                    .expect("resolve --list should contain whitespace after path");

                path_completion_candidate_from(current, &normalized_prefix, Path::new(path), None)
            })
            .dedup() // directories may occur multiple times
            .collect())
    })
}

pub fn modified_files(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    modified_files_from_rev(("@".into(), None), current)
}

pub fn all_revision_files(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    all_files_from_rev(parse::revision_or_wc(), current)
}

pub fn modified_revision_files(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    modified_files_from_rev((parse::revision_or_wc(), None), current)
}

pub fn modified_range_files(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    match parse::range() {
        Some((from, to)) => modified_files_from_rev((from, Some(to)), current),
        None => modified_files_from_rev(("@".into(), None), current),
    }
}

/// Completes files in `@` *or* the `--from` revision (not the diff between
/// `--from` and `@`)
pub fn modified_from_files(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    modified_files_from_rev((parse::from_or_wc(), None), current)
}

pub fn modified_revision_or_range_files(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    if let Some(rev) = parse::revision() {
        return modified_files_from_rev((rev, None), current);
    }
    modified_range_files(current)
}

pub fn modified_changes_in_or_range_files(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    if let Some(rev) = parse::changes_in() {
        return modified_files_from_rev((rev, None), current);
    }
    modified_range_files(current)
}

pub fn revision_conflicted_files(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    conflicted_files_from_rev(&parse::revision_or_wc(), current)
}

/// Specific function for completing file paths for `jj squash`
pub fn squash_revision_files(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let rev = parse::squash_revision().unwrap_or_else(|| "@".into());
    modified_files_from_rev((rev, None), current)
}

/// Specific function for completing file paths for `jj interdiff`
pub fn interdiff_files(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some((from, to)) = parse::range() else {
        return Vec::new();
    };
    // Complete all modified files in "from" and "to". This will also suggest
    // files that are the same in both, which is a false positive. This approach
    // is more lightweight than actually doing a temporary rebase here.
    with_jj(|jj, _| {
        let mut res = modified_files_from_rev_with_jj_cmd((from, None), jj.build(), current)?;
        res.extend(modified_files_from_rev_with_jj_cmd(
            (to, None),
            jj.build(),
            current,
        )?);
        Ok(res)
    })
}

/// Specific function for completing file paths for `jj log`
pub fn log_files(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let mut rev = parse::log_revisions().join(")|(");
    if rev.is_empty() {
        rev = "@".into();
    } else {
        rev = format!("latest(heads(({rev})))"); // limit to one
    };
    all_files_from_rev(rev, current)
}

/// Shell out to jj during dynamic completion generation
///
/// In case of errors, print them and early return an empty vector.
fn with_jj<F>(completion_fn: F) -> Vec<CompletionCandidate>
where
    F: FnOnce(JjBuilder, &UserSettings) -> Result<Vec<CompletionCandidate>, CommandError>,
{
    get_jj_command()
        .and_then(|(jj, settings)| completion_fn(jj, &settings))
        .unwrap_or_else(|e| {
            eprintln!("{}", e.error);
            Vec::new()
        })
}

/// Shell out to jj during dynamic completion generation
///
/// This is necessary because dynamic completion code needs to be aware of
/// global configuration like custom storage backends. Dynamic completion
/// code via clap_complete doesn't accept arguments, so they cannot be passed
/// that way. Another solution would've been to use global mutable state, to
/// give completion code access to custom backends. Shelling out was chosen as
/// the preferred method, because it's more maintainable and the performance
/// requirements of completions aren't very high.
fn get_jj_command() -> Result<(JjBuilder, UserSettings), CommandError> {
    let current_exe = std::env::current_exe().map_err(user_error)?;
    let mut cmd_args = Vec::<String>::new();

    // Snapshotting could make completions much slower in some situations
    // and be undesired by the user.
    cmd_args.push("--ignore-working-copy".into());
    cmd_args.push("--color=never".into());
    cmd_args.push("--no-pager".into());

    // Parse some of the global args we care about for passing along to the
    // child process. This shouldn't fail, since none of the global args are
    // required.
    let app = crate::commands::default_app();
    let mut raw_config = config_from_environment(default_config_layers());
    let ui = Ui::null();
    let cwd = std::env::current_dir()
        .and_then(dunce::canonicalize)
        .map_err(user_error)?;
    let mut config_env = ConfigEnv::from_environment();
    let maybe_cwd_workspace_loader = DefaultWorkspaceLoaderFactory.create(find_workspace_dir(&cwd));
    let _ = config_env.reload_user_config(&mut raw_config);
    if let Ok(loader) = &maybe_cwd_workspace_loader {
        config_env.reset_repo_path(loader.repo_path());
        let _ = config_env.reload_repo_config(&mut raw_config);
        config_env.reset_workspace_path(loader.workspace_root());
        let _ = config_env.reload_workspace_config(&mut raw_config);
    }
    let mut config = config_env.resolve_config(&raw_config)?;
    // skip 2 because of the clap_complete prelude: jj -- jj <actual args...>
    let args = std::env::args_os().skip(2);
    let args = expand_args(&ui, &app, args, &config)?;
    let arg_matches = app
        .clone()
        .disable_version_flag(true)
        .disable_help_flag(true)
        .ignore_errors(true)
        .try_get_matches_from(args)?;
    let args: GlobalArgs = GlobalArgs::from_arg_matches(&arg_matches)?;

    if let Some(repository) = args.repository {
        // Try to update repo-specific config on a best-effort basis.
        if let Ok(loader) = DefaultWorkspaceLoaderFactory.create(&cwd.join(&repository)) {
            config_env.reset_repo_path(loader.repo_path());
            let _ = config_env.reload_repo_config(&mut raw_config);
            config_env.reset_workspace_path(loader.workspace_root());
            let _ = config_env.reload_workspace_config(&mut raw_config);
            if let Ok(new_config) = config_env.resolve_config(&raw_config) {
                config = new_config;
            }
        }
        cmd_args.push("--repository".into());
        cmd_args.push(repository);
    }
    if let Some(at_operation) = args.at_operation {
        // We cannot assume that the value of at_operation is valid, because
        // the user may be requesting completions precisely for this invalid
        // operation ID. Additionally, the user may have mistyped the ID,
        // in which case adding the argument blindly would break all other
        // completions, even unrelated ones.
        //
        // To avoid this, we shell out to ourselves once with the argument
        // and check the exit code. There is some performance overhead to this,
        // but this code path is probably only executed in exceptional
        // situations.
        let mut canary_cmd = std::process::Command::new(&current_exe);
        canary_cmd.args(&cmd_args);
        canary_cmd.arg("--at-operation");
        canary_cmd.arg(&at_operation);
        canary_cmd.arg("debug");
        canary_cmd.arg("snapshot");

        match canary_cmd.output() {
            Ok(output) if output.status.success() => {
                // Operation ID is valid, add it to the completion command.
                cmd_args.push("--at-operation".into());
                cmd_args.push(at_operation);
            }
            _ => {} // Invalid operation ID, ignore.
        }
    }
    for (kind, value) in args.early_args.merged_config_args(&arg_matches) {
        let arg = match kind {
            ConfigArgKind::Item => format!("--config={value}"),
            ConfigArgKind::File => format!("--config-file={value}"),
        };
        cmd_args.push(arg);
    }

    let builder = JjBuilder {
        cmd: current_exe,
        args: cmd_args,
    };
    let settings = UserSettings::from_config(config)?;

    Ok((builder, settings))
}

/// A helper struct to allow completion functions to call jj multiple times with
/// different arguments.
struct JjBuilder {
    cmd: std::path::PathBuf,
    args: Vec<String>,
}

impl JjBuilder {
    fn build(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new(&self.cmd);
        cmd.args(&self.args);
        cmd
    }
}

/// Functions for parsing revisions and revision ranges from the command line.
/// Parsing is done on a best-effort basis and relies on the heuristic that
/// most command line flags are consistent across different subcommands.
///
/// In some cases, this parsing will be incorrect, but it's not worth the effort
/// to fix that. For example, if the user specifies any of the relevant flags
/// multiple times, the parsing will pick any of the available ones, while the
/// actual execution of the command would fail.
mod parse {
    pub(super) fn parse_flag(
        candidates: &[&str],
        mut args: impl Iterator<Item = String>,
    ) -> impl Iterator<Item = String> {
        std::iter::from_fn(move || {
            for arg in args.by_ref() {
                // -r REV syntax
                if candidates.contains(&arg.as_ref()) {
                    match args.next() {
                        Some(val) if !val.starts_with('-') => {
                            return Some(strip_shell_quotes(&val).into());
                        }
                        _ => return None,
                    }
                }

                // -r=REV syntax
                if let Some(value) = candidates.iter().find_map(|candidate| {
                    let rest = arg.strip_prefix(candidate)?;
                    match rest.strip_prefix('=') {
                        Some(value) => Some(value),

                        // -rREV syntax
                        None if candidate.len() == 2 => Some(rest),

                        None => None,
                    }
                }) {
                    return Some(strip_shell_quotes(value).into());
                };
            }
            None
        })
    }

    pub fn parse_revision_impl(args: impl Iterator<Item = String>) -> Option<String> {
        parse_flag(&["-r", "--revision"], args).next()
    }

    pub fn revision() -> Option<String> {
        parse_revision_impl(std::env::args())
    }

    pub fn parse_changes_in_impl(args: impl Iterator<Item = String>) -> Option<String> {
        parse_flag(&["-c", "--changes-in"], args).next()
    }

    pub fn changes_in() -> Option<String> {
        parse_changes_in_impl(std::env::args())
    }

    pub fn revision_or_wc() -> String {
        revision().unwrap_or_else(|| "@".into())
    }

    pub fn from_or_wc() -> String {
        parse_flag(&["-f", "--from"], std::env::args())
            .next()
            .unwrap_or_else(|| "@".into())
    }

    pub fn parse_range_impl<T>(args: impl Fn() -> T) -> Option<(String, String)>
    where
        T: Iterator<Item = String>,
    {
        let from = parse_flag(&["-f", "--from"], args()).next()?;
        let to = parse_flag(&["-t", "--to"], args())
            .next()
            .unwrap_or_else(|| "@".into());

        Some((from, to))
    }

    pub fn range() -> Option<(String, String)> {
        parse_range_impl(std::env::args)
    }

    // Special parse function only for `jj squash`. While squash has --from and
    // --to arguments, only files within --from should be completed, because
    // the files changed only in some other revision in the range between
    // --from and --to cannot be squashed into --to like that.
    pub fn squash_revision() -> Option<String> {
        if let Some(rev) = parse_flag(&["-r", "--revision"], std::env::args()).next() {
            return Some(rev);
        }
        parse_flag(&["-f", "--from"], std::env::args()).next()
    }

    // Special parse function only for `jj log`. It has a --revisions flag,
    // instead of the usual --revision, and it can be supplied multiple times.
    pub fn log_revisions() -> Vec<String> {
        let candidates = &["-r", "--revisions"];
        parse_flag(candidates, std::env::args()).collect()
    }

    fn strip_shell_quotes(s: &str) -> &str {
        if s.len() >= 2
            && (s.starts_with('"') && s.ends_with('"') || s.starts_with('\'') && s.ends_with('\''))
        {
            &s[1..s.len() - 1]
        } else {
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_revset_trailing_name() {
        assert_eq!(split_revset_trailing_name(""), Some(("", "")));
        assert_eq!(split_revset_trailing_name(" "), Some((" ", "")));
        assert_eq!(split_revset_trailing_name("foo"), Some(("", "foo")));
        assert_eq!(split_revset_trailing_name(" foo"), Some((" ", "foo")));
        assert_eq!(split_revset_trailing_name("foo "), None);
        assert_eq!(split_revset_trailing_name("foo_"), Some(("", "foo_")));
        assert_eq!(split_revset_trailing_name("foo/"), Some(("", "foo/")));
        assert_eq!(split_revset_trailing_name("foo/b"), Some(("", "foo/b")));

        assert_eq!(split_revset_trailing_name("foo-"), Some(("", "foo-")));
        assert_eq!(split_revset_trailing_name("foo+"), Some(("", "foo+")));
        assert_eq!(
            split_revset_trailing_name("foo-bar-"),
            Some(("", "foo-bar-"))
        );
        assert_eq!(
            split_revset_trailing_name("foo-bar-b"),
            Some(("", "foo-bar-b"))
        );

        assert_eq!(split_revset_trailing_name("foo."), Some(("", "foo.")));
        assert_eq!(split_revset_trailing_name("foo..b"), Some(("foo..", "b")));
        assert_eq!(split_revset_trailing_name("..foo"), Some(("..", "foo")));

        assert_eq!(split_revset_trailing_name("foo(bar"), Some(("foo(", "bar")));
        assert_eq!(split_revset_trailing_name("foo(bar)"), None);
        assert_eq!(split_revset_trailing_name("(f"), Some(("(", "f")));

        assert_eq!(split_revset_trailing_name("foo@"), Some(("", "foo@")));
        assert_eq!(split_revset_trailing_name("foo@b"), Some(("", "foo@b")));
        assert_eq!(split_revset_trailing_name("..foo@"), Some(("..", "foo@")));
        assert_eq!(
            split_revset_trailing_name("::F(foo@origin.1..bar@origin."),
            Some(("::F(foo@origin.1..", "bar@origin."))
        );
    }

    #[test]
    fn test_split_revset_trailing_name_with_trailing_operator() {
        assert_eq!(split_revset_trailing_name("foo|"), Some(("foo|", "")));
        assert_eq!(split_revset_trailing_name("foo | "), Some(("foo | ", "")));
        assert_eq!(split_revset_trailing_name("foo&"), Some(("foo&", "")));
        assert_eq!(split_revset_trailing_name("foo~"), Some(("foo~", "")));

        assert_eq!(split_revset_trailing_name(".."), Some(("..", "")));
        assert_eq!(split_revset_trailing_name("foo.."), Some(("foo..", "")));
        assert_eq!(split_revset_trailing_name("::"), Some(("::", "")));
        assert_eq!(split_revset_trailing_name("foo::"), Some(("foo::", "")));

        assert_eq!(split_revset_trailing_name("("), Some(("(", "")));
        assert_eq!(split_revset_trailing_name("foo("), Some(("foo(", "")));
        assert_eq!(split_revset_trailing_name("foo()"), None);
        assert_eq!(split_revset_trailing_name("foo(bar)"), None);
    }

    #[test]
    fn test_split_revset_trailing_name_with_modifier() {
        assert_eq!(split_revset_trailing_name("all:"), Some(("all:", "")));
        assert_eq!(split_revset_trailing_name("all: "), Some(("all: ", "")));
        assert_eq!(split_revset_trailing_name("all:f"), Some(("all:", "f")));
        assert_eq!(split_revset_trailing_name("all: f"), Some(("all: ", "f")));
    }

    #[test]
    fn test_config_keys() {
        // Just make sure the schema is parsed without failure.
        let _ = config_keys();
    }

    #[test]
    fn test_parse_revision_impl() {
        let good_cases: &[&[&str]] = &[
            &["-r", "foo"],
            &["-r", "'foo'"],
            &["-r", "\"foo\""],
            &["-rfoo"],
            &["-r'foo'"],
            &["-r\"foo\""],
            &["--revision", "foo"],
            &["-r=foo"],
            &["-r='foo'"],
            &["-r=\"foo\""],
            &["--revision=foo"],
            &["--revision='foo'"],
            &["--revision=\"foo\""],
            &["preceding_arg", "-r", "foo"],
            &["-r", "foo", "following_arg"],
        ];
        for case in good_cases {
            let args = case.iter().map(|s| s.to_string());
            assert_eq!(
                parse::parse_revision_impl(args),
                Some("foo".into()),
                "case: {case:?}",
            );
        }
        let bad_cases: &[&[&str]] = &[&[], &["-r"], &["foo"], &["-R", "foo"], &["-R=foo"]];
        for case in bad_cases {
            let args = case.iter().map(|s| s.to_string());
            assert_eq!(parse::parse_revision_impl(args), None, "case: {case:?}");
        }
    }

    #[test]
    fn test_parse_changes_in_impl() {
        let good_cases: &[&[&str]] = &[
            &["-c", "foo"],
            &["--changes-in", "foo"],
            &["-cfoo"],
            &["--changes-in=foo"],
        ];
        for case in good_cases {
            let args = case.iter().map(|s| s.to_string());
            assert_eq!(
                parse::parse_changes_in_impl(args),
                Some("foo".into()),
                "case: {case:?}",
            );
        }
        let bad_cases: &[&[&str]] = &[&[], &["-c"], &["-r"], &["foo"]];
        for case in bad_cases {
            let args = case.iter().map(|s| s.to_string());
            assert_eq!(parse::parse_revision_impl(args), None, "case: {case:?}");
        }
    }

    #[test]
    fn test_parse_range_impl() {
        let wc_cases: &[&[&str]] = &[
            &["-f", "foo"],
            &["--from", "foo"],
            &["-f=foo"],
            &["preceding_arg", "-f", "foo"],
            &["-f", "foo", "following_arg"],
        ];
        for case in wc_cases {
            let args = case.iter().map(|s| s.to_string());
            assert_eq!(
                parse::parse_range_impl(|| args.clone()),
                Some(("foo".into(), "@".into())),
                "case: {case:?}",
            );
        }
        let to_cases: &[&[&str]] = &[
            &["-f", "foo", "-t", "bar"],
            &["-f", "foo", "--to", "bar"],
            &["-f=foo", "-t=bar"],
            &["-t=bar", "-f=foo"],
        ];
        for case in to_cases {
            let args = case.iter().map(|s| s.to_string());
            assert_eq!(
                parse::parse_range_impl(|| args.clone()),
                Some(("foo".into(), "bar".into())),
                "case: {case:?}",
            );
        }
        let bad_cases: &[&[&str]] = &[&[], &["-f"], &["foo"], &["-R", "foo"], &["-R=foo"]];
        for case in bad_cases {
            let args = case.iter().map(|s| s.to_string());
            assert_eq!(
                parse::parse_range_impl(|| args.clone()),
                None,
                "case: {case:?}"
            );
        }
    }

    #[test]
    fn test_parse_multiple_flags() {
        let candidates = &["-r", "--revisions"];
        let args = &[
            "unrelated_arg_at_the_beginning",
            "-r",
            "1",
            "--revisions",
            "2",
            "-r=3",
            "--revisions=4",
            "unrelated_arg_in_the_middle",
            "-r5",
            "unrelated_arg_at_the_end",
        ];
        let flags: Vec<_> =
            parse::parse_flag(candidates, args.iter().map(|a| a.to_string())).collect();
        let expected = ["1", "2", "3", "4", "5"];
        assert_eq!(flags, expected);
    }
}
