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
use std::vec;

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
#[derive(Clone, Debug, Eq, PartialEq)]
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
#[derive(Clone, Debug, Eq, PartialEq)]
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

    /// Returns the path prefixes this expression is a union of, or `None` if
    /// it is anything else. `all()` is the root prefix.
    ///
    /// This is the subset of expressions that a plain list of paths can
    /// represent, e.g. for `jj sparse list`.
    pub fn to_path_prefixes(&self) -> Option<Vec<&RepoPath>> {
        // `Option<Vec<_>>: FromIterator<Option<_>>` stops at the first `None`.
        self.dfs_pre()
            .filter_map(|expr| match expr {
                Self::None | Self::UnionAll(_) => None,
                Self::All => Some(Some(RepoPath::root())),
                Self::Pattern(FilePattern::PrefixPath(path)) => Some(Some(&**path)),
                Self::Pattern(_) | Self::Intersection(..) | Self::Difference(..) => Some(None),
            })
            .collect()
    }

    /// Returns true if this expression obviously matches every path, i.e. if
    /// it is a union containing the root prefix. A compound expression that
    /// happens to match everything is not detected.
    pub fn matches_everything(&self) -> bool {
        let mut found = false;
        for expr in self.dfs_pre() {
            match expr {
                Self::All => found = true,
                Self::Pattern(FilePattern::PrefixPath(path)) if path.is_root() => found = true,
                Self::None | Self::UnionAll(_) | Self::Pattern(_) => {}
                Self::Intersection(..) | Self::Difference(..) => return false,
            }
        }
        found
    }

    /// Transforms the expression tree to `Matcher` object.
    pub fn to_matcher(&self) -> Box<dyn Matcher> {
        build_union_matcher(self.as_union_all())
    }
}

/// Whether a [`FilesetRule`] adds or removes the paths it matches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilesetRuleKind {
    /// Adds the matched paths to the selection.
    Include,
    /// Removes the matched paths from the selection.
    Exclude,
}

/// One step of a [`FilesetRules`] selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilesetRule {
    /// Whether the matched paths are added or removed.
    pub kind: FilesetRuleKind,
    /// Paths this rule matches.
    pub expression: FilesetExpression,
}

/// Include/exclude rules applied in order, starting from no paths.
///
/// The last rule matching a path decides whether it is selected. This is the
/// form in which an incrementally edited selection such as a sparse checkout
/// is kept: appending a rule never deepens any single [`FilesetExpression`],
/// so expression depth stays bounded by what the user wrote rather than by how
/// many times they edited the selection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FilesetRules {
    rules: Vec<FilesetRule>,
}

impl FilesetRules {
    /// Rules that select nothing.
    pub fn none() -> Self {
        Self::default()
    }

    /// Rules that select everything.
    pub fn all() -> Self {
        Self::from_path_prefixes([RepoPathBuf::root()])
    }

    /// Rules that include each of the given path prefixes.
    ///
    /// The paths are expected to be distinct, so this skips the shadowing
    /// scan [`Self::push()`] does, which would make bulk construction
    /// quadratic.
    pub fn from_path_prefixes(paths: impl IntoIterator<Item = RepoPathBuf>) -> Self {
        let mut rules: Vec<_> = paths
            .into_iter()
            .map(|path| FilesetRule {
                kind: FilesetRuleKind::Include,
                expression: FilesetExpression::prefix_path(path),
            })
            .collect();
        // The root prefix selects everything, so it shadows all earlier rules.
        if let Some(i) = rules
            .iter()
            .rposition(|rule| rule.expression.matches_everything())
        {
            rules.drain(..i);
            rules[0].expression = FilesetExpression::all();
        }
        Self { rules }
    }

    /// Returns the path prefixes these rules include, or `None` if they
    /// exclude anything or match by glob. Inverse of
    /// [`Self::from_path_prefixes()`].
    pub fn to_path_prefixes(&self) -> Option<Vec<&RepoPath>> {
        let mut paths = Vec::new();
        for rule in &self.rules {
            if rule.kind != FilesetRuleKind::Include {
                return None;
            }
            paths.extend(rule.expression.to_path_prefixes()?);
        }
        Some(paths)
    }

