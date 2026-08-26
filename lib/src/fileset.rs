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

//! Functional language for selecting a set of paths.

use std::collections::HashMap;
use std::path;
use std::sync::LazyLock;

use itertools::Itertools as _;
pub use jj_core::fileset_backend::FilePattern;
pub use jj_core::fileset_backend::FilesetExpression;
use thiserror::Error;

use crate::dsl_util::collect_similar;
use crate::fileset_parser;
use crate::fileset_parser::BinaryOp;
use crate::fileset_parser::ExpressionKind;
use crate::fileset_parser::ExpressionNode;
pub use crate::fileset_parser::FilesetAliasesMap;
pub use crate::fileset_parser::FilesetDiagnostics;
pub use crate::fileset_parser::FilesetParseError;
pub use crate::fileset_parser::FilesetParseErrorKind;
pub use crate::fileset_parser::FilesetParseResult;
use crate::fileset_parser::FunctionCallNode;
use crate::fileset_parser::UnaryOp;
use crate::matchers::PathGlobPattern;
use crate::repo_path::RelativePathParseError;
use crate::repo_path::RepoPathBuf;
use crate::ui_path::RepoPathUiConverter;
use crate::ui_path::UiPathParseError;

/// Error occurred during file pattern parsing.
#[derive(Debug, Error)]
pub enum FilePatternParseError {
    /// Unknown pattern kind is specified.
    #[error("Invalid file pattern kind `{0}:`")]
    InvalidKind(String),
    /// Failed to parse input UI path.
    #[error(transparent)]
    UiPath(#[from] UiPathParseError),
    /// Failed to parse input workspace-relative path.
    #[error(transparent)]
    RelativePath(#[from] RelativePathParseError),
    /// Failed to parse glob pattern.
    #[error(transparent)]
    GlobPattern(#[from] globset::Error),
}

/// Parses the given `input` string as pattern of the specified `kind`.
fn parse_pattern_kind(
    path_converter: &RepoPathUiConverter,
    input: &str,
    kind: &str,
) -> Result<FilePattern, FilePatternParseError> {
    // Naming convention:
    // * path normalization
    //   * cwd: cwd-relative path (default)
    //   * root: workspace-relative path
    // * where to anchor
    //   * file: exact file path
    //   * prefix: path prefix (files under directory recursively)
    //   * files-in: files in directory non-recursively
    //   * name: file name component (or suffix match?)
    //   * substring: substring match?
    // * string pattern syntax (+ case sensitivity?)
    //   * path: literal path (default) (default anchor: prefix)
    //   * glob: glob pattern (default anchor: file)
    //   * regex?
    match kind {
        "cwd" => parse_cwd_prefix_path(path_converter, input),
        "cwd-file" | "file" => parse_cwd_file_path(path_converter, input),
        "cwd-glob" | "glob" => parse_cwd_file_glob(path_converter, input),
        "cwd-glob-i" | "glob-i" => parse_cwd_file_glob_i(path_converter, input),
        "cwd-prefix-glob" | "prefix-glob" => parse_cwd_prefix_glob(path_converter, input),
        "cwd-prefix-glob-i" | "prefix-glob-i" => parse_cwd_prefix_glob_i(path_converter, input),
        "root" => parse_root_prefix_path(input),
        "root-file" => parse_root_file_path(input),
        "root-glob" => parse_root_file_glob(input),
        "root-glob-i" => parse_root_file_glob_i(input),
        "root-prefix-glob" => parse_root_prefix_glob(input),
        "root-prefix-glob-i" => parse_root_prefix_glob_i(input),
        _ => Err(FilePatternParseError::InvalidKind(kind.to_owned())),
    }
}

/// Parses pattern that matches cwd-relative file (or exact) path.
fn parse_cwd_file_path(
    path_converter: &RepoPathUiConverter,
    input: impl AsRef<str>,
) -> Result<FilePattern, FilePatternParseError> {
    let path = path_converter.parse_file_path(input.as_ref())?;
    Ok(FilePattern::FilePath(path))
}

/// Parses pattern that matches cwd-relative path prefix.
fn parse_cwd_prefix_path(
    path_converter: &RepoPathUiConverter,
    input: impl AsRef<str>,
) -> Result<FilePattern, FilePatternParseError> {
    let path = path_converter.parse_file_path(input.as_ref())?;
    Ok(FilePattern::PrefixPath(path))
}

/// Parses pattern that matches cwd-relative file path glob.
fn parse_cwd_file_glob(
    path_converter: &RepoPathUiConverter,
    input: impl AsRef<str>,
) -> Result<FilePattern, FilePatternParseError> {
    let (dir, pattern) = split_glob_path(input.as_ref());
    let dir = path_converter.parse_file_path(dir)?;
    file_glob_at(dir, pattern, false)
}

/// Parses pattern that matches cwd-relative file path glob
/// (case-insensitive).
fn parse_cwd_file_glob_i(
    path_converter: &RepoPathUiConverter,
    input: impl AsRef<str>,
) -> Result<FilePattern, FilePatternParseError> {
    let (dir, pattern) = split_glob_path_i(input.as_ref());
    let dir = path_converter.parse_file_path(dir)?;
    file_glob_at(dir, pattern, true)
}

/// Parses pattern that matches cwd-relative path prefix by glob.
fn parse_cwd_prefix_glob(
    path_converter: &RepoPathUiConverter,
    input: impl AsRef<str>,
) -> Result<FilePattern, FilePatternParseError> {
    let (dir, pattern) = split_glob_path(input.as_ref());
    let dir = path_converter.parse_file_path(dir)?;
    prefix_glob_at(dir, pattern, false)
}

/// Parses pattern that matches cwd-relative path prefix by glob
/// (case-insensitive).
fn parse_cwd_prefix_glob_i(
    path_converter: &RepoPathUiConverter,
    input: impl AsRef<str>,
) -> Result<FilePattern, FilePatternParseError> {
    let (dir, pattern) = split_glob_path_i(input.as_ref());
    let dir = path_converter.parse_file_path(dir)?;
    prefix_glob_at(dir, pattern, true)
}

/// Parses pattern that matches workspace-relative file (or exact) path.
fn parse_root_file_path(input: impl AsRef<str>) -> Result<FilePattern, FilePatternParseError> {
    // TODO: Let caller pass in converter for root-relative paths too
    let path = RepoPathBuf::from_relative_path(input.as_ref())?;
    Ok(FilePattern::FilePath(path))
}

/// Parses pattern that matches workspace-relative path prefix.
fn parse_root_prefix_path(input: impl AsRef<str>) -> Result<FilePattern, FilePatternParseError> {
    let path = RepoPathBuf::from_relative_path(input.as_ref())?;
    Ok(FilePattern::PrefixPath(path))
}

/// Parses pattern that matches workspace-relative file path glob.
fn parse_root_file_glob(input: impl AsRef<str>) -> Result<FilePattern, FilePatternParseError> {
    let (dir, pattern) = split_glob_path(input.as_ref());
    let dir = RepoPathBuf::from_relative_path(dir)?;
    file_glob_at(dir, pattern, false)
}

/// Parses pattern that matches workspace-relative file path glob
/// (case-insensitive).
fn parse_root_file_glob_i(input: impl AsRef<str>) -> Result<FilePattern, FilePatternParseError> {
    let (dir, pattern) = split_glob_path_i(input.as_ref());
    let dir = RepoPathBuf::from_relative_path(dir)?;
    file_glob_at(dir, pattern, true)
}

/// Parses pattern that matches workspace-relative path prefix by glob.
fn parse_root_prefix_glob(input: impl AsRef<str>) -> Result<FilePattern, FilePatternParseError> {
    let (dir, pattern) = split_glob_path(input.as_ref());
    let dir = RepoPathBuf::from_relative_path(dir)?;
    prefix_glob_at(dir, pattern, false)
}

/// Parses pattern that matches workspace-relative path prefix by glob
/// (case-insensitive).
fn parse_root_prefix_glob_i(input: impl AsRef<str>) -> Result<FilePattern, FilePatternParseError> {
    let (dir, pattern) = split_glob_path_i(input.as_ref());
    let dir = RepoPathBuf::from_relative_path(dir)?;
    prefix_glob_at(dir, pattern, true)
}

fn file_glob_at(
    dir: RepoPathBuf,
    input: &str,
    icase: bool,
) -> Result<FilePattern, FilePatternParseError> {
    if input.is_empty() {
        return Ok(FilePattern::FilePath(dir));
    }
    // Normalize separator to '/', reject ".." which will never match
    let normalized = RepoPathBuf::from_relative_path(input)?;
    let pattern = Box::new(parse_file_glob(
        normalized.as_internal_file_string(),
        icase,
    )?);
    Ok(FilePattern::FileGlob { dir, pattern })
}

fn prefix_glob_at(
    dir: RepoPathBuf,
    input: &str,
    icase: bool,
) -> Result<FilePattern, FilePatternParseError> {
    if input.is_empty() {
        return Ok(FilePattern::PrefixPath(dir));
    }
    // Normalize separator to '/', reject ".." which will never match
    let normalized = RepoPathBuf::from_relative_path(input)?;
    let pattern = Box::new(parse_file_glob(
        normalized.as_internal_file_string(),
        icase,
    )?);
    Ok(FilePattern::PrefixGlob { dir, pattern })
}

fn parse_file_glob(input: &str, icase: bool) -> Result<PathGlobPattern, globset::Error> {
    if icase {
        PathGlobPattern::parse_i(input)
    } else {
        PathGlobPattern::parse(input)
    }
}

/// Checks if a character is a glob metacharacter.
fn is_glob_char(c: char) -> bool {
    // See globset::escape(). In addition to that, backslash is parsed as an
    // escape sequence on Unix.
    const GLOB_CHARS: &[char] = if cfg!(windows) {
        &['?', '*', '[', ']', '{', '}']
    } else {
        &['?', '*', '[', ']', '{', '}', '\\']
    };
    GLOB_CHARS.contains(&c)
}

/// Splits `input` path into literal directory path and glob pattern.
fn split_glob_path(input: &str) -> (&str, &str) {
    let prefix_len = input
        .split_inclusive(path::is_separator)
        .take_while(|component| !component.contains(is_glob_char))
        .map(|component| component.len())
        .sum();
    input.split_at(prefix_len)
}

/// Splits `input` path into literal directory path and glob pattern, for
/// case-insensitive patterns.
fn split_glob_path_i(input: &str) -> (&str, &str) {
    let prefix_len = input
        .split_inclusive(path::is_separator)
        .take_while(|component| {
            !component.contains(|c: char| c.is_ascii_alphabetic() || is_glob_char(c))
        })
        .map(|component| component.len())
        .sum();
    input.split_at(prefix_len)
}

type FilesetFunction = fn(
    &mut FilesetDiagnostics,
    &RepoPathUiConverter,
    &FunctionCallNode,
) -> FilesetParseResult<FilesetExpression>;

static BUILTIN_FUNCTION_MAP: LazyLock<HashMap<&str, FilesetFunction>> = LazyLock::new(|| {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map: HashMap<&str, FilesetFunction> = HashMap::new();
    map.insert("none", |_diagnostics, _path_converter, function| {
        function.expect_no_arguments()?;
        Ok(FilesetExpression::none())
    });
    map.insert("all", |_diagnostics, _path_converter, function| {
        function.expect_no_arguments()?;
        Ok(FilesetExpression::all())
    });
    map
});

fn resolve_function(
    diagnostics: &mut FilesetDiagnostics,
    path_converter: &RepoPathUiConverter,
    function: &FunctionCallNode,
) -> FilesetParseResult<FilesetExpression> {
    if let Some(func) = BUILTIN_FUNCTION_MAP.get(function.name) {
        func(diagnostics, path_converter, function)
    } else {
        Err(FilesetParseError::new(
            FilesetParseErrorKind::NoSuchFunction {
                name: function.name.to_owned(),
                candidates: collect_similar(function.name, BUILTIN_FUNCTION_MAP.keys()),
            },
            function.name_span,
        ))
    }
}

fn resolve_expression(
    diagnostics: &mut FilesetDiagnostics,
    path_converter: &RepoPathUiConverter,
    node: &ExpressionNode,
) -> FilesetParseResult<FilesetExpression> {
    fileset_parser::catch_aliases(diagnostics, node, |diagnostics, node| {
        let wrap_pattern_error =
            |err| FilesetParseError::expression("Invalid file pattern", node.span).with_source(err);
        match &node.kind {
            ExpressionKind::Identifier(name) => {
                let pattern =
                    parse_cwd_prefix_glob(path_converter, name).map_err(wrap_pattern_error)?;
                Ok(FilesetExpression::pattern(pattern))
            }
            ExpressionKind::String(name) => {
                let pattern =
                    parse_cwd_prefix_glob(path_converter, name).map_err(wrap_pattern_error)?;
                Ok(FilesetExpression::pattern(pattern))
            }
            ExpressionKind::Pattern(pattern) => {
                let value = fileset_parser::expect_string_literal("string", &pattern.value)?;
                let pattern = parse_pattern_kind(path_converter, value, pattern.name)
                    .map_err(wrap_pattern_error)?;
                Ok(FilesetExpression::pattern(pattern))
            }
            ExpressionKind::Unary(op, arg_node) => {
                let arg = resolve_expression(diagnostics, path_converter, arg_node)?;
                match op {
                    UnaryOp::Negate => Ok(FilesetExpression::all().difference(arg)),
                }
            }
            ExpressionKind::Binary(op, lhs_node, rhs_node) => {
                let lhs = resolve_expression(diagnostics, path_converter, lhs_node)?;
                let rhs = resolve_expression(diagnostics, path_converter, rhs_node)?;
                match op {
                    BinaryOp::Intersection => Ok(lhs.intersection(rhs)),
                    BinaryOp::Difference => Ok(lhs.difference(rhs)),
                }
            }
            ExpressionKind::UnionAll(nodes) => {
                let expressions = nodes
                    .iter()
                    .map(|node| resolve_expression(diagnostics, path_converter, node))
                    .try_collect()?;
                Ok(FilesetExpression::union_all(expressions))
            }
            ExpressionKind::FunctionCall(function) => {
                resolve_function(diagnostics, path_converter, function)
            }
            ExpressionKind::AliasExpanded(..) => unreachable!(),
        }
    })
}

/// Information needed to parse fileset expression.
#[derive(Clone, Debug)]
pub struct FilesetParseContext<'a> {
    /// Aliases to be expanded.
    pub aliases_map: &'a FilesetAliasesMap,
    /// Context to resolve cwd-relative paths.
    pub path_converter: &'a RepoPathUiConverter,
}

/// Parses text into `FilesetExpression` without bare string fallback.
pub fn parse(
    diagnostics: &mut FilesetDiagnostics,
    text: &str,
    context: &FilesetParseContext,
) -> FilesetParseResult<FilesetExpression> {
    let node = fileset_parser::parse_program(text)?;
    let node = fileset_parser::expand_aliases(node, context.aliases_map)?;
    // TODO: add basic tree substitution pass to eliminate redundant expressions
    resolve_expression(diagnostics, context.path_converter, &node)
}

/// Parses text into `FilesetExpression` with bare string fallback.
///
/// If the text can't be parsed as a fileset expression, and if it doesn't
/// contain any operator-like characters, it will be parsed as a file path.
pub fn parse_maybe_bare(
    diagnostics: &mut FilesetDiagnostics,
    text: &str,
    context: &FilesetParseContext,
) -> FilesetParseResult<FilesetExpression> {
    let node = fileset_parser::parse_program_or_bare_string(text)?;
    let node = fileset_parser::expand_aliases(node, context.aliases_map)?;
    // TODO: add basic tree substitution pass to eliminate redundant expressions
    resolve_expression(diagnostics, context.path_converter, &node)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::tests::TestResult;

    fn insta_settings() -> insta::Settings {
        let mut settings = insta::Settings::clone_current();
        // Collapse short "Thing(_,)" repeatedly to save vertical space and make
        // the output more readable.
        for _ in 0..4 {
            settings.add_filter(
                r"(?x)
                \b([A-Z]\w*)\(\n
                    \s*(.{1,60}),\n
                \s*\)",
                "$1($2)",
            );
        }
        settings
    }

    #[test]
    fn test_parse_file_pattern() -> TestResult {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let context = FilesetParseContext {
            aliases_map: &FilesetAliasesMap::new(),
            path_converter: &RepoPathUiConverter::Fs {
                cwd: PathBuf::from("/ws/cur"),
                base: PathBuf::from("/ws"),
            },
        };
        let parse = |text| parse_maybe_bare(&mut FilesetDiagnostics::new(), text, &context);

        // cwd-relative patterns
        insta::assert_debug_snapshot!(
            parse(".")?,
            @r#"Pattern(PrefixPath("cur"))"#);
        insta::assert_debug_snapshot!(
            parse("..")?,
            @r#"Pattern(PrefixPath(""))"#);
        assert!(parse("../..").is_err());
        insta::assert_debug_snapshot!(
            parse("foo")?,
            @r#"Pattern(PrefixPath("cur/foo"))"#);
        insta::assert_debug_snapshot!(
            parse("*.*")?,
            @r#"
        Pattern(
            PrefixGlob {
                dir: "cur",
                pattern: PathGlobPattern {
                    glob: "*.*",
                    re: "(?-u)^[^/]*\\.[^/]*$",
                    ..
                },
            },
        )
        "#);
        insta::assert_debug_snapshot!(
            parse("cwd:.")?,
            @r#"Pattern(PrefixPath("cur"))"#);
        insta::assert_debug_snapshot!(
            parse("cwd-file:foo")?,
            @r#"Pattern(FilePath("cur/foo"))"#);
        insta::assert_debug_snapshot!(
            parse("file:../foo/bar")?,
            @r#"Pattern(FilePath("foo/bar"))"#);

        // workspace-relative patterns
        insta::assert_debug_snapshot!(
            parse("root:.")?,
            @r#"Pattern(PrefixPath(""))"#);
        assert!(parse("root:..").is_err());
        insta::assert_debug_snapshot!(
            parse("root:foo/bar")?,
            @r#"Pattern(PrefixPath("foo/bar"))"#);
        insta::assert_debug_snapshot!(
            parse("root-file:bar")?,
            @r#"Pattern(FilePath("bar"))"#);

        insta::assert_debug_snapshot!(
            parse("file:(foo|bar)").unwrap_err().kind(),
            @r#"Expression("Expected string")"#);
        Ok(())
    }

    #[test]
    fn test_parse_glob_pattern() -> TestResult {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let context = FilesetParseContext {
            aliases_map: &FilesetAliasesMap::new(),
            path_converter: &RepoPathUiConverter::Fs {
                // meta character in cwd path shouldn't be expanded
                cwd: PathBuf::from("/ws/cur*"),
                base: PathBuf::from("/ws"),
            },
        };
        let parse = |text| parse_maybe_bare(&mut FilesetDiagnostics::new(), text, &context);

        // cwd-relative, without meta characters
        insta::assert_debug_snapshot!(
            parse(r#"cwd-glob:"foo""#)?,
            @r#"Pattern(FilePath("cur*/foo"))"#);
        // Strictly speaking, glob:"" shouldn't match a file named <cwd>, but
        // file pattern doesn't distinguish "foo/" from "foo".
        insta::assert_debug_snapshot!(
            parse(r#"glob:"""#)?,
            @r#"Pattern(FilePath("cur*"))"#);
        insta::assert_debug_snapshot!(
            parse(r#"glob:".""#)?,
            @r#"Pattern(FilePath("cur*"))"#);
        insta::assert_debug_snapshot!(
            parse(r#"glob:"..""#)?,
            @r#"Pattern(FilePath(""))"#);

        // cwd-relative, with meta characters
        insta::assert_debug_snapshot!(
            parse(r#"glob:"*""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "cur*",
                pattern: PathGlobPattern {
                    glob: "*",
                    re: "(?-u)^[^/]*$",
                    ..
                },
            },
        )
        "#);
        insta::assert_debug_snapshot!(
            parse(r#"glob:"./*""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "cur*",
                pattern: PathGlobPattern {
                    glob: "*",
                    re: "(?-u)^[^/]*$",
                    ..
                },
            },
        )
        "#);
        insta::assert_debug_snapshot!(
            parse(r#"glob:"../*""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "",
                pattern: PathGlobPattern {
                    glob: "*",
                    re: "(?-u)^[^/]*$",
                    ..
                },
            },
        )
        "#);
        // glob:"**" is equivalent to root-glob:"<cwd>/**", not root-glob:"**"
        insta::assert_debug_snapshot!(
            parse(r#"glob:"**""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "cur*",
                pattern: PathGlobPattern {
                    glob: "**",
                    re: "(?-u)^.*$",
                    ..
                },
            },
        )
        "#);
        insta::assert_debug_snapshot!(
            parse(r#"glob:"../foo/b?r/baz""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "foo",
                pattern: PathGlobPattern {
                    glob: "b?r/baz",
                    re: "(?-u)^b[^/]r/baz$",
                    ..
                },
            },
        )
        "#);
        assert!(parse(r#"glob:"../../*""#).is_err());
        assert!(parse(r#"glob-i:"../../*""#).is_err());
        assert!(parse(r#"glob:"/*""#).is_err());
        assert!(parse(r#"glob-i:"/*""#).is_err());
        // no support for relative path component after glob meta character
        assert!(parse(r#"glob:"*/..""#).is_err());
        assert!(parse(r#"glob-i:"*/..""#).is_err());

        if cfg!(windows) {
            // cwd-relative, with Windows path separators
            insta::assert_debug_snapshot!(
                parse(r#"glob:"..\\foo\\*\\bar""#)?, @r#"
            Pattern(
                FileGlob {
                    dir: "foo",
                    pattern: PathGlobPattern {
                        glob: "*/bar",
                        re: "(?-u)^[^/]*/bar$",
                        ..
                    },
                },
            )
            "#);
        } else {
            // backslash is an escape character on Unix
            insta::assert_debug_snapshot!(
                parse(r#"glob:"..\\foo\\*\\bar""#)?, @r#"
            Pattern(
                FileGlob {
                    dir: "cur*",
                    pattern: PathGlobPattern {
                        glob: "..\\foo\\*\\bar",
                        re: "(?-u)^\\.\\.foo\\*bar$",
                        ..
                    },
                },
            )
            "#);
        }

        // workspace-relative, without meta characters
        insta::assert_debug_snapshot!(
            parse(r#"root-glob:"foo""#)?,
            @r#"Pattern(FilePath("foo"))"#);
        insta::assert_debug_snapshot!(
            parse(r#"root-glob:"""#)?,
            @r#"Pattern(FilePath(""))"#);
        insta::assert_debug_snapshot!(
            parse(r#"root-glob:".""#)?,
            @r#"Pattern(FilePath(""))"#);

        // workspace-relative, with meta characters
        insta::assert_debug_snapshot!(
            parse(r#"root-glob:"*""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "",
                pattern: PathGlobPattern {
                    glob: "*",
                    re: "(?-u)^[^/]*$",
                    ..
                },
            },
        )
        "#);
        insta::assert_debug_snapshot!(
            parse(r#"root-glob:"foo/bar/b[az]""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "foo/bar",
                pattern: PathGlobPattern {
                    glob: "b[az]",
                    re: "(?-u)^b[az]$",
                    ..
                },
            },
        )
        "#);
        insta::assert_debug_snapshot!(
            parse(r#"root-glob:"foo/bar/b{ar,az}""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "foo/bar",
                pattern: PathGlobPattern {
                    glob: "b{ar,az}",
                    re: "(?-u)^b(?:ar|az)$",
                    ..
                },
            },
        )
        "#);
        assert!(parse(r#"root-glob:"../*""#).is_err());
        assert!(parse(r#"root-glob-i:"../*""#).is_err());
        assert!(parse(r#"root-glob:"/*""#).is_err());
        assert!(parse(r#"root-glob-i:"/*""#).is_err());

        // workspace-relative, backslash escape without meta characters
        if cfg!(not(windows)) {
            insta::assert_debug_snapshot!(
                parse(r#"root-glob:'foo/bar\baz'"#)?, @r#"
            Pattern(
                FileGlob {
                    dir: "foo",
                    pattern: PathGlobPattern {
                        glob: "bar\\baz",
                        re: "(?-u)^barbaz$",
                        ..
                    },
                },
            )
            "#);
        }
        Ok(())
    }

    #[test]
    fn test_parse_glob_pattern_case_insensitive() -> TestResult {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let context = FilesetParseContext {
            aliases_map: &FilesetAliasesMap::new(),
            path_converter: &RepoPathUiConverter::Fs {
                cwd: PathBuf::from("/ws/cur"),
                base: PathBuf::from("/ws"),
            },
        };
        let parse = |text| parse_maybe_bare(&mut FilesetDiagnostics::new(), text, &context);

        // cwd-relative case-insensitive glob
        insta::assert_debug_snapshot!(
            parse(r#"glob-i:"*.TXT""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "cur",
                pattern: PathGlobPattern {
                    glob: "*.TXT",
                    re: "(?-u)(?i)^[^/]*\\.TXT$",
                    ..
                },
            },
        )
        "#);

        // cwd-relative case-insensitive glob with more specific pattern
        insta::assert_debug_snapshot!(
            parse(r#"cwd-glob-i:"[Ff]oo""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "cur",
                pattern: PathGlobPattern {
                    glob: "[Ff]oo",
                    re: "(?-u)(?i)^[Ff]oo$",
                    ..
                },
            },
        )
        "#);

        // workspace-relative case-insensitive glob
        insta::assert_debug_snapshot!(
            parse(r#"root-glob-i:"*.Rs""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "",
                pattern: PathGlobPattern {
                    glob: "*.Rs",
                    re: "(?-u)(?i)^[^/]*\\.Rs$",
                    ..
                },
            },
        )
        "#);

        // case-insensitive pattern with directory component (should not split the path)
        insta::assert_debug_snapshot!(
            parse(r#"glob-i:"SubDir/*.rs""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "cur",
                pattern: PathGlobPattern {
                    glob: "SubDir/*.rs",
                    re: "(?-u)(?i)^SubDir/[^/]*\\.rs$",
                    ..
                },
            },
        )
        "#);

        // case-sensitive pattern with directory component (should split the path)
        insta::assert_debug_snapshot!(
            parse(r#"glob:"SubDir/*.rs""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "cur/SubDir",
                pattern: PathGlobPattern {
                    glob: "*.rs",
                    re: "(?-u)^[^/]*\\.rs$",
                    ..
                },
            },
        )
        "#);

        // case-insensitive pattern with leading dots (should split dots but not dirs)
        insta::assert_debug_snapshot!(
            parse(r#"glob-i:"../SomeDir/*.rs""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "",
                pattern: PathGlobPattern {
                    glob: "SomeDir/*.rs",
                    re: "(?-u)(?i)^SomeDir/[^/]*\\.rs$",
                    ..
                },
            },
        )
        "#);

        // case-insensitive pattern with single leading dot
        insta::assert_debug_snapshot!(
            parse(r#"glob-i:"./SomeFile*.txt""#)?, @r#"
        Pattern(
            FileGlob {
                dir: "cur",
                pattern: PathGlobPattern {
                    glob: "SomeFile*.txt",
                    re: "(?-u)(?i)^SomeFile[^/]*\\.txt$",
                    ..
                },
            },
        )
        "#);
        Ok(())
    }

    #[test]
    fn test_parse_prefix_glob_pattern() -> TestResult {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let context = FilesetParseContext {
            aliases_map: &FilesetAliasesMap::new(),
            path_converter: &RepoPathUiConverter::Fs {
                // meta character in cwd path shouldn't be expanded
                cwd: PathBuf::from("/ws/cur*"),
                base: PathBuf::from("/ws"),
            },
        };
        let parse = |text| parse_maybe_bare(&mut FilesetDiagnostics::new(), text, &context);

        // cwd-relative, without meta/case-insensitive characters
        insta::assert_debug_snapshot!(
            parse("cwd-prefix-glob:'foo'")?,
            @r#"Pattern(PrefixPath("cur*/foo"))"#);
        insta::assert_debug_snapshot!(
            parse("prefix-glob:'.'")?,
            @r#"Pattern(PrefixPath("cur*"))"#);
        insta::assert_debug_snapshot!(
            parse("cwd-prefix-glob-i:'..'")?,
            @r#"Pattern(PrefixPath(""))"#);
        insta::assert_debug_snapshot!(
            parse("prefix-glob-i:'../_'")?,
            @r#"Pattern(PrefixPath("_"))"#);

        // cwd-relative, with meta characters
        insta::assert_debug_snapshot!(
            parse("cwd-prefix-glob:'*'")?, @r#"
        Pattern(
            PrefixGlob {
                dir: "cur*",
                pattern: PathGlobPattern {
                    glob: "*",
                    re: "(?-u)^[^/]*$",
                    ..
                },
            },
        )
        "#);

        // cwd-relative, with case-insensitive characters
        insta::assert_debug_snapshot!(
            parse("cwd-prefix-glob-i:'../foo'")?, @r#"
        Pattern(
            PrefixGlob {
                dir: "",
                pattern: PathGlobPattern {
                    glob: "foo",
                    re: "(?-u)(?i)^foo$",
                    ..
                },
            },
        )
        "#);

        // workspace-relative, without meta/case-insensitive characters
        insta::assert_debug_snapshot!(
            parse("root-prefix-glob:'foo'")?,
            @r#"Pattern(PrefixPath("foo"))"#);
        insta::assert_debug_snapshot!(
            parse("root-prefix-glob-i:'.'")?,
            @r#"Pattern(PrefixPath(""))"#);

        // workspace-relative, with meta characters
        insta::assert_debug_snapshot!(
            parse("root-prefix-glob:'*'")?, @r#"
        Pattern(
            PrefixGlob {
                dir: "",
                pattern: PathGlobPattern {
                    glob: "*",
                    re: "(?-u)^[^/]*$",
                    ..
                },
            },
        )
        "#);

        // workspace-relative, with case-insensitive characters
        insta::assert_debug_snapshot!(
            parse("root-prefix-glob-i:'_/foo'")?, @r#"
        Pattern(
            PrefixGlob {
                dir: "_",
                pattern: PathGlobPattern {
                    glob: "foo",
                    re: "(?-u)(?i)^foo$",
                    ..
                },
            },
        )
        "#);
        Ok(())
    }

    #[test]
    fn test_parse_function() -> TestResult {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let context = FilesetParseContext {
            aliases_map: &FilesetAliasesMap::new(),
            path_converter: &RepoPathUiConverter::Fs {
                cwd: PathBuf::from("/ws/cur"),
                base: PathBuf::from("/ws"),
            },
        };
        let parse = |text| parse_maybe_bare(&mut FilesetDiagnostics::new(), text, &context);

        insta::assert_debug_snapshot!(parse("all()")?, @"All");
        insta::assert_debug_snapshot!(parse("none()")?, @"None");
        insta::assert_debug_snapshot!(parse("all(x)").unwrap_err().kind(), @r#"
        InvalidArguments {
            name: "all",
            message: "Expected 0 arguments",
        }
        "#);
        insta::assert_debug_snapshot!(parse("ale()").unwrap_err().kind(), @r#"
        NoSuchFunction {
            name: "ale",
            candidates: [
                "all",
            ],
        }
        "#);
        Ok(())
    }

    #[test]
    fn test_parse_compound_expression() -> TestResult {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let context = FilesetParseContext {
            aliases_map: &FilesetAliasesMap::new(),
            path_converter: &RepoPathUiConverter::Fs {
                cwd: PathBuf::from("/ws/cur"),
                base: PathBuf::from("/ws"),
            },
        };
        let parse = |text| parse_maybe_bare(&mut FilesetDiagnostics::new(), text, &context);

        insta::assert_debug_snapshot!(parse("~x")?, @r#"
        Difference(
            All,
            Pattern(PrefixPath("cur/x")),
        )
        "#);
        insta::assert_debug_snapshot!(parse("x|y|root:z")?, @r#"
        UnionAll(
            [
                Pattern(PrefixPath("cur/x")),
                Pattern(PrefixPath("cur/y")),
                Pattern(PrefixPath("z")),
            ],
        )
        "#);
        insta::assert_debug_snapshot!(parse("x|y&z")?, @r#"
        UnionAll(
            [
                Pattern(PrefixPath("cur/x")),
                Intersection(
                    Pattern(PrefixPath("cur/y")),
                    Pattern(PrefixPath("cur/z")),
                ),
            ],
        )
        "#);
        Ok(())
    }
}
