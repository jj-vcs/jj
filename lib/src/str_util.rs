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

/// A wrapper for [`Glob`] and its matcher with a more concise `Debug` impl.
#[derive(Clone)]
pub struct GlobPattern {
    glob: Glob,
    // TODO: Maybe better to add StringPattern::to_matcher(), and move regex
    // compilation there.
    regex: regex::bytes::Regex,
}

impl GlobPattern {
    /// Returns true if this pattern matches `haystack`.
    pub fn is_match(&self, haystack: &[u8]) -> bool {
        self.regex.is_match(haystack)
    }

    /// Returns the original glob pattern.
    pub fn as_str(&self) -> &str {
        self.glob.glob()
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
    // Based on new_regex() in globset. We don't use GlobMatcher::is_match(path)
    // because the input string shouldn't be normalized as path.
    let regex = regex::bytes::RegexBuilder::new(glob.regex())
        .dot_matches_new_line(true)
        .build()
        .expect("glob regex should be valid");
    Ok(GlobPattern { glob, regex })
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
    pub const fn everything() -> Self {
        Self::Substring(String::new())
    }

    /// Parses the given string as a [`StringPattern`]. Everything before the
    /// first ":" is considered the string's prefix. If the prefix is
    /// "exact[-i]:", "glob[-i]:", or "substring[-i]:", a pattern of the
    /// specified kind is returned. Returns an error if the string has an
    /// unrecognized prefix. Otherwise, a `StringPattern::Exact` is
    /// returned.
    pub fn parse(src: &str) -> Result<Self, StringPatternParseError> {
        if let Some((kind, pat)) = src.split_once(':') {
            Self::from_str_kind(pat, kind)
        } else {
            Ok(Self::exact(src))
        }
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
        // TODO: if no meta character found, it can be mapped to Exact.
        Ok(Self::Glob(Box::new(parse_glob(src, false)?)))
    }

    /// Parses the given string as a case‐insensitive glob pattern.
    pub fn glob_i(src: &str) -> Result<Self, StringPatternParseError> {
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

    /// Returns true if this pattern matches the `haystack` string.
    ///
    /// When matching against a case‐insensitive pattern, only ASCII case
    /// differences are currently folded. This may change in the future.
    pub fn is_match(&self, haystack: &str) -> bool {
        self.is_match_bytes(haystack.as_bytes())
    }

    /// Returns true if this pattern matches the `haystack` bytes.
    pub fn is_match_bytes(&self, haystack: &[u8]) -> bool {
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
            Self::Exact(literal) => haystack == literal.as_bytes(),
            Self::ExactI(literal) => haystack.eq_ignore_ascii_case(literal.as_bytes()),
            Self::Substring(needle) => haystack.contains_str(needle),
            Self::SubstringI(needle) => haystack
                .to_ascii_lowercase()
                .contains_str(needle.to_ascii_lowercase()),
            // (Glob, GlobI) and (Regex, RegexI) pairs are identical here, but
            // callers might want to translate these to backend-specific query
            // differently.
            Self::Glob(pattern) => pattern.is_match(haystack),
            Self::GlobI(pattern) => pattern.is_match(haystack),
            Self::Regex(pattern) => pattern.is_match(haystack),
            Self::RegexI(pattern) => pattern.is_match(haystack),
        }
    }

    /// Iterates entries of the given `map` whose string keys match this
    /// pattern.
    pub fn filter_btree_map<'a, 'b, K: Borrow<str> + Ord, V>(
        &'b self,
        map: &'a BTreeMap<K, V>,
    ) -> impl Iterator<Item = (&'a K, &'a V)> + use<'a, 'b, K, V> {
        self.filter_btree_map_with(map, |key| key, |key| key)
    }

    /// Iterates entries of the given `map` whose string-like keys match this
    /// pattern.
    ///
    /// The borrowed key type is constrained by the `Deref::Target`. It must be
    /// convertible to/from `str`.
    pub fn filter_btree_map_as_deref<'a, 'b, K, V>(
        &'b self,
        map: &'a BTreeMap<K, V>,
    ) -> impl Iterator<Item = (&'a K, &'a V)> + use<'a, 'b, K, V>
    where
        K: Borrow<K::Target> + Deref + Ord,
        K::Target: AsRef<str> + Ord,
        str: AsRef<K::Target>,
    {
        self.filter_btree_map_with(map, AsRef::as_ref, AsRef::as_ref)
    }

    fn filter_btree_map_with<'a, 'b, K, Q, V, FromKey, ToKey>(
        &'b self,
        map: &'a BTreeMap<K, V>,
        from_key: FromKey,
        to_key: ToKey,
        // TODO: Q, FromKey, and ToKey don't have to be captured, but
        // "currently, all type parameters are required to be mentioned in the
        // precise captures list" as of rustc 1.85.0.
    ) -> impl Iterator<Item = (&'a K, &'a V)> + use<'a, 'b, K, Q, V, FromKey, ToKey>
    where
        K: Borrow<Q> + Ord,
        Q: Ord + ?Sized,
        FromKey: Fn(&Q) -> &str,
        ToKey: Fn(&str) -> &Q,
    {
        if let Some(key) = self.as_exact() {
            Either::Left(map.get_key_value(to_key(key)).into_iter())
        } else {
            Either::Right(
                map.iter()
                    .filter(move |&(key, _)| self.is_match(from_key(key.borrow()))),
            )
        }
    }
}

impl fmt::Display for StringPattern {
    /// Shows the original string of this pattern.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;

