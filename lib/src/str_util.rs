// Copyright 2021-2023 The Jujutsu Authors
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

//! String helpers.

use std::borrow::Borrow;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Debug;
use std::iter;
use std::ops::Deref;

use bstr::ByteSlice as _;
use either::Either;
use globset::Glob;
use globset::GlobBuilder;
use thiserror::Error;

/// Error occurred during pattern string parsing.
#[derive(Debug, Error)]
pub enum StringPatternParseError {
    /// Unknown pattern kind is specified.
    #[error("Invalid string pattern kind `{0}:`")]
    InvalidKind(String),
    /// Failed to parse glob pattern.
    #[error(transparent)]
    GlobPattern(globset::Error),
    /// Failed to parse regular expression.
    #[error(transparent)]
    Regex(regex::Error),
}

/// A wrapper for [`Glob`] with a more concise `Debug` impl.
#[derive(Clone)]
pub struct GlobPattern {
    glob: Glob,
}

impl GlobPattern {
    /// Returns the original glob pattern.
    pub fn as_str(&self) -> &str {
        self.glob.glob()
    }

    /// Converts this glob pattern to a bytes regex.
    pub fn to_regex(&self) -> regex::bytes::Regex {
        // Based on new_regex() in globset. We don't use GlobMatcher::is_match(path)
        // because the input string shouldn't be normalized as path.
        regex::bytes::RegexBuilder::new(self.glob.regex())
            .dot_matches_new_line(true)
            .build()
            .expect("glob regex should be valid")
    }
}

impl Debug for GlobPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("GlobPattern").field(&self.as_str()).finish()
    }
}

fn parse_glob(src: &str, icase: bool) -> Result<GlobPattern, StringPatternParseError> {
    let glob = GlobBuilder::new(src)
        .case_insensitive(icase)
        // Don't use platform-dependent default. This pattern isn't meant for
        // testing file-system paths. If backslash escape were disabled, "\" in
        // pattern would be normalized to "/" on Windows.
        .backslash_escape(true)
        .build()
        .map_err(StringPatternParseError::GlobPattern)?;
    Ok(GlobPattern { glob })
}

fn is_glob_char(c: char) -> bool {
    // See globset::escape(). In addition to that, backslash is parsed as an
    // escape sequence on all platforms.
    matches!(c, '?' | '*' | '[' | ']' | '{' | '}' | '\\')
}

/// Pattern to be tested against string property like commit description or
/// bookmark name.
#[derive(Clone, Debug)]
pub enum StringPattern {
    /// Matches strings exactly.
    Exact(String),
    /// Matches strings case‐insensitively.
    ExactI(String),
    /// Matches strings that contain a substring.
    Substring(String),
    /// Matches strings that case‐insensitively contain a substring.
    SubstringI(String),
    /// Matches with a Unix‐style shell wildcard pattern.
    Glob(Box<GlobPattern>),
    /// Matches with a case‐insensitive Unix‐style shell wildcard pattern.
    GlobI(Box<GlobPattern>),
    /// Matches substrings with a regular expression.
    Regex(regex::bytes::Regex),
    /// Matches substrings with a case‐insensitive regular expression.
    RegexI(regex::bytes::Regex),
}

impl StringPattern {
    /// Pattern that matches any string.
    pub const fn all() -> Self {
        Self::Substring(String::new())
    }

    /// Constructs a pattern that matches exactly.
    pub fn exact(src: impl Into<String>) -> Self {
        Self::Exact(src.into())
    }

    /// Constructs a pattern that matches case‐insensitively.
    pub fn exact_i(src: impl Into<String>) -> Self {
        Self::ExactI(src.into())
    }

    /// Constructs a pattern that matches a substring.
    pub fn substring(src: impl Into<String>) -> Self {
        Self::Substring(src.into())
    }

    /// Constructs a pattern that case‐insensitively matches a substring.
    pub fn substring_i(src: impl Into<String>) -> Self {
        Self::SubstringI(src.into())
    }