    /// Returns true if there are no rules, i.e. nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The rules in application order.
    pub fn iter(&self) -> slice::Iter<'_, FilesetRule> {
        self.rules.iter()
    }

    /// Appends a rule, dropping the rules it makes unreachable.
    ///
    /// A path matched by an earlier rule with the same expression is always
    /// decided by the new rule, so that earlier rule can no longer apply at
    /// all. A rule matching every path makes every earlier rule unreachable,
    /// and is normalized to `all()` so that equivalent selections look and
    /// compare the same.
    pub fn push(&mut self, kind: FilesetRuleKind, mut expression: FilesetExpression) {
        if expression.matches_everything() {
            self.rules.clear();
            if kind == FilesetRuleKind::Exclude {
                return;
            }
            expression = FilesetExpression::all();
        } else {
            self.rules.retain(|rule| rule.expression != expression);
        }
        self.rules.push(FilesetRule { kind, expression });
    }

    /// Builds the equivalent expression.
    pub fn to_expression(&self) -> FilesetExpression {
        self.iter().fold(FilesetExpression::none(), |selected, rule| {
            match (rule.kind, selected) {
                // Nothing is selected yet, so there is nothing to exclude.
                (FilesetRuleKind::Exclude, FilesetExpression::None) => FilesetExpression::None,
                (FilesetRuleKind::Exclude, selected) => {
                    selected.difference(rule.expression.clone())
                }
                (FilesetRuleKind::Include, FilesetExpression::None) => rule.expression.clone(),
                // Flatten consecutive inclusions into a single union so that
                // they can share one `Matcher` per pattern kind.
                (FilesetRuleKind::Include, FilesetExpression::UnionAll(mut operands)) => {
                    operands.push(rule.expression.clone());
                    FilesetExpression::UnionAll(operands)
                }
                (FilesetRuleKind::Include, selected) => {
                    FilesetExpression::UnionAll(vec![selected, rule.expression.clone()])
                }
            }
        })
    }

    /// Transforms the rules to `Matcher` object.
    pub fn to_matcher(&self) -> Box<dyn Matcher> {
        self.to_expression().to_matcher()
    }
}

impl Extend<FilesetRule> for FilesetRules {
    fn extend<T: IntoIterator<Item = FilesetRule>>(&mut self, rules: T) {
        for rule in rules {
            self.push(rule.kind, rule.expression);
        }
    }
}

impl FromIterator<FilesetRule> for FilesetRules {
    fn from_iter<T: IntoIterator<Item = FilesetRule>>(rules: T) -> Self {
        let mut result = Self::none();
        result.extend(rules);
        result
    }
}

impl IntoIterator for FilesetRules {
    type IntoIter = vec::IntoIter<FilesetRule>;
    type Item = FilesetRule;

    fn into_iter(self) -> Self::IntoIter {
        self.rules.into_iter()
    }
}

impl<'a> IntoIterator for &'a FilesetRules {
    type IntoIter = slice::Iter<'a, FilesetRule>;
    type Item = &'a FilesetRule;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
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
    use itertools::Itertools as _;

    use super::*;
    use crate::matchers::Visit;
    use crate::matchers::VisitDirs;
    use crate::matchers::VisitFiles;

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

    /// Asserts that `visit()` neither prunes away a selected file nor promises
    /// an unselected one.
    fn assert_visit_agrees(matcher: &dyn Matcher, dir: &RepoPath, selected: &[(RepoPathBuf, bool)]) {
        let visit = matcher.visit(dir);
        for (file, selected) in selected {
            let Some(tail) = file.strip_prefix(dir) else {
                continue;
            };
            let mut components = tail.components();
            let name = components.next().expect("file should be under dir");
            let reachable = match &visit {
                Visit::AllRecursively => true,
                Visit::Nothing => false,
                Visit::Specific { dirs, files } => {
                    if components.next().is_some() {
                        matches!(dirs, VisitDirs::All) || matches!(dirs, VisitDirs::Set(d) if d.contains(name))
                    } else {
                        matches!(files, VisitFiles::All) || matches!(files, VisitFiles::Set(f) if f.contains(name))
                    }
                }
            };
            match visit {
                // `AllRecursively` is a promise, the rest are permissions.
                Visit::AllRecursively => assert!(*selected, "{dir:?} promises {file:?}"),
                _ if *selected => assert!(reachable, "{dir:?} hides {file:?}"),
                _ => {}
            }
        }
    }

