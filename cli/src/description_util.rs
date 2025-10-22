use std::collections::HashMap;
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitStatus;

use bstr::ByteVec as _;
use indexmap::IndexMap;
use indoc::indoc;
use itertools::FoldWhile;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::commit_builder::DetachedCommitBuilder;
use jj_lib::config::ConfigGetError;
use jj_lib::file_util::IoResultExt as _;
use jj_lib::file_util::PathError;
use jj_lib::settings::UserSettings;
use jj_lib::trailer::parse_description_trailers;
use jj_lib::trailer::parse_trailers;
use pollster::FutureExt as _;
use thiserror::Error;

use crate::cli_util::WorkspaceCommandTransaction;
use crate::cli_util::short_commit_hash;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::config::CommandNameAndArgs;
use crate::formatter::PlainTextFormatter;
use crate::templater::TemplateRenderer;
use crate::text_util;
use crate::ui::Ui;

#[derive(Debug, Error)]
pub enum TextEditError {
    #[error("Failed to run editor '{name}'")]
    FailedToRun { name: String, source: io::Error },
    #[error("Editor '{command}' exited with {status}")]
    ExitStatus { command: String, status: ExitStatus },
}

#[derive(Debug, Error)]
#[error("Failed to edit {name}", name = name.as_deref().unwrap_or("file"))]
pub struct TempTextEditError {
    #[source]
    pub error: Box<dyn std::error::Error + Send + Sync>,
    /// Short description of the edited content.
    pub name: Option<String>,
    /// Path to the temporary file.
    pub path: Option<PathBuf>,
}

impl TempTextEditError {
    fn new(error: Box<dyn std::error::Error + Send + Sync>, path: Option<PathBuf>) -> Self {
        Self {
            error,
            name: None,
            path,
        }
    }

    /// Adds short description of the edited content.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// Configured text editor.
#[derive(Clone, Debug)]
pub struct TextEditor {
    editor: CommandNameAndArgs,
    dir: Option<PathBuf>,
}

impl TextEditor {
    pub fn from_settings(settings: &UserSettings) -> Result<Self, ConfigGetError> {
        let editor = settings.get("ui.editor")?;
        Ok(Self { editor, dir: None })
    }

    pub fn with_temp_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// Opens the given `path` in editor.
    pub fn edit_file(&self, path: impl AsRef<Path>) -> Result<(), TextEditError> {
        let mut cmd = self.editor.to_command();
        cmd.arg(path.as_ref());
        tracing::info!(?cmd, "running editor");
        let status = cmd.status().map_err(|source| TextEditError::FailedToRun {
            name: self.editor.split_name().into_owned(),
            source,
        })?;
        if status.success() {
            Ok(())
        } else {
            let command = self.editor.to_string();
            Err(TextEditError::ExitStatus { command, status })
        }
    }

    /// Writes the given `content` to temporary file and opens it in editor.
    pub fn edit_str(
        &self,
        content: impl AsRef<[u8]>,
        suffix: Option<&str>,
    ) -> Result<String, TempTextEditError> {
        let path = self
            .write_temp_file(content.as_ref(), suffix)
            .map_err(|err| TempTextEditError::new(err.into(), None))?;
        self.edit_file(&path)
            .map_err(|err| TempTextEditError::new(err.into(), Some(path.clone())))?;
        let edited = fs::read_to_string(&path)
            .context(&path)
            .map_err(|err| TempTextEditError::new(err.into(), Some(path.clone())))?;
        // Delete the file only if everything went well.
        fs::remove_file(path).ok();
        Ok(edited)
    }