    /// Parses the given string as a glob pattern.
    pub fn glob(src: &str) -> Result<Self, StringPatternParseError> {
        if !src.contains(is_glob_char) {
            return Ok(Self::exact(src));
        }
        Ok(Self::Glob(Box::new(parse_glob(src, false)?)))
    }

    /// Parses the given string as a case‐insensitive glob pattern.
    pub fn glob_i(src: &str) -> Result<Self, StringPatternParseError> {
        // No special case for !src.contains(is_glob_char) because it's unclear
        // whether we'll use unicode case comparison for "exact-i" patterns.
        // "glob-i" should always be ASCII-based.
        Ok(Self::GlobI(Box::new(parse_glob(src, true)?)))
    }

    /// Parses the given string as a regular expression.
    pub fn regex(src: &str) -> Result<Self, StringPatternParseError> {
        let pattern = regex::bytes::Regex::new(src).map_err(StringPatternParseError::Regex)?;
        Ok(Self::Regex(pattern))
    }

    /// Parses the given string as a case-insensitive regular expression.
    pub fn regex_i(src: &str) -> Result<Self, StringPatternParseError> {
        let pattern = regex::bytes::RegexBuilder::new(src)
            .case_insensitive(true)
            .build()
            .map_err(StringPatternParseError::Regex)?;
        Ok(Self::RegexI(pattern))
    }

    /// Parses the given string as a pattern of the specified `kind`.
    pub fn from_str_kind(src: &str, kind: &str) -> Result<Self, StringPatternParseError> {
        match kind {
            "exact" => Ok(Self::exact(src)),
            "exact-i" => Ok(Self::exact_i(src)),
            "substring" => Ok(Self::substring(src)),
            "substring-i" => Ok(Self::substring_i(src)),
            "glob" => Self::glob(src),
            "glob-i" => Self::glob_i(src),
            "regex" => Self::regex(src),
            "regex-i" => Self::regex_i(src),
            _ => Err(StringPatternParseError::InvalidKind(kind.to_owned())),
        }
    }

    /// Returns true if this pattern trivially matches any input strings.
    fn is_all(&self) -> bool {
        match self {
            Self::Exact(_) | Self::ExactI(_) => false,
            Self::Substring(needle) | Self::SubstringI(needle) => needle.is_empty(),
            Self::Glob(pattern) | Self::GlobI(pattern) => pattern.as_str() == "*",
            Self::Regex(pattern) | Self::RegexI(pattern) => pattern.as_str().is_empty(),
        }
    }

    /// Returns true if this pattern matches input strings exactly.
    pub fn is_exact(&self) -> bool {
        self.as_exact().is_some()
    }

    /// Returns a literal pattern if this should match input strings exactly.
    ///
    /// This can be used to optimize map lookup by exact key.
    pub fn as_exact(&self) -> Option<&str> {
        // TODO: Handle trivial case‐insensitive patterns here? It might make people
        // expect they can use case‐insensitive patterns in contexts where they
        // generally can’t.
        match self {
            Self::Exact(literal) => Some(literal),
            _ => None,
        }
    }