    #[test]
    fn test_to_path_prefixes() {
        let to_prefixes = |expr: &FilesetExpression| {
            let paths = expr.to_path_prefixes()?;
            Some(paths.iter().map(|path| path.as_internal_file_string()).join(" "))
        };
        let prefix = |path: &str| FilesetExpression::prefix_path(repo_path_buf(path));

        assert_eq!(to_prefixes(&FilesetExpression::none()).as_deref(), Some(""));
        assert_eq!(to_prefixes(&FilesetExpression::all()).as_deref(), Some(""));
        assert_eq!(to_prefixes(&prefix("a")).as_deref(), Some("a"));
        assert_eq!(
            to_prefixes(&FilesetExpression::union_all(vec![prefix("a"), prefix("b")])).as_deref(),
            Some("a b")
        );
        // Anything that isn't a literal prefix has no path-list equivalent
        assert_eq!(to_prefixes(&FilesetExpression::file_path(repo_path_buf("a"))), None);
        assert_eq!(to_prefixes(&prefix("a").difference(prefix("b"))), None);
        assert_eq!(to_prefixes(&prefix("a").intersection(prefix("b"))), None);

        // Round trips through the path list
        let rules = FilesetRules::from_path_prefixes(["", "a", "b/c"].map(repo_path_buf));
        let paths = rules.to_path_prefixes().unwrap();
        assert_eq!(
            paths.iter().map(|path| path.as_internal_file_string()).join(" "),
            " a b/c"
        );
        assert_eq!(
            FilesetRules::from_path_prefixes(paths.into_iter().map(RepoPath::to_owned)),
            rules
        );
        assert_eq!(FilesetRules::all(), FilesetRules::from_path_prefixes([RepoPathBuf::root()]));
        assert!(FilesetRules::none().to_path_prefixes().unwrap().is_empty());

        let mut rules = FilesetRules::all();
        rules.push(FilesetRuleKind::Exclude, prefix("a"));
        assert_eq!(rules.to_path_prefixes(), None);

        // Rules that can never apply again are dropped as they are shadowed
        rules.push(FilesetRuleKind::Include, prefix("a"));
        assert_eq!(rules.to_path_prefixes().unwrap().len(), 2);
        rules.push(FilesetRuleKind::Include, FilesetExpression::all());
        assert_eq!(rules, FilesetRules::all());
        let root = || FilesetExpression::prefix_path(RepoPathBuf::root());
        rules.push(FilesetRuleKind::Exclude, root());
        assert_eq!(rules, FilesetRules::none());
        // Anything matching every path is stored as `all()`
        rules.push(FilesetRuleKind::Include, root());
        assert_eq!(rules, FilesetRules::all());
    }

    #[test]
    fn test_rules_to_expression() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let prefix = |path: &str| FilesetExpression::prefix_path(repo_path_buf(path));
        let to_expression = |kinds: &[FilesetRuleKind]| {
            let mut rules = FilesetRules::none();
            for (i, &kind) in kinds.iter().enumerate() {
                rules.push(kind, prefix(&format!("p{i}")));
            }
            rules.to_expression()
        };
        use FilesetRuleKind::Exclude;
        use FilesetRuleKind::Include;

        assert_eq!(FilesetRules::none().to_expression(), FilesetExpression::None);
        assert_eq!(FilesetRules::all().to_expression(), FilesetExpression::All);
        // An exclusion with nothing selected yet is a no-op
        assert_eq!(to_expression(&[Exclude]), FilesetExpression::None);
        assert_eq!(to_expression(&[Include]), prefix("p0"));
        // Consecutive inclusions stay in one flat union, so the depth grows
        // only with the number of include/exclude alternations.
        insta::assert_debug_snapshot!(
            to_expression(&[Include, Include, Include, Exclude, Include]), @r#"
        UnionAll(
            [
                Difference(
                    UnionAll(
                        [
                            Pattern(PrefixPath("p0")),
                            Pattern(PrefixPath("p1")),
                            Pattern(PrefixPath("p2")),
                        ],
                    ),
                    Pattern(PrefixPath("p3")),
                ),
                Pattern(PrefixPath("p4")),
            ],
        )
        "#);
    }

