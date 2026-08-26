// Copyright 2026 The Jujutsu Authors
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

//! Types for evaluating filesets: file patterns, the [`FilesetExpression`]
//! tree, and their conversion to [`Matcher`]s. The fileset language (parsing,
//! alias expansion) and the parsing of `FilePattern`s from user input live in
//! `jj_lib::fileset`.

use std::iter;
use std::slice;

use crate::matchers::DifferenceMatcher;
use crate::matchers::EverythingMatcher;
use crate::matchers::FilesMatcher;
use crate::matchers::GlobsMatcher;
use crate::matchers::IntersectionMatcher;
use crate::matchers::Matcher;
use crate::matchers::NothingMatcher;
use crate::matchers::PathGlobPattern;
use crate::matchers::PrefixMatcher;
use crate::matchers::UnionMatcher;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;

/// Basic pattern to match `RepoPath`.
#[derive(Clone, Debug)]
pub enum FilePattern {
    /// Matches file (or exact) path.
    FilePath(RepoPathBuf),
    /// Matches path prefix.
    PrefixPath(RepoPathBuf),
    /// Matches file (or exact) path with glob pattern.
    FileGlob {
        /// Prefix directory path where the `pattern` will be evaluated.
        dir: RepoPathBuf,
        /// Glob pattern relative to `dir`.
        pattern: Box<PathGlobPattern>,
    },
    /// Matches path prefix with glob pattern.
    PrefixGlob {
        /// Prefix directory path where the `pattern` will be evaluated.
        dir: RepoPathBuf,
        /// Glob pattern relative to `dir`.
        pattern: Box<PathGlobPattern>,
    },
    // TODO: add more patterns:
    // - FilesInPath: files in directory, non-recursively?
    // - NameGlob or SuffixGlob: file name with glob?
}

impl FilePattern {
    /// Returns path if this pattern represents a literal path in a workspace.
    /// Returns `None` if this is a glob pattern for example.
    pub fn as_path(&self) -> Option<&RepoPath> {
        match self {
            Self::FilePath(path) => Some(path),
            Self::PrefixPath(path) => Some(path),
            Self::FileGlob { .. } | Self::PrefixGlob { .. } => None,
        }
    }
}

/// AST-level representation of the fileset expression.
#[derive(Clone, Debug)]
pub enum FilesetExpression {
    /// Matches nothing.
    None,
    /// Matches everything.
    All,
    /// Matches basic pattern.
    Pattern(FilePattern),
    /// Matches any of the expressions.
    ///
    /// Use `FilesetExpression::union_all()` to construct a union expression.
    /// It will normalize 0-ary or 1-ary union.
    UnionAll(Vec<Self>),
    /// Matches both expressions.
    Intersection(Box<Self>, Box<Self>),
    /// Matches the first expression, but not the second expression.
    Difference(Box<Self>, Box<Self>),
}

impl FilesetExpression {
    /// Expression that matches nothing.
    pub fn none() -> Self {
        Self::None
    }

    /// Expression that matches everything.
    pub fn all() -> Self {
        Self::All
    }

    /// Expression that matches the given `pattern`.
    pub fn pattern(pattern: FilePattern) -> Self {
        Self::Pattern(pattern)
    }

    /// Expression that matches file (or exact) path.
    pub fn file_path(path: RepoPathBuf) -> Self {
        Self::Pattern(FilePattern::FilePath(path))
    }

    /// Expression that matches path prefix.
    pub fn prefix_path(path: RepoPathBuf) -> Self {
        Self::Pattern(FilePattern::PrefixPath(path))
    }

    /// Expression that matches any of the given `expressions`.
    pub fn union_all(expressions: Vec<Self>) -> Self {
        match expressions.len() {
            0 => Self::none(),
            1 => expressions.into_iter().next().unwrap(),
            _ => Self::UnionAll(expressions),
        }
    }

    /// Expression that matches both `self` and `other`.
    pub fn intersection(self, other: Self) -> Self {
        Self::Intersection(Box::new(self), Box::new(other))
    }