    fn write_temp_file(&self, content: &[u8], suffix: Option<&str>) -> Result<PathBuf, PathError> {
        let dir = self.dir.clone().unwrap_or_else(tempfile::env::temp_dir);
        let mut file = tempfile::Builder::new()
            .prefix("editor-")
            .suffix(suffix.unwrap_or(""))
            .tempfile_in(&dir)
            .context(&dir)?;
        file.write_all(content).context(file.path())?;
        let (_, path) = file
            .keep()
            .or_else(|err| Err(err.error).context(err.file.path()))?;
        Ok(path)
    }
}

fn append_blank_line(text: &mut String) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    let last_line = text.lines().next_back();
    if last_line.is_some_and(|line| line.starts_with("JJ:")) {
        text.push_str("JJ:\n");
    } else {
        text.push('\n');
    }
}

/// Cleanup a description by normalizing line endings, and removing leading and
/// trailing blank lines.
fn cleanup_description_lines<I>(lines: I) -> String
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let description = lines
        .into_iter()
        .fold_while(String::new(), |acc, line| {
            let line = line.as_ref();
            if line.strip_prefix("JJ: ignore-rest").is_some() {
                FoldWhile::Done(acc)
            } else if line.starts_with("JJ:") {
                FoldWhile::Continue(acc)
            } else {
                FoldWhile::Continue(acc + line + "\n")
            }
        })
        .into_inner();
    text_util::complete_newline(description.trim_matches('\n'))
}

pub fn edit_description(editor: &TextEditor, description: &str) -> Result<String, CommandError> {
    let mut description = description.to_owned();
    append_blank_line(&mut description);
    description.push_str("JJ: Lines starting with \"JJ:\" (like this one) will be removed.\n");

    let description = editor
        .edit_str(description, Some(".jjdescription"))
        .map_err(|err| err.with_name("description"))?;

    Ok(cleanup_description_lines(description.lines()))
}