    #[test]
    fn test_rules_to_matcher() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let prefix = |path: &str| FilesetExpression::prefix_path(repo_path_buf(path));

        // Prefix-only inclusions compile to a single trie
        let rules = FilesetRules::from_path_prefixes(["a", "b"].map(repo_path_buf));
        insta::assert_debug_snapshot!(rules.to_matcher(), @r#"
        PrefixMatcher {
            tree: Dir {
                "a": Prefix {},
                "b": Prefix {},
            },
        }
        "#);
        assert_eq!(
            format!("{:?}", FilesetRules::all().to_matcher()),
            "EverythingMatcher"
        );

        let mut rules = FilesetRules::all();
        rules.push(FilesetRuleKind::Exclude, prefix("build"));
        rules.push(FilesetRuleKind::Include, prefix("build/config"));
        insta::assert_debug_snapshot!(rules.to_matcher(), @r#"
        UnionMatcher {
            input1: DifferenceMatcher {
                wanted: EverythingMatcher,
                unwanted: PrefixMatcher {
                    tree: Dir {
                        "build": Prefix {},
                    },
                },
            },
            input2: PrefixMatcher {
                tree: Dir {
                    "build": Dir {
                        "config": Prefix {},
                    },
                },
            },
        }
        "#);
        let matcher = rules.to_matcher();
        for (path, selected) in [
            ("src/file", true),
            ("build/file", false),
            ("build/config/file", true),
        ] {
            assert_eq!(matcher.matches(&repo_path_buf(path)), selected, "{path}");
        }
        assert_eq!(matcher.visit(&repo_path_buf("src")), Visit::AllRecursively);
        assert_eq!(
            matcher.visit(&repo_path_buf("build/config")),
            Visit::AllRecursively
        );
    }

    #[test]
    fn test_rules_last_matching_rule_wins() {
        let prefix = |path: &str| FilesetExpression::prefix_path(repo_path_buf(path));
        // Atoms deliberately overlap, nest, and include a compound expression
        // whose `visit()` can only answer "maybe".
        let atoms = [
            FilesetExpression::none(),
            FilesetExpression::all(),
            prefix("a"),
            prefix("a/b"),
            FilesetExpression::file_path(repo_path_buf("a/f")),
            prefix("a").difference(prefix("a/b")),
        ];
        let atom_matchers = atoms.iter().map(FilesetExpression::to_matcher).collect_vec();
        let files = ["f", "a/f", "a/b/f", "a/b/c/f", "a/c/f", "b/f"].map(repo_path_buf);
        let dirs = ["", "a", "a/b", "a/b/c", "a/c", "b"].map(repo_path_buf);
        let kinds = [FilesetRuleKind::Include, FilesetRuleKind::Exclude];

        // Exhaustively compare every ordered triple of rules against a direct
        // "last rule that matches a path decides" evaluation.
        for combination in 0..(kinds.len() * atoms.len()).pow(3) {
            let mut remaining = combination;
            let mut rules = FilesetRules::none();
            let mut selected = files.clone().map(|file| (file, false));
            for _ in 0..3 {
                let kind = kinds[remaining % kinds.len()];
                let atom = remaining / kinds.len() % atoms.len();
                remaining /= kinds.len() * atoms.len();
                for (file, selected) in &mut selected {
                    if atom_matchers[atom].matches(file) {
                        *selected = kind == FilesetRuleKind::Include;
                    }
                }
                rules.push(kind, atoms[atom].clone());
            }
            let matcher = rules.to_matcher();
            for (file, selected) in &selected {
                assert_eq!(matcher.matches(file), *selected, "{rules:?}: {file:?}");
            }
            for dir in &dirs {
                assert_visit_agrees(&*matcher, dir, &selected);
            }
        }
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
