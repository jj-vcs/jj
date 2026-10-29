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

use std::io;
use std::ops::Range;

use bstr::BStr;
use gix_diff::blob::hunk_header::Definition;
use gix_diff::blob::hunk_header::Matcher;
use jj_lib::repo_path::RepoPath;

#[cfg(feature = "git")]
pub(crate) mod git;

/// Gitoxide matchers selected for a file, including ambiguous filename extensions.
pub(crate) struct SourceSymbolMatcher {
    matchers: Vec<Matcher>,
}

impl SourceSymbolMatcher {
    /// Keep filename convenience rules in the UI; Gitoxide owns language matching.
    pub(crate) fn for_file_name(file_name: &str) -> io::Result<Self> {
        let extension = file_name.rsplit_once('.').map(|(_, extension)| extension);
        let names: &[&str] = match extension {
            Some("adb" | "ads") => &["ada"],
            Some("bib") => &["bibtex"],
            Some("c" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx") => &["cpp"],
            Some("h" | "mm") => &["cpp", "objc"],
            Some("cs") => &["csharp"],
            Some("css") => &["css"],
            Some("dts" | "dtsi") => &["dts"],
            Some("ex" | "exs") => &["elixir"],
            Some("f" | "for" | "f90" | "f95" | "f03" | "f08") => &["fortran"],
            Some("fountain") => &["fountain"],
            Some("go") => &["golang"],
            Some("htm" | "html" | "xhtml") => &["html"],
            Some("cfg" | "conf" | "config" | "ini") => &["ini"],
            Some("java") => &["java"],
            Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx") => &["javascript"],
            Some("kt" | "kts") => &["kotlin"],
            Some("markdown" | "md" | "mdown") => &["markdown"],
            Some("m") => &["matlab", "objc"],
            Some("matlab") => &["matlab"],
            Some("p" | "pas") => &["pascal"],
            Some("pl" | "pm" | "pod" | "psgi" | "t") => &["perl"],
            Some("php" | "php3" | "php4" | "php5" | "phtml") => &["php"],
            Some("py" | "pyw") => &["python"],
            Some("r" | "R") => &["r"],
            Some("rb" | "rake" | "gemspec") => &["ruby"],
            Some("rs") => &["rust"],
            Some("scm" | "scheme" | "ss" | "lisp" | "lsp") => &["scheme"],
            Some("sh" | "bash" | "zsh" | "fish") => &["bash"],
            Some("swift") => &["swift"],
            Some("tex" | "ltx" | "latex") => &["tex"],
            _ => match file_name {
                "Gemfile" | "Rakefile" => &["ruby"],
                _ => &[],
            },
        };
        if names.is_empty() {
            return Ok(Matcher::default().into());
        }
        let matchers = names
            .iter()
            .map(|name| {
                Definition::builtin(name)
                    .expect("filename rules name built-in Gitoxide drivers")
                    .to_matcher()
                    .map_err(io::Error::other)
            })
            .collect::<io::Result<_>>()?;
        Ok(Self { matchers })
    }

    /// Explicit attributes take precedence over filename convenience rules.
    pub(crate) fn for_path(
        path: &RepoPath,
        #[cfg(feature = "git")] attributes: Option<&mut gix::AttributeStack<'_>>,
    ) -> io::Result<Option<Self>> {
        #[cfg(feature = "git")]
        if let Some(attributes) = attributes {
            let selected = attributes
                .diff_driver(path.as_internal_file_string(), None)
                .map_err(io::Error::other)?;
            if selected.attribute != gix::attrs::State::Unspecified {
                return selected
                    .hunk_header()
                    .map(|matcher| matcher.map(Self::from))
                    .map_err(io::Error::other);
            }
        }
        Self::for_file_name(
            path.components()
                .next_back()
                .map_or("", |name| name.as_internal_str()),
        )
        .map(Some)
    }

    pub(crate) fn find<'a>(&self, line: &'a [u8]) -> io::Result<Option<&'a [u8]>> {
        for matcher in &self.matchers {
            if let Some(symbol) = matcher.find(line).map_err(io::Error::other)? {
                return Ok(Some(symbol));
            }
        }
        Ok(None)
    }
}