/// Edits the descriptions of the given commits in a single editor session.
pub fn edit_multiple_descriptions(
    ui: &Ui,
    editor: &TextEditor,
    tx: &WorkspaceCommandTransaction,
    commits: &[(&CommitId, Commit)],
) -> Result<ParsedBulkEditMessage<CommitId>, CommandError> {
    let mut commits_map = IndexMap::new();
    let mut bulk_message = String::new();

    bulk_message.push_str(indoc! {r#"
        JJ: Enter or edit commit descriptions after the `JJ: describe` lines.
        JJ: Warning:
        JJ: - The text you enter will be lost on a syntax error.
        JJ: - The syntax of the separator lines may change in the future.
        JJ:
    "#});
    for (commit_id, temp_commit) in commits {
        let commit_hash = short_commit_hash(commit_id);
        bulk_message.push_str("JJ: describe ");
        bulk_message.push_str(&commit_hash);
        bulk_message.push_str(" -------\n");
        commits_map.insert(commit_hash, *commit_id);
        let intro = "";
        let template = description_template(ui, tx, intro, temp_commit)?;
        bulk_message.push_str(&template);
        append_blank_line(&mut bulk_message);
    }
    bulk_message.push_str("JJ: Lines starting with \"JJ:\" (like this one) will be removed.\n");

    let bulk_message = editor
        .edit_str(bulk_message, Some(".jjdescription"))
        .map_err(|err| err.with_name("description"))?;

    Ok(parse_bulk_edit_message(&bulk_message, &commits_map)?)
}

#[derive(Debug)]
pub struct ParsedBulkEditMessage<T> {
    /// The parsed, formatted descriptions.
    pub descriptions: HashMap<T, String>,
    /// Commit IDs that were expected while parsing the edited messages, but
    /// which were not found.
    pub missing: Vec<String>,
    /// Commit IDs that were found multiple times while parsing the edited
    /// messages.
    pub duplicates: Vec<String>,
    /// Commit IDs that were found while parsing the edited messages, but which
    /// were not originally being edited.
    pub unexpected: Vec<String>,
}

#[derive(Debug, Error, PartialEq)]
pub enum ParseBulkEditMessageError {
    #[error(r#"Found the following line without a commit header: "{0}""#)]
    LineWithoutCommitHeader(String),
}

/// Parse the bulk message of edited commit descriptions.
fn parse_bulk_edit_message<T>(
    message: &str,
    commit_ids_map: &IndexMap<String, &T>,
) -> Result<ParsedBulkEditMessage<T>, ParseBulkEditMessageError>
where
    T: Eq + std::hash::Hash + Clone,
{
    let mut descriptions = HashMap::new();
    let mut duplicates = Vec::new();
    let mut unexpected = Vec::new();

    let mut messages: Vec<(&str, Vec<&str>)> = vec![];
    for line in message.lines() {
        if let Some(commit_id_prefix) = line.strip_prefix("JJ: describe ") {
            let commit_id_prefix =
                commit_id_prefix.trim_end_matches(|c: char| c.is_ascii_whitespace() || c == '-');
            messages.push((commit_id_prefix, vec![]));
        } else if let Some((_, lines)) = messages.last_mut() {
            lines.push(line);
        }
        // Do not allow lines without a commit header, except for empty lines or comments.
        else if !line.trim().is_empty() && !line.starts_with("JJ:") {
            return Err(ParseBulkEditMessageError::LineWithoutCommitHeader(
                line.to_owned(),
            ));
        };
    }

    for (commit_id_prefix, description_lines) in messages {
        let Some(&commit_id) = commit_ids_map.get(commit_id_prefix) else {
            unexpected.push(commit_id_prefix.to_string());
            continue;
        };
        if descriptions.contains_key(commit_id) {
            duplicates.push(commit_id_prefix.to_string());
            continue;
        }
        descriptions.insert(
            commit_id.clone(),
            cleanup_description_lines(&description_lines),
        );
    }

    let missing: Vec<_> = commit_ids_map
        .iter()
        .filter(|(_, commit_id)| !descriptions.contains_key(*commit_id))
        .map(|(commit_id_prefix, _)| commit_id_prefix.clone())
        .collect();

    Ok(ParsedBulkEditMessage {
        descriptions,
        missing,
        duplicates,
        unexpected,
    })
}

/// Combines the descriptions from the input commits. If only one is non-empty,
/// then that one is used.
pub fn try_combine_messages(sources: &[Commit], destination: &Commit) -> Option<String> {
    let non_empty = sources
        .iter()
        .chain(std::iter::once(destination))
        .filter(|c| !c.description().is_empty())
        .take(2)
        .collect_vec();
    match *non_empty.as_slice() {
        [] => Some(String::new()),
        [commit] => Some(commit.description().to_owned()),
        [_, _, ..] => None,
    }
}

/// Produces a combined description with "JJ: " comment lines.
///
/// This includes empty descriptins too, so the user doesn't have to wonder why
/// they only see 2 descriptions when they combined 3 commits.
pub fn combine_messages_for_editing(
    ui: &Ui,
    tx: &WorkspaceCommandTransaction,
    sources: &[Commit],
    destination: Option<&Commit>,
    commit_builder: &DetachedCommitBuilder,
) -> Result<String, CommandError> {
    let mut combined = String::new();
    if let Some(destination) = destination {
        combined.push_str("JJ: Description from the destination commit:\n");
        combined.push_str(destination.description());
    }
    for commit in sources {
        combined.push_str("\nJJ: Description from source commit:\n");
        combined.push_str(commit.description());
    }

    if let Some(template) = parse_trailers_template(ui, tx)? {
        // show the user only trailers that were not in one of the squashed commits
        let old_trailers: Vec<_> = sources
            .iter()
            .chain(destination)
            .flat_map(|commit| parse_description_trailers(commit.description()))
            .collect();
        let commit = commit_builder.write_hidden().block_on()?;
        let trailer_lines = template
            .format_plain_text(&commit)
            .into_string()
            .map_err(|_| user_error("Trailers should be valid utf-8"))?;
        let new_trailers = parse_trailers(&trailer_lines)?;
        let trailers: String = new_trailers
            .iter()
            .filter(|trailer| !old_trailers.contains(trailer))
            .map(|trailer| format!("{}: {}\n", trailer.key, trailer.value))
            .collect();
        if !trailers.is_empty() {
            combined.push_str("\nJJ: Trailers not found in the squashed commits:\n");
            combined.push_str(&trailers);
        }
    }

    Ok(combined)
}

/// Create a description from a list of paragraphs.
///
/// Based on the Git CLI behavior. See `opt_parse_m()` and `cleanup_mode` in
/// `git/builtin/commit.c`.
pub fn join_message_paragraphs(paragraphs: &[String]) -> String {
    // Ensure each paragraph ends with a newline, then add another newline between
    // paragraphs.
    paragraphs
        .iter()
        .map(|p| text_util::complete_newline(p.as_str()))
        .join("\n")
}

/// Parse the commit trailers template from the configuration
///
/// Returns None if the commit trailers template is empty.
pub fn parse_trailers_template<'a>(
    ui: &Ui,
    tx: &'a WorkspaceCommandTransaction,
) -> Result<Option<TemplateRenderer<'a, Commit>>, CommandError> {
    let trailer_template = tx.settings().get_string("templates.commit_trailers")?;
    if trailer_template.is_empty() {
        Ok(None)
    } else {
        tx.parse_commit_template(ui, &trailer_template).map(Some)
    }
}