    /// Expression that matches `self` but not `other`.
    pub fn difference(self, other: Self) -> Self {
        Self::Difference(Box::new(self), Box::new(other))
    }

    /// Flattens union expression at most one level.
    fn as_union_all(&self) -> &[Self] {
        match self {
            Self::None => &[],
            Self::UnionAll(exprs) => exprs,
            _ => slice::from_ref(self),
        }
    }

    fn dfs_pre(&self) -> impl Iterator<Item = &Self> {
        let mut stack: Vec<&Self> = vec![self];
        iter::from_fn(move || {
            let expr = stack.pop()?;
            match expr {
                Self::None | Self::All | Self::Pattern(_) => {}
                Self::UnionAll(exprs) => stack.extend(exprs.iter().rev()),
                Self::Intersection(expr1, expr2) | Self::Difference(expr1, expr2) => {
                    stack.push(expr2);
                    stack.push(expr1);
                }
            }
            Some(expr)
        })
    }

    /// Iterates literal paths recursively from this expression.
    ///
    /// For example, `"a", "b", "c"` will be yielded in that order for
    /// expression `"a" | all() & "b" | ~"c"`.
    pub fn explicit_paths(&self) -> impl Iterator<Item = &RepoPath> {
        // pre/post-ordering doesn't matter so long as children are visited from
        // left to right.
        self.dfs_pre().filter_map(|expr| match expr {
            Self::Pattern(pattern) => pattern.as_path(),
            _ => None,
        })
    }

    /// Transforms the expression tree to `Matcher` object.
    pub fn to_matcher(&self) -> Box<dyn Matcher> {
        build_union_matcher(self.as_union_all())
    }
}

/// Transforms the union `expressions` to `Matcher` object.
///
/// Since `Matcher` typically accepts a set of patterns to be OR-ed, this
/// function takes a list of union `expressions` as input.
fn build_union_matcher(expressions: &[FilesetExpression]) -> Box<dyn Matcher> {
    let mut file_paths = Vec::new();
    let mut prefix_paths = Vec::new();
    let mut file_globs = GlobsMatcher::builder().prefix_paths(false);
    let mut prefix_globs = GlobsMatcher::builder().prefix_paths(true);
    let mut matchers: Vec<Option<Box<dyn Matcher>>> = Vec::new();
    for expr in expressions {
        let matcher: Box<dyn Matcher> = match expr {
            // None and All are supposed to be simplified by caller.
            FilesetExpression::None => Box::new(NothingMatcher),
            FilesetExpression::All => Box::new(EverythingMatcher),
            FilesetExpression::Pattern(pattern) => {
                match pattern {
                    FilePattern::FilePath(path) => file_paths.push(path),
                    FilePattern::PrefixPath(path) => prefix_paths.push(path),
                    FilePattern::FileGlob { dir, pattern } => file_globs.add(dir, pattern),
                    FilePattern::PrefixGlob { dir, pattern } => prefix_globs.add(dir, pattern),
                }
                continue;
            }
            // UnionAll is supposed to be flattened by caller.
            FilesetExpression::UnionAll(exprs) => build_union_matcher(exprs),
            FilesetExpression::Intersection(expr1, expr2) => {
                let m1 = build_union_matcher(expr1.as_union_all());
                let m2 = build_union_matcher(expr2.as_union_all());
                Box::new(IntersectionMatcher::new(m1, m2))
            }
            FilesetExpression::Difference(expr1, expr2) => {
                let m1 = build_union_matcher(expr1.as_union_all());
                let m2 = build_union_matcher(expr2.as_union_all());
                Box::new(DifferenceMatcher::new(m1, m2))
            }
        };
        matchers.push(Some(matcher));
    }

    if !file_paths.is_empty() {
        matchers.push(Some(Box::new(FilesMatcher::new(file_paths))));
    }
    if !prefix_paths.is_empty() {
        matchers.push(Some(Box::new(PrefixMatcher::new(prefix_paths))));
    }
    if !file_globs.is_empty() {
        matchers.push(Some(Box::new(file_globs.build())));
    }
    if !prefix_globs.is_empty() {
        matchers.push(Some(Box::new(prefix_globs.build())));
    }
    union_all_matchers(&mut matchers)
}