    /// Returns the original string of this pattern.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Exact(literal) => literal,
            Self::ExactI(literal) => literal,
            Self::Substring(needle) => needle,
            Self::SubstringI(needle) => needle,
            Self::Glob(pattern) => pattern.as_str(),
            Self::GlobI(pattern) => pattern.as_str(),
            Self::Regex(pattern) => pattern.as_str(),
            Self::RegexI(pattern) => pattern.as_str(),
        }
    }

    /// Converts this pattern to a glob string. Returns `None` if the pattern
    /// can't be represented as a glob.
    pub fn to_glob(&self) -> Option<Cow<'_, str>> {
        // TODO: Handle trivial case‐insensitive patterns here? It might make people
        // expect they can use case‐insensitive patterns in contexts where they
        // generally can’t.
        match self {
            Self::Exact(literal) => Some(globset::escape(literal).into()),
            Self::Substring(needle) => {
                if needle.is_empty() {
                    Some("*".into())
                } else {
                    Some(format!("*{}*", globset::escape(needle)).into())
                }
            }
            Self::Glob(pattern) => Some(pattern.as_str().into()),
            Self::ExactI(_) => None,
            Self::SubstringI(_) => None,
            Self::GlobI(_) => None,
            Self::Regex(_) => None,
            Self::RegexI(_) => None,
        }
    }

    fn to_match_fn(&self) -> Box<DynMatchFn> {
        // TODO: Unicode case folding is complicated and can be
        // locale‐specific. The `globset` crate and Gitoxide only deal with
        // ASCII case folding, so we do the same here; a more elaborate case
        // folding system will require making sure those behave in a matching
        // manner where relevant. That said, regex patterns are unicode-aware by
        // default, so we already have some inconsistencies.
        //
        // Care will need to be taken regarding normalization and the choice of an
        // appropriate case‐insensitive comparison scheme (`toNFKC_Casefold`?) to ensure
        // that it is compatible with the standard case‐insensitivity of haystack
        // components (like internationalized domain names in email addresses). The
        // availability of normalization and case folding schemes in database backends
        // will also need to be considered. A locale‐specific case folding
        // scheme would likely not be appropriate for Jujutsu.
        //
        // For some discussion of this topic, see:
        // <https://github.com/unicode-org/icu4x/issues/3151>
        match self {
            Self::Exact(literal) => {
                let literal = literal.clone();
                Box::new(move |haystack| haystack == literal.as_bytes())
            }
            Self::ExactI(literal) => {
                let literal = literal.clone();
                Box::new(move |haystack| haystack.eq_ignore_ascii_case(literal.as_bytes()))
            }
            Self::Substring(needle) => {
                let needle = needle.clone();
                Box::new(move |haystack| haystack.contains_str(&needle))
            }
            Self::SubstringI(needle) => {
                let needle = needle.to_ascii_lowercase();
                Box::new(move |haystack| haystack.to_ascii_lowercase().contains_str(&needle))
            }
            // (Glob, GlobI) and (Regex, RegexI) pairs are identical here, but
            // callers might want to translate these to backend-specific query
            // differently.
            Self::Glob(pattern) | Self::GlobI(pattern) => {
                let pattern = pattern.to_regex();
                Box::new(move |haystack| pattern.is_match(haystack))
            }
            Self::Regex(pattern) | Self::RegexI(pattern) => {
                let pattern = pattern.clone();
                Box::new(move |haystack| pattern.is_match(haystack))
            }
        }
    }

    /// Creates matcher object from this pattern.
    pub fn to_matcher(&self) -> StringMatcher {
        if self.is_all() {
            StringMatcher::All
        } else if let Some(literal) = self.as_exact() {
            StringMatcher::Exact(literal.to_owned())
        } else {
            StringMatcher::Fn(self.to_match_fn())
        }
    }

    /// Converts the pattern into a bytes regex.
    pub fn to_regex(&self) -> regex::bytes::Regex {
        match self {
            Self::Exact(literal) => {
                regex::bytes::RegexBuilder::new(&format!("^{}$", regex::escape(literal)))
                    .build()
                    .expect("impossible to fail to compile regex of literal")
            }
            Self::ExactI(literal) => {
                regex::bytes::RegexBuilder::new(&format!("^{}$", regex::escape(literal)))
                    .case_insensitive(true)
                    .build()
                    .expect("impossible to fail to compile regex of literal")
            }
            Self::Substring(literal) => regex::bytes::RegexBuilder::new(&regex::escape(literal))
                .build()
                .expect("impossible to fail to compile regex of literal"),
            Self::SubstringI(literal) => regex::bytes::RegexBuilder::new(&regex::escape(literal))
                .case_insensitive(true)
                .build()
                .expect("impossible to fail to compile regex of literal"),
            Self::Glob(glob_pattern) => glob_pattern.to_regex(),
            // The regex generated represents the case insensitivity itself
            Self::GlobI(glob_pattern) => glob_pattern.to_regex(),
            Self::Regex(regex) => regex.clone(),
            Self::RegexI(regex) => regex.clone(),
        }
    }
}