/// Add the trailers from the given `template` in the last paragraph of
/// the description
///
/// It just lets the description untouched if the trailers are already there.
pub fn add_trailers_with_template(
    template: &TemplateRenderer<'_, Commit>,
    commit: &Commit,
) -> Result<String, CommandError> {
    let trailers = parse_description_trailers(commit.description());
    let trailer_lines = template
        .format_plain_text(commit)
        .into_string()
        .map_err(|_| user_error("Trailers should be valid utf-8"))?;
    let new_trailers = parse_trailers(&trailer_lines)?;
    let mut description = commit.description().to_owned();
    if trailers.is_empty() && !new_trailers.is_empty() {
        if description.is_empty() {
            // a first empty line where the user will edit the commit summary
            description.push('\n');
        }
        // create a new paragraph for the trailer
        description.push('\n');
    }
    for new_trailer in new_trailers {
        if !trailers.contains(&new_trailer) {
            description.push_str(&format!("{}: {}\n", new_trailer.key, new_trailer.value));
        }
    }
    Ok(description)
}

/// Add the trailers from `templates.commit_trailers` in the last paragraph of
/// the description
///
/// It just lets the description untouched if the trailers are already there.
pub fn add_trailers(
    ui: &Ui,
    tx: &WorkspaceCommandTransaction,
    commit_builder: &DetachedCommitBuilder,
) -> Result<String, CommandError> {
    if let Some(renderer) = parse_trailers_template(ui, tx)? {
        let commit = commit_builder.write_hidden().block_on()?;
        add_trailers_with_template(&renderer, &commit)
    } else {
        Ok(commit_builder.description().to_owned())
    }
}

/// Renders commit description template, which will be edited by user.
pub fn description_template(
    ui: &Ui,
    tx: &WorkspaceCommandTransaction,
    intro: &str,
    commit: &Commit,
) -> Result<String, CommandError> {
    // Named as "draft" because the output can contain "JJ:" comment lines.
    let template_key = "templates.draft_commit_description";
    let template_text = tx.settings().get_string(template_key)?;
    let template = tx.parse_commit_template(ui, &template_text)?;

    let mut output = Vec::new();
    if !intro.is_empty() {
        writeln!(output, "JJ: {intro}").unwrap();
    }
    template
        .format(commit, &mut PlainTextFormatter::new(&mut output))
        .expect("write() to vec backed formatter should never fail");
    // Template output is usually UTF-8, but it can contain file content.
    Ok(output.into_string_lossy())
}

#[cfg(test)]
mod tests {
    use indexmap::indexmap;
    use indoc::indoc;
    use maplit::hashmap;