/// Concatenates all `matchers` as union.
///
/// Each matcher element must be wrapped in `Some` so the matchers can be moved
/// in arbitrary order.
fn union_all_matchers(matchers: &mut [Option<Box<dyn Matcher>>]) -> Box<dyn Matcher> {
    match matchers {
        [] => Box::new(NothingMatcher),
        [matcher] => matcher.take().expect("matcher should still be available"),
        _ => {
            // Build balanced tree to minimize the recursion depth.
            let (left, right) = matchers.split_at_mut(matchers.len() / 2);
            let m1 = union_all_matchers(left);
            let m2 = union_all_matchers(right);
            Box::new(UnionMatcher::new(m1, m2))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_path_buf(value: impl Into<String>) -> RepoPathBuf {
        RepoPathBuf::from_internal_string(value).unwrap()
    }

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
    fn test_explicit_paths() {
        let collect = |expr: &FilesetExpression| -> Vec<RepoPathBuf> {
            expr.explicit_paths().map(|path| path.to_owned()).collect()
        };
        let file_expr = |path: &str| FilesetExpression::file_path(repo_path_buf(path));
        assert!(collect(&FilesetExpression::none()).is_empty());
        assert_eq!(collect(&file_expr("a")), ["a"].map(repo_path_buf));
        assert_eq!(
            collect(&FilesetExpression::union_all(vec![
                file_expr("a"),
                file_expr("b"),
                file_expr("c"),
            ])),
            ["a", "b", "c"].map(repo_path_buf)
        );
        assert_eq!(
            collect(&FilesetExpression::intersection(
                FilesetExpression::union_all(vec![
                    file_expr("a"),
                    FilesetExpression::none(),
                    file_expr("b"),
                    file_expr("c"),
                ]),
                FilesetExpression::difference(
                    file_expr("d"),
                    FilesetExpression::union_all(vec![file_expr("e"), file_expr("f")])
                )
            )),
            ["a", "b", "c", "d", "e", "f"].map(repo_path_buf)
        );
    }

    #[test]
    fn test_build_matcher_simple() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(FilesetExpression::none().to_matcher(), @"NothingMatcher");
        insta::assert_debug_snapshot!(FilesetExpression::all().to_matcher(), @"EverythingMatcher");
        insta::assert_debug_snapshot!(
            FilesetExpression::file_path(repo_path_buf("foo")).to_matcher(),
            @r#"
        FilesMatcher {
            tree: Dir {
                "foo": File {},
            },
        }
        "#);
        insta::assert_debug_snapshot!(
            FilesetExpression::prefix_path(repo_path_buf("foo")).to_matcher(),
            @r#"
        PrefixMatcher {
            tree: Dir {
                "foo": Prefix {},
            },
        }
        "#);
    }

    #[test]
    fn test_build_matcher_glob_pattern() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let file_glob_expr = |dir: &str, pattern: &str| {
            FilesetExpression::pattern(FilePattern::FileGlob {
                dir: repo_path_buf(dir),
                pattern: Box::new(PathGlobPattern::parse(pattern).unwrap()),
            })
        };
        let prefix_glob_expr = |dir: &str, pattern: &str| {
            FilesetExpression::pattern(FilePattern::PrefixGlob {
                dir: repo_path_buf(dir),
                pattern: Box::new(PathGlobPattern::parse(pattern).unwrap()),
            })
        };

        insta::assert_debug_snapshot!(file_glob_expr("", "*").to_matcher(), @r#"
        GlobsMatcher {
            tree: Some(RegexSet(["(?-u)^[^/]*$"])) {},
            matches_prefix_paths: false,
        }
        "#);

        let expr = FilesetExpression::union_all(vec![
            file_glob_expr("foo", "*"),
            file_glob_expr("foo/bar", "*"),
            file_glob_expr("foo", "?"),
            prefix_glob_expr("foo", "ba[rz]"),
            prefix_glob_expr("foo", "qu*x"),
        ]);
        insta::assert_debug_snapshot!(expr.to_matcher(), @r#"
        UnionMatcher {
            input1: GlobsMatcher {
                tree: None {
                    "foo": Some(RegexSet(["(?-u)^[^/]*$", "(?-u)^[^/]$"])) {
                        "bar": Some(RegexSet(["(?-u)^[^/]*$"])) {},
                    },
                },
                matches_prefix_paths: false,
            },
            input2: GlobsMatcher {
                tree: None {
                    "foo": Some(RegexSet(["(?-u)^ba[rz](?:/|$)", "(?-u)^qu[^/]*x(?:/|$)"])) {},
                },
                matches_prefix_paths: true,
            },
        }
        "#);
    }

    #[test]
    fn test_build_matcher_union_patterns_of_same_kind() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        let expr = FilesetExpression::union_all(vec![
            FilesetExpression::file_path(repo_path_buf("foo")),
            FilesetExpression::file_path(repo_path_buf("foo/bar")),
        ]);
        insta::assert_debug_snapshot!(expr.to_matcher(), @r#"
        FilesMatcher {
            tree: Dir {
                "foo": File {
                    "bar": File {},
                },
            },
        }
        "#);

        let expr = FilesetExpression::union_all(vec![
            FilesetExpression::prefix_path(repo_path_buf("bar")),
            FilesetExpression::prefix_path(repo_path_buf("bar/baz")),
        ]);
        insta::assert_debug_snapshot!(expr.to_matcher(), @r#"
        PrefixMatcher {
            tree: Dir {
                "bar": Prefix {
                    "baz": Prefix {},
                },
            },
        }
        "#);
    }

    #[test]
    fn test_build_matcher_union_patterns_of_different_kind() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        let expr = FilesetExpression::union_all(vec![
            FilesetExpression::file_path(repo_path_buf("foo")),
            FilesetExpression::prefix_path(repo_path_buf("bar")),
        ]);
        insta::assert_debug_snapshot!(expr.to_matcher(), @r#"
        UnionMatcher {
            input1: FilesMatcher {
                tree: Dir {
                    "foo": File {},
                },
            },
            input2: PrefixMatcher {
                tree: Dir {
                    "bar": Prefix {},
                },
            },
        }
        "#);
    }

    #[test]
    fn test_build_matcher_unnormalized_union() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        let expr = FilesetExpression::UnionAll(vec![]);
        insta::assert_debug_snapshot!(expr.to_matcher(), @"NothingMatcher");

        let expr =
            FilesetExpression::UnionAll(vec![FilesetExpression::None, FilesetExpression::All]);
        insta::assert_debug_snapshot!(expr.to_matcher(), @"
        UnionMatcher {
            input1: NothingMatcher,
            input2: EverythingMatcher,
        }
        ");
    }

    #[test]
    fn test_build_matcher_combined() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        let expr = FilesetExpression::union_all(vec![
            FilesetExpression::intersection(FilesetExpression::all(), FilesetExpression::none()),
            FilesetExpression::difference(FilesetExpression::none(), FilesetExpression::all()),
            FilesetExpression::file_path(repo_path_buf("foo")),
            FilesetExpression::prefix_path(repo_path_buf("bar")),
        ]);
        insta::assert_debug_snapshot!(expr.to_matcher(), @r#"
        UnionMatcher {
            input1: UnionMatcher {
                input1: IntersectionMatcher {
                    input1: EverythingMatcher,
                    input2: NothingMatcher,
                },
                input2: DifferenceMatcher {
                    wanted: NothingMatcher,
                    unwanted: EverythingMatcher,
                },
            },
            input2: UnionMatcher {
                input1: FilesMatcher {
                    tree: Dir {
                        "foo": File {},
                    },
                },
                input2: PrefixMatcher {
                    tree: Dir {
                        "bar": Prefix {},
                    },
                },
            },
        }
        "#);
    }
}