impl fmt::Display for StringPattern {
    /// Shows the original string of this pattern.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// AST-level representation of the string matcher expression.
#[derive(Clone, Debug)]
pub enum StringExpression {
    // None and All can be represented by using Pattern. Add them if needed.
    /// Matches pattern.
    Pattern(Box<StringPattern>),
    /// Matches anything other than the expression.
    NotIn(Box<Self>),
    /// Matches one of the expressions.
    Union(Box<Self>, Box<Self>),
    /// Matches both expressions.
    Intersection(Box<Self>, Box<Self>),
}

impl StringExpression {
    /// Expression that matches nothing.
    pub fn none() -> Self {
        Self::all().negated()
    }

    /// Expression that matches everything.
    pub fn all() -> Self {
        Self::pattern(StringPattern::all())
    }

    /// Expression that matches the given pattern.
    pub fn pattern(pattern: StringPattern) -> Self {
        Self::Pattern(Box::new(pattern))
    }

    /// Expression that matches strings exactly.
    pub fn exact(src: impl Into<String>) -> Self {
        Self::pattern(StringPattern::exact(src))
    }

    /// Expression that matches substrings.
    pub fn substring(src: impl Into<String>) -> Self {
        Self::pattern(StringPattern::substring(src))
    }

    /// Expression that matches anything other than this expression.
    pub fn negated(self) -> Self {
        Self::NotIn(Box::new(self))
    }

    /// Expression that matches `self` or `other` (or both).
    pub fn union(self, other: Self) -> Self {
        Self::Union(Box::new(self), Box::new(other))
    }

    /// Expression that matches any of the given `expressions`.
    pub fn union_all(expressions: Vec<Self>) -> Self {
        to_binary_expression(expressions, &Self::none, &Self::union)
    }

    /// Expression that matches both `self` and `other`.
    pub fn intersection(self, other: Self) -> Self {
        Self::Intersection(Box::new(self), Box::new(other))
    }

    fn dfs_pre(&self) -> impl Iterator<Item = &Self> {
        let mut stack: Vec<&Self> = vec![self];
        iter::from_fn(move || {
            let expr = stack.pop()?;
            match expr {
                Self::Pattern(_) => {}
                Self::NotIn(expr) => stack.push(expr),
                Self::Union(expr1, expr2) | Self::Intersection(expr1, expr2) => {
                    stack.push(expr2);
                    stack.push(expr1);
                }
            }
            Some(expr)
        })
    }

    /// Iterates exact string patterns recursively from this expression.
    ///
    /// For example, `"a", "b", "c"` will be yielded in that order for
    /// expression `"a" | glob:"?" & "b" | ~"c"`.
    pub fn exact_strings(&self) -> impl Iterator<Item = &str> {
        // pre/post-ordering doesn't matter so long as children are visited from
        // left to right.
        self.dfs_pre().filter_map(|expr| match expr {
            Self::Pattern(pattern) => pattern.as_exact(),
            _ => None,
        })
    }

    /// Transforms the expression tree to matcher object.
    pub fn to_matcher(&self) -> StringMatcher {
        match self {
            Self::Pattern(pattern) => pattern.to_matcher(),
            Self::NotIn(expr) => {
                let p = expr.to_matcher().into_match_fn();
                StringMatcher::Fn(Box::new(move |haystack| !p(haystack)))
            }
            Self::Union(expr1, expr2) => {
                let p1 = expr1.to_matcher().into_match_fn();
                let p2 = expr2.to_matcher().into_match_fn();
                StringMatcher::Fn(Box::new(move |haystack| p1(haystack) || p2(haystack)))
            }
            Self::Intersection(expr1, expr2) => {
                let p1 = expr1.to_matcher().into_match_fn();
                let p2 = expr2.to_matcher().into_match_fn();
                StringMatcher::Fn(Box::new(move |haystack| p1(haystack) && p2(haystack)))
            }
        }
    }
}

/// Constructs binary tree from `expressions` list, `unit` node, and associative
/// `binary` operation.
fn to_binary_expression<T>(
    expressions: Vec<T>,
    unit: &impl Fn() -> T,
    binary: &impl Fn(T, T) -> T,
) -> T {
    match expressions.len() {
        0 => unit(),
        1 => expressions.into_iter().next().unwrap(),
        _ => {
            // Build balanced tree to minimize the recursion depth.
            let mut left = expressions;
            let right = left.split_off(left.len() / 2);
            binary(
                to_binary_expression(left, unit, binary),
                to_binary_expression(right, unit, binary),
            )
        }
    }
}

type DynMatchFn = dyn Fn(&[u8]) -> bool;

/// Matcher for strings and bytes.
pub enum StringMatcher {
    /// Matches any strings.
    All,
    /// Matches strings exactly.
    Exact(String),
    /// Tests matches by arbitrary function.
    Fn(Box<DynMatchFn>),
}

impl StringMatcher {
    /// Matcher that matches any strings.
    pub const fn all() -> Self {
        Self::All
    }