    use super::*;

    #[test]
    fn test_string_pattern_to_glob() {
        assert_eq!(StringPattern::everything().to_glob(), Some("*".into()));
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
            StringPattern::parse("exact:foo"),
            Ok(StringPattern::Exact(s)) if s == "foo"
        );
        assert_matches!(
            StringPattern::from_str_kind("foo", "exact"),
            Ok(StringPattern::Exact(s)) if s == "foo"
        );
        assert_matches!(
            StringPattern::parse("glob:foo*"),
            Ok(StringPattern::Glob(p)) if p.as_str() == "foo*"
        );
        assert_matches!(
            StringPattern::from_str_kind("foo*", "glob"),
            Ok(StringPattern::Glob(p)) if p.as_str() == "foo*"
        );
        assert_matches!(
            StringPattern::parse("substring:foo"),
            Ok(StringPattern::Substring(s)) if s == "foo"
        );
        assert_matches!(
            StringPattern::from_str_kind("foo", "substring"),
            Ok(StringPattern::Substring(s)) if s == "foo"
        );
        assert_matches!(
            StringPattern::parse("substring-i:foo"),
            Ok(StringPattern::SubstringI(s)) if s == "foo"
        );
        assert_matches!(
            StringPattern::from_str_kind("foo", "substring-i"),
            Ok(StringPattern::SubstringI(s)) if s == "foo"
        );
        assert_matches!(
            StringPattern::parse("regex:foo"),
            Ok(StringPattern::Regex(p)) if p.as_str() == "foo"
        );
        assert_matches!(
            StringPattern::from_str_kind("foo", "regex"),
            Ok(StringPattern::Regex(p)) if p.as_str() == "foo"
        );
        assert_matches!(
            StringPattern::parse("regex-i:foo"),
            Ok(StringPattern::RegexI(p)) if p.as_str() == "foo"
        );
        assert_matches!(
            StringPattern::from_str_kind("foo", "regex-i"),
            Ok(StringPattern::RegexI(p)) if p.as_str() == "foo"
        );

        // Parse a pattern that contains a : itself.
        assert_matches!(
            StringPattern::parse("exact:foo:bar"),
            Ok(StringPattern::Exact(s)) if s == "foo:bar"
        );

        // If no kind is specified, the input is treated as an exact pattern.
        assert_matches!(
            StringPattern::parse("foo"),
            Ok(StringPattern::Exact(s)) if s == "foo"
        );

        // Parsing an unknown prefix results in an error.
        assert_matches!(
            StringPattern::parse("unknown-prefix:foo"),
            Err(StringPatternParseError::InvalidKind(_))
        );
    }

    #[test]
    fn test_glob_is_match() {
        assert!(StringPattern::glob("foo").unwrap().is_match("foo"));
        assert!(!StringPattern::glob("foo").unwrap().is_match("foobar"));

        // "." in string isn't any special
        assert!(StringPattern::glob("*").unwrap().is_match(".foo"));

        // "/" in string isn't any special
        assert!(StringPattern::glob("*").unwrap().is_match("foo/bar"));
        assert!(StringPattern::glob(r"*/*").unwrap().is_match("foo/bar"));
        assert!(!StringPattern::glob(r"*/*").unwrap().is_match(r"foo\bar"));

        // "\" is an escape character
        assert!(!StringPattern::glob(r"*\*").unwrap().is_match("foo/bar"));
        assert!(StringPattern::glob(r"*\*").unwrap().is_match("foo*"));
        assert!(StringPattern::glob(r"\\").unwrap().is_match(r"\"));

        // "*" matches newline
        assert!(StringPattern::glob(r"*").unwrap().is_match("foo\nbar"));

        assert!(!StringPattern::glob("f?O").unwrap().is_match("Foo"));
        assert!(StringPattern::glob_i("f?O").unwrap().is_match("Foo"));
    }

    #[test]
    fn test_regex_is_match() {
        // Unicode mode is enabled by default
        assert!(StringPattern::regex(r"^\w$").unwrap().is_match("\u{c0}"));
        assert!(StringPattern::regex(r"^.$").unwrap().is_match("\u{c0}"));
        // ASCII-compatible mode should also work
        assert!(StringPattern::regex(r"^(?-u)\w$").unwrap().is_match("a"));
        assert!(
            !StringPattern::regex(r"^(?-u)\w$")
                .unwrap()
                .is_match("\u{c0}")
        );
        assert!(
            StringPattern::regex(r"^(?-u).{2}$")
                .unwrap()
                .is_match("\u{c0}")
        );
    }
}