    use super::parse_bulk_edit_message;
    use crate::description_util::ParseBulkEditMessageError;

    #[test]
    fn test_parse_complete_bulk_edit_message() {
        let result = parse_bulk_edit_message(
            indoc! {"
                JJ: describe 1 -------
                Description 1

                JJ: describe 2
                Description 2

                JJ: describe 3 --
                Description 3
            "},
            &indexmap! {
                "1".to_string() => &1,
                "2".to_string() => &2,
                "3".to_string() => &3,
            },
        )
        .unwrap();
        assert_eq!(
            result.descriptions,
            hashmap! {
                1 => "Description 1\n".to_string(),
                2 => "Description 2\n".to_string(),
                3 => "Description 3\n".to_string(),
            }
        );
        assert!(result.missing.is_empty());
        assert!(result.duplicates.is_empty());
        assert!(result.unexpected.is_empty());
    }

    #[test]
    fn test_parse_bulk_edit_message_with_missing_descriptions() {
        let result = parse_bulk_edit_message(
            indoc! {"
                JJ: describe 1 -------
                Description 1
            "},
            &indexmap! {
                "1".to_string() => &1,
                "2".to_string() => &2,
            },
        )
        .unwrap();
        assert_eq!(
            result.descriptions,
            hashmap! {
                1 => "Description 1\n".to_string(),
            }
        );
        assert_eq!(result.missing, vec!["2".to_string()]);
        assert!(result.duplicates.is_empty());
        assert!(result.unexpected.is_empty());
    }

    #[test]
    fn test_parse_bulk_edit_message_with_duplicate_descriptions() {
        let result = parse_bulk_edit_message(
            indoc! {"
                JJ: describe 1 -------
                Description 1

                JJ: describe 1 -------
                Description 1 (repeated)
            "},
            &indexmap! {
                "1".to_string() => &1,
            },
        )
        .unwrap();
        assert_eq!(
            result.descriptions,
            hashmap! {
                1 => "Description 1\n".to_string(),
            }
        );
        assert!(result.missing.is_empty());
        assert_eq!(result.duplicates, vec!["1".to_string()]);
        assert!(result.unexpected.is_empty());
    }

    #[test]
    fn test_parse_bulk_edit_message_with_unexpected_descriptions() {
        let result = parse_bulk_edit_message(
            indoc! {"
                JJ: describe 1 -------
                Description 1

                JJ: describe 3 -------
                Description 3 (unexpected)
            "},
            &indexmap! {
                "1".to_string() => &1,
            },
        )
        .unwrap();
        assert_eq!(
            result.descriptions,
            hashmap! {
                1 => "Description 1\n".to_string(),
            }
        );
        assert!(result.missing.is_empty());
        assert!(result.duplicates.is_empty());
        assert_eq!(result.unexpected, vec!["3".to_string()]);
    }

    #[test]
    fn test_parse_bulk_edit_message_with_no_header() {
        let result = parse_bulk_edit_message(
            indoc! {"
                Description 1
            "},
            &indexmap! {
                "1".to_string() => &1,
            },
        );
        assert_eq!(
            result.unwrap_err(),
            ParseBulkEditMessageError::LineWithoutCommitHeader("Description 1".to_string())
        );
    }

    #[test]
    fn test_parse_bulk_edit_message_with_comment_before_header() {
        let result = parse_bulk_edit_message(
            indoc! {"
                JJ: Custom comment and empty lines below should be accepted


                JJ: describe 1 -------
                Description 1
            "},
            &indexmap! {
                "1".to_string() => &1,
            },
        )
        .unwrap();
        assert_eq!(
            result.descriptions,
            hashmap! {
                1 => "Description 1\n".to_string(),
            }
        );
        assert!(result.missing.is_empty());
        assert!(result.duplicates.is_empty());
        assert!(result.unexpected.is_empty());
    }
}