    /// Matcher that matches `src` exactly.
    pub fn exact(src: impl Into<String>) -> Self {
        Self::Exact(src.into())
    }

    /// Returns true if this matches the `haystack` string.
    pub fn is_match(&self, haystack: &str) -> bool {
        self.is_match_bytes(haystack.as_bytes())
    }

    /// Returns true if this matches the `haystack` bytes.
    pub fn is_match_bytes(&self, haystack: &[u8]) -> bool {
        match self {
            Self::All => true,
            Self::Exact(needle) => haystack == needle.as_bytes(),
            Self::Fn(predicate) => predicate(haystack),
        }
    }

    fn into_match_fn(self) -> Box<DynMatchFn> {
        match self {
            Self::All => Box::new(|_haystack| true),
            Self::Exact(needle) => Box::new(move |haystack| haystack == needle.as_bytes()),
            Self::Fn(predicate) => predicate,
        }
    }

    /// Iterates entries of the given `map` whose string keys match this.
    pub fn filter_btree_map<'a, K: Borrow<str> + Ord, V>(
        &self,
        map: &'a BTreeMap<K, V>,
    ) -> impl Iterator<Item = (&'a K, &'a V)> {
        self.filter_btree_map_with(map, |key| key, |key| key)
    }

    /// Iterates entries of the given `map` whose string-like keys match this.
    ///
    /// The borrowed key type is constrained by the `Deref::Target`. It must be
    /// convertible to/from `str`.
    pub fn filter_btree_map_as_deref<'a, K, V>(
        &self,
        map: &'a BTreeMap<K, V>,
    ) -> impl Iterator<Item = (&'a K, &'a V)>
    where
        K: Borrow<K::Target> + Deref + Ord,
        K::Target: AsRef<str> + Ord,
        str: AsRef<K::Target>,
    {
        self.filter_btree_map_with(map, AsRef::as_ref, AsRef::as_ref)
    }

    fn filter_btree_map_with<'a, K, Q, V>(
        &self,
        map: &'a BTreeMap<K, V>,
        from_key: impl Fn(&Q) -> &str,
        to_key: impl Fn(&str) -> &Q,
    ) -> impl Iterator<Item = (&'a K, &'a V)>
    where
        K: Borrow<Q> + Ord,
        Q: Ord + ?Sized,
    {
        match self {
            Self::All => Either::Left(map.iter()),
            Self::Exact(key) => {
                Either::Right(Either::Left(map.get_key_value(to_key(key)).into_iter()))
            }
            Self::Fn(predicate) => {
                Either::Right(Either::Right(map.iter().filter(move |&(key, _)| {
                    predicate(from_key(key.borrow()).as_bytes())
                })))
            }
        }
    }
}