impl From<Matcher> for SourceSymbolMatcher {
    fn from(matcher: Matcher) -> Self {
        Self {
            matchers: vec![matcher],
        }
    }
}

/// Finds source symbols near line ranges in one version of a file.
///
/// A scanner operates on a single file content so it can also be used for
/// diffs with more than two sides.
pub(crate) struct SourceSymbolScanner<'a> {
    symbols: Vec<(usize, &'a [u8])>,
}

impl<'a> SourceSymbolScanner<'a> {
    pub(crate) fn new(matcher: &SourceSymbolMatcher, content: &'a BStr) -> io::Result<Self> {
        let mut symbols = Vec::new();
        for (line_number, line) in content.split_inclusive(|byte| *byte == b'\n').enumerate() {
            if let Some(symbol) = matcher.find(line)? {
                symbols.push((line_number, symbol));
            }
        }
        Ok(Self { symbols })
    }

    /// Finds a symbol on the first line of a nonempty range, or the closest
    /// symbol strictly before the range.
    ///
    /// If there is no preceding symbol, the first symbol in the changed range
    /// is returned. This is useful for newly-added sections.
    pub(crate) fn find(&self, line_range: &Range<usize>) -> Option<&'a [u8]> {
        let preceding_end = self
            .symbols
            .partition_point(|(line, _)| *line < line_range.start);
        let first_in_range = self
            .symbols
            .get(preceding_end)
            .filter(|(line, _)| *line < line_range.end);
        if let Some((line, symbol)) = first_in_range
            && *line == line_range.start
        {
            return Some(symbol);
        }

        self.symbols[..preceding_end]
            .last()
            .or(first_in_range)
            .map(|(_, symbol)| *symbol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_rules_select_gitoxide_definitions() -> io::Result<()> {
        for (name, line) in [
            ("main.rs", "pub async fn run() {"),
            ("main.c", "int run(void) {"),
            ("script.ts", "export async function run() {"),
            ("Gemfile", "def dependencies"),
            ("main.swift", "public func run() {"),
            ("unknown", "generic identifier"),
        ] {
            assert_eq!(
                SourceSymbolMatcher::for_file_name(name)?.find(line.as_bytes())?,
                Some(line.as_bytes()),
                "filename fallback for {name}"
            );
        }
        assert_eq!(
            SourceSymbolMatcher::for_file_name("unknown")?.find(b"  indented")?,
            None
        );
        Ok(())
    }

    #[test]
    fn ambiguous_extensions_accept_each_language() -> io::Result<()> {
        for (name, lines) in [
            ("source.m", ["function y = example(x)", "- (void)method {"]),
            (
                "source.h",
                ["int example(void);", "@interface Example : NSObject"],
            ),
            ("source.mm", ["int example(void);", "- (void)method {"]),
        ] {
            let matcher = SourceSymbolMatcher::for_file_name(name)?;
            for line in lines {
                assert_eq!(matcher.find(line.as_bytes())?, Some(line.as_bytes()));
            }
        }
        Ok(())
    }

    #[test]
    fn scanner_uses_first_symbol_in_range() -> io::Result<()> {
        let matcher = SourceSymbolMatcher::for_file_name("main.rs")?;
        let scanner = SourceSymbolScanner::new(
            &matcher,
            BStr::new(b"preamble\n\nfn first() {\n}\n\nfn second() {\n}\n"),
        )?;
        assert_eq!(scanner.find(&(1..7)), Some(&b"fn first() {"[..]));
        assert_eq!(scanner.find(&(1..2)), None);
        Ok(())
    }

    #[test]
    fn scanner_prefers_range_start_then_preceding_symbol() -> io::Result<()> {
        let matcher = SourceSymbolMatcher::for_file_name("main.rs")?;
        let scanner = SourceSymbolScanner::new(
            &matcher,
            BStr::new(b"fn first() {\n}\n\nfn second() {\n}\n"),
        )?;
        assert_eq!(scanner.find(&(3..4)), Some(&b"fn second() {"[..]));
        assert_eq!(scanner.find(&(3..3)), Some(&b"fn first() {"[..]));
        assert_eq!(scanner.find(&(0..0)), None);
        assert_eq!(scanner.find(&(2..5)), Some(&b"fn first() {"[..]));
        Ok(())
    }
}