impl Debug for StringMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => write!(f, "All"),
            Self::Exact(needle) => f.debug_tuple("Exact").field(needle).finish(),
            Self::Fn(_) => f.debug_tuple("Fn").finish_non_exhaustive(),
        }
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use itertools::Itertools as _;
    use maplit::btreemap;

    use super::*;

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
    fn test_string_pattern_to_glob() {
        assert_eq!(StringPattern::all().to_glob(), Some("*".into()));
        assert_eq!(StringPattern::exact("a").to_glob(), Some("a".into()));
        assert_eq!(StringPattern::exact("*").to_glob(), Some("[*]".into()));
        assert_eq!(
            StringPattern::glob("*").unwrap().to_glob(),
            Some("*".into())
        );
        assert_eq!(
            StringPattern::Substring("a".into()).to_glob(),
            Some("*a*".into())
        );
        assert_eq!(
            StringPattern::Substring("*".into()).to_glob(),
            Some("*[*]*".into())
        );
    }

    #[test]
    fn test_parse() {
        // Parse specific pattern kinds.
        assert_matches!(
            StringPattern::from_str_kind("foo", "exact"),
            Ok(StringPattern::Exact(s)) if s == "foo"
        );
        assert_matches!(
            StringPattern::from_str_kind("foo*", "glob"),
            Ok(StringPattern::Glob(p)) if p.as_str() == "foo*"
        );
        assert_matches!(
            StringPattern::from_str_kind("foo", "substring"),
            Ok(StringPattern::Substring(s)) if s == "foo"
        );
        assert_matches!(
            StringPattern::from_str_kind("foo", "substring-i"),
            Ok(StringPattern::SubstringI(s)) if s == "foo"
        );
        assert_matches!(
            StringPattern::from_str_kind("foo", "regex"),
            Ok(StringPattern::Regex(p)) if p.as_str() == "foo"
        );
        assert_matches!(
            StringPattern::from_str_kind("foo", "regex-i"),
            Ok(StringPattern::RegexI(p)) if p.as_str() == "foo"
        );
    }

    #[test]
    fn test_glob_is_match() {
        let glob = |src: &str| StringPattern::glob(src).unwrap().to_matcher();
        let glob_i = |src: &str| StringPattern::glob_i(src).unwrap().to_matcher();

        assert!(glob("foo").is_match("foo"));
        assert!(!glob("foo").is_match("foobar"));

        // "." in string isn't any special
        assert!(glob("*").is_match(".foo"));

        // "/" in string isn't any special
        assert!(glob("*").is_match("foo/bar"));
        assert!(glob(r"*/*").is_match("foo/bar"));
        assert!(!glob(r"*/*").is_match(r"foo\bar"));

        // "\" is an escape character
        assert!(!glob(r"*\*").is_match("foo/bar"));
        assert!(glob(r"*\*").is_match("foo*"));
        assert!(glob(r"\\").is_match(r"\"));

        // "*" matches newline
        assert!(glob(r"*").is_match("foo\nbar"));

        assert!(!glob("f?O").is_match("Foo"));
        assert!(glob_i("f?O").is_match("Foo"));
    }

    #[test]
    fn test_regex_is_match() {
        let regex = |src: &str| StringPattern::regex(src).unwrap().to_matcher();
        // Unicode mode is enabled by default
        assert!(regex(r"^\w$").is_match("\u{c0}"));
        assert!(regex(r"^.$").is_match("\u{c0}"));
        // ASCII-compatible mode should also work
        assert!(regex(r"^(?-u)\w$").is_match("a"));
        assert!(!regex(r"^(?-u)\w$").is_match("\u{c0}"));
        assert!(regex(r"^(?-u).{2}$").is_match("\u{c0}"));
    }

    #[test]
    fn test_string_pattern_to_regex() {
        let check = |pattern: StringPattern, match_to: &str| {
            let regex = pattern.to_regex();
            regex.is_match(match_to.as_bytes())
        };
        assert!(check(StringPattern::exact("$a"), "$a"));
        assert!(!check(StringPattern::exact("$a"), "$A"));
        assert!(!check(StringPattern::exact("a"), "aa"));
        assert!(!check(StringPattern::exact("a"), "aa"));
        assert!(check(StringPattern::exact_i("a"), "A"));
        assert!(check(StringPattern::substring("$a"), "$abc"));
        assert!(!check(StringPattern::substring("$a"), "$Abc"));
        assert!(check(StringPattern::substring_i("$a"), "$Abc"));
        assert!(!check(StringPattern::glob("a").unwrap(), "A"));
        assert!(check(StringPattern::glob_i("a").unwrap(), "A"));
        assert!(check(StringPattern::regex("^a{1,3}").unwrap(), "abcde"));
        assert!(!check(StringPattern::regex("^a{1,3}").unwrap(), "Abcde"));
        assert!(check(StringPattern::regex_i("^a{1,3}").unwrap(), "Abcde"));
    }

    #[test]
    fn test_exact_pattern_to_matcher() {
        assert_matches!(
            StringPattern::exact("").to_matcher(),
            StringMatcher::Exact(needle) if needle.is_empty()
        );
        assert_matches!(
            StringPattern::exact("x").to_matcher(),
            StringMatcher::Exact(needle) if needle == "x"
        );

        assert_matches!(
            StringPattern::exact_i("").to_matcher(),
            StringMatcher::Fn(_) // or Exact
        );
        assert_matches!(
            StringPattern::exact_i("x").to_matcher(),
            StringMatcher::Fn(_)
        );
    }

    #[test]
    fn test_substring_pattern_to_matcher() {
        assert_matches!(
            StringPattern::substring("").to_matcher(),
            StringMatcher::All
        );
        assert_matches!(
            StringPattern::substring("x").to_matcher(),
            StringMatcher::Fn(_)
        );

        assert_matches!(
            StringPattern::substring_i("").to_matcher(),
            StringMatcher::All
        );
        assert_matches!(
            StringPattern::substring_i("x").to_matcher(),
            StringMatcher::Fn(_)
        );
    }

    #[test]
    fn test_glob_pattern_to_matcher() {
        assert_matches!(
            StringPattern::glob("").unwrap().to_matcher(),
            StringMatcher::Exact(_)
        );
        assert_matches!(
            StringPattern::glob("x").unwrap().to_matcher(),
            StringMatcher::Exact(_)
        );
        assert_matches!(
            StringPattern::glob("x?").unwrap().to_matcher(),
            StringMatcher::Fn(_)
        );
        assert_matches!(
            StringPattern::glob("*").unwrap().to_matcher(),
            StringMatcher::All
        );
        assert_matches!(
            StringPattern::glob(r"\\").unwrap().to_matcher(),
            StringMatcher::Fn(_) // or Exact(r"\")
        );

        assert_matches!(
            StringPattern::glob_i("").unwrap().to_matcher(),
            StringMatcher::Fn(_) // or Exact
        );
        assert_matches!(
            StringPattern::glob_i("x").unwrap().to_matcher(),
            StringMatcher::Fn(_)
        );
        assert_matches!(
            StringPattern::glob_i("x?").unwrap().to_matcher(),
            StringMatcher::Fn(_)
        );
        assert_matches!(
            StringPattern::glob_i("*").unwrap().to_matcher(),
            StringMatcher::All
        );
    }

    #[test]
    fn test_regex_pattern_to_matcher() {
        assert_matches!(
            StringPattern::regex("").unwrap().to_matcher(),
            StringMatcher::All
        );
        assert_matches!(
            StringPattern::regex("x").unwrap().to_matcher(),
            StringMatcher::Fn(_)
        );
        assert_matches!(
            StringPattern::regex(".").unwrap().to_matcher(),
            StringMatcher::Fn(_)
        );

        assert_matches!(
            StringPattern::regex_i("").unwrap().to_matcher(),
            StringMatcher::All
        );
        assert_matches!(
            StringPattern::regex_i("x").unwrap().to_matcher(),
            StringMatcher::Fn(_)
        );
        assert_matches!(
            StringPattern::regex_i(".").unwrap().to_matcher(),
            StringMatcher::Fn(_)
        );
    }

    #[test]
    fn test_union_all_expressions() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(
            StringExpression::union_all(vec![]),
            @r#"NotIn(Pattern(Substring("")))"#);
        insta::assert_debug_snapshot!(
            StringExpression::union_all(vec![StringExpression::exact("a")]),
            @r#"Pattern(Exact("a"))"#);
        insta::assert_debug_snapshot!(
            StringExpression::union_all(vec![
                StringExpression::exact("a"),
                StringExpression::exact("b"),
            ]),
            @r#"
        Union(
            Pattern(Exact("a")),
            Pattern(Exact("b")),
        )
        "#);
        insta::assert_debug_snapshot!(
            StringExpression::union_all(vec![
                StringExpression::exact("a"),
                StringExpression::exact("b"),
                StringExpression::exact("c"),
            ]),
            @r#"
        Union(
            Pattern(Exact("a")),
            Union(
                Pattern(Exact("b")),
                Pattern(Exact("c")),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            StringExpression::union_all(vec![
                StringExpression::exact("a"),
                StringExpression::exact("b"),
                StringExpression::exact("c"),
                StringExpression::exact("d"),
            ]),
            @r#"
        Union(
            Union(
                Pattern(Exact("a")),
                Pattern(Exact("b")),
            ),
            Union(
                Pattern(Exact("c")),
                Pattern(Exact("d")),
            ),
        )
        "#);
    }

    #[test]
    fn test_exact_strings_in_expression() {
        assert_eq!(
            StringExpression::all().exact_strings().collect_vec(),
            [""; 0]
        );
        assert_eq!(
            StringExpression::union_all(vec![
                StringExpression::exact("a"),
                StringExpression::substring("b"),
                StringExpression::intersection(
                    StringExpression::exact("c"),
                    StringExpression::exact("d").negated(),
                ),
            ])
            .exact_strings()
            .collect_vec(),
            ["a", "c", "d"]
        );
    }

    #[test]
    fn test_trivial_expression_to_matcher() {
        assert_matches!(StringExpression::all().to_matcher(), StringMatcher::All);
        assert_matches!(
            StringExpression::exact("x").to_matcher(),
            StringMatcher::Exact(needle) if needle == "x"
        );
    }

    #[test]
    fn test_compound_expression_to_matcher() {
        let matcher = StringExpression::exact("foo").negated().to_matcher();
        assert!(!matcher.is_match("foo"));
        assert!(matcher.is_match("bar"));

        let matcher = StringExpression::union(
            StringExpression::exact("foo"),
            StringExpression::exact("bar"),
        )
        .to_matcher();
        assert!(matcher.is_match("foo"));
        assert!(matcher.is_match("bar"));
        assert!(!matcher.is_match("baz"));

        let matcher = StringExpression::intersection(
            StringExpression::substring("a"),
            StringExpression::substring("r"),
        )
        .to_matcher();
        assert!(!matcher.is_match("foo"));
        assert!(matcher.is_match("bar"));
        assert!(!matcher.is_match("baz"));
    }

    #[test]
    fn test_matcher_is_match() {
        assert!(StringMatcher::all().is_match(""));
        assert!(StringMatcher::all().is_match("foo"));
        assert!(!StringMatcher::exact("o").is_match(""));
        assert!(!StringMatcher::exact("o").is_match("foo"));
        assert!(StringMatcher::exact("foo").is_match("foo"));
        assert!(StringPattern::substring("o").to_matcher().is_match("foo"));
    }

    #[test]
    fn test_matcher_filter_btree_map() {
        let data = btreemap! {
            "bar" => (),
            "baz" => (),
            "foo" => (),
        };
        let filter = |matcher: &StringMatcher| {
            matcher
                .filter_btree_map(&data)
                .map(|(&key, ())| key)
                .collect_vec()
        };
        assert_eq!(filter(&StringMatcher::all()), vec!["bar", "baz", "foo"]);
        assert_eq!(filter(&StringMatcher::exact("o")), vec![""; 0]);
        assert_eq!(filter(&StringMatcher::exact("foo")), vec!["foo"]);
        assert_eq!(
            filter(&StringPattern::substring("o").to_matcher()),
            vec!["foo"]
        );
        assert_eq!(
            filter(&StringPattern::substring("a").to_matcher()),
            vec!["bar", "baz"]
        );
    }
}
