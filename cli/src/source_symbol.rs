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

use std::ops::Range;
use std::sync::LazyLock;

use bstr::BStr;
use bstr::ByteSlice;
use regex::bytes::Regex;

// Built-in source symbol patterns for common languages. This stop-gap only
// selects patterns from common file names/extensions.
// TODO: Replace this with gix support once available, including
// .gitattributes-based driver selection and custom hunk-header regexes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SourceLanguage {
    BibTeX,
    CLike,
    CSharp,
    Dts,
    Elixir,
    Go,
    Html,
    Ini,
    Java,
    JavaScript,
    Kotlin,
    Markdown,
    Matlab,
    MatlabOrObjC,
    ObjC,
    CLikeOrObjC,
    Pascal,
    Perl,
    Php,
    Python,
    R,
    Ruby,
    Rust,
    Scheme,
    Shell,
    Tex,
}

impl SourceLanguage {
    pub(crate) fn from_file_name(file_name: &str) -> Option<Self> {
        let extension = file_name.rsplit_once('.').map(|(_, extension)| extension);
        match extension {
            Some("bib") => Some(Self::BibTeX),
            Some("c" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx") => Some(Self::CLike),
            Some("h") => Some(Self::CLikeOrObjC),
            Some("cs") => Some(Self::CSharp),
            Some("dts" | "dtsi") => Some(Self::Dts),
            Some("ex" | "exs") => Some(Self::Elixir),
            Some("go") => Some(Self::Go),
            Some("htm" | "html" | "xhtml") => Some(Self::Html),
            Some("cfg" | "conf" | "config" | "ini") => Some(Self::Ini),
            Some("java") => Some(Self::Java),
            Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx") => Some(Self::JavaScript),
            Some("kt" | "kts") => Some(Self::Kotlin),
            Some("markdown" | "md" | "mdown") => Some(Self::Markdown),
            // `.m` is shared by MATLAB and Objective-C. Objective-C++ `.mm`
            // and Objective-C headers can also contain ordinary C-like symbols.
            Some("m") => Some(Self::MatlabOrObjC),
            Some("matlab") => Some(Self::Matlab),
            Some("mm") => Some(Self::CLikeOrObjC),
            Some("p" | "pas") => Some(Self::Pascal),
            Some("pl" | "pm" | "pod" | "psgi" | "t") => Some(Self::Perl),
            Some("php" | "php3" | "php4" | "php5" | "phtml") => Some(Self::Php),
            Some("py" | "pyw") => Some(Self::Python),
            Some("r" | "R") => Some(Self::R),
            Some("rb" | "rake" | "gemspec") => Some(Self::Ruby),
            Some("rs") => Some(Self::Rust),
            Some("scm" | "scheme" | "ss" | "lisp" | "lsp") => Some(Self::Scheme),
            Some("sh" | "bash" | "zsh" | "fish") => Some(Self::Shell),
            Some("tex" | "ltx" | "latex") => Some(Self::Tex),
            _ => match file_name {
                "Gemfile" | "Rakefile" => Some(Self::Ruby),
                _ => None,
            },
        }
    }

    fn is_source_symbol(self, line: &[u8]) -> bool {
        static BIBTEX_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"^@[A-Za-z]+[ \t]*\{?[ \t]*[^ \t"@',\\#}{~%]*"#).unwrap()
        });
        static C_LIKE_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"^(?:[A-Za-z_][\w:<>,~]*|[*&]|\[[^\]]*\]|\s+)+[A-Za-z_~][\w:~]*\s*\([^;{}]*",
            )
            .unwrap()
        });
        static CSHARP_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"^(?:(?:[\w@_.]+(?:<[\w@_, \t<>]+>)?)(?:\s+[\w@_.]+(?:<[\w@_, \t<>]+>)?)+\s*\([^;]*|(?:(?:static|public|internal|private|protected|new|unsafe|sealed|abstract|partial)\s+)*(?:class|enum|interface|struct|record)\s+.*|namespace\s+.*)$",
            )
            .unwrap()
        });
        static DTS_FUNCTION_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^(?:/[ \t]*\{|&?[A-Za-z_].*)").unwrap());
        static ELIXIR_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(?:def(?:macro|module|impl|protocol|p)?|test)\s+.*").unwrap()
        });
        static GO_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(?:func\s*.*(?:\{\s*)?|type\s+.*(?:struct|interface)\s*(?:\{\s*)?)$")
                .unwrap()
        });
        static HTML_FUNCTION_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^<[Hh][1-6](?:\s.*)?>.*").unwrap());
        static INI_FUNCTION_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^\[[^]]+\]").unwrap());
        static JAVA_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"^(?:(?:[a-z-]+\s+)*(?:class|enum|interface|record)\s+.*|(?:[A-Za-z_<>&][\]?&<>.,A-Za-z_0-9]*\s+)+[A-Za-z_][A-Za-z_0-9]*\s*\([^;]*)$",
            )
            .unwrap()
        });
        static JAVASCRIPT_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"^(?:(?:export|default|async|static|get|set)\s+)*(?:function\b|class\b|[A-Za-z_$][\w$]*\s*\([^)]*\)\s*\{|[A-Za-z_$][\w$]*\s*[:=]\s*(?:async\s*)?(?:function\b|\([^)]*\)\s*=>))",
            )
            .unwrap()
        });
        static KOTLIN_FUNCTION_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^(?:[a-z]+\s+)*(?:fun|class|interface)\s+.*").unwrap());
        static MARKDOWN_FUNCTION_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^#{1,6}\s+.*").unwrap());
        static MATLAB_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(?:(?:classdef|function)\s+.*|(?:%%%?|##)\s+.*)$").unwrap()
        });
        static OBJC_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"^(?:[-+]\s*\(\s*[A-Za-z_][A-Za-z_0-9* \t]*\)\s*[A-Za-z_].*|(?:[A-Za-z_][A-Za-z_0-9]*\s+)+[A-Za-z_][A-Za-z_0-9]*\s*\([^;]*|@(?:implementation|interface|protocol)\s+.*)$",
            )
            .unwrap()
        });
        static PASCAL_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"^(?:(?:(?:class\s+)?(?:procedure|function)|constructor|destructor|interface|implementation|initialization|finalization)\s*.*|.*=\s*(?:class|record).*)$",
            )
            .unwrap()
        });
        static PERL_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"^(?:package .*|sub [[:alnum:]_':]+[ \t]*(?:\([^)]*\)[ \t]*)?(?::[^;#]*)?(?:\{[ \t]*)?(?:#.*)?|(?:BEGIN|END|INIT|CHECK|UNITCHECK|AUTOLOAD|DESTROY)[ \t]*(?:\{[ \t]*)?(?:#.*)?|=head[0-9] .*)$",
            )
            .unwrap()
        });
        static PHP_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"^(?:(?:(?:public|protected|private|static|abstract|final)\s+)*function.*|(?:(?:(?:final|abstract)\s+)?class|enum|interface|trait).*)$",
            )
            .unwrap()
        });
        static PYTHON_FUNCTION_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^(?:async\s+def|def|class)\s+\w+").unwrap());
        static R_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^[A-Za-z][A-Za-z0-9_.]*\s*(?:<-|=)\s*function.*$").unwrap()
        });
        static RUBY_FUNCTION_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^(?:class|module|def)\s+.*").unwrap());
        static RUST_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r#"^(?:(?:pub(?:\([^)]*\))?|const|async|unsafe|extern(?:\s+"[^"]+")?)\s+)*(?:(?:fn|struct|enum|impl|trait|mod|type|union)\b|macro_rules!\s*)"#,
            )
            .unwrap()
        });
        static SCHEME_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"^(?:\((?:(?:define|def(?:struct|syntax|class|method|rules|record|proto|alias)?)[-*/ \t]|(?:library|module|struct|class)[*+ \t]).*|[Dd][Ee][Ff].*)$",
            )
            .unwrap()
        });
        static SHELL_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"^(?:[A-Za-z_][A-Za-z0-9_]*\s*\(\s*\)|function\s+[A-Za-z_][A-Za-z0-9_]*(?:(?:\s*\(\s*\))|\s+)).*$",
            )
            .unwrap()
        });
        static TEX_FUNCTION_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^\\(?:(?:sub)*section|chapter|part)\*?\{.*").unwrap());

        match self {
            Self::BibTeX => BIBTEX_FUNCTION_RE.is_match(line),
            Self::CLike => {
                let before_paren = line.split(|byte| *byte == b'(').next().unwrap_or(line);
                !line.first().is_some_and(u8::is_ascii_whitespace)
                    && !before_paren.contains_str(": ")
                    && !starts_with_keyword(line, &["do", "for", "if", "switch", "while"])
                    && C_LIKE_FUNCTION_RE.is_match(line)
            }
            Self::CSharp => {
                !starts_with_keyword(
                    line,
                    &[
                        "do", "while", "for", "foreach", "if", "else", "new", "default", "return",
                        "switch", "case", "throw", "catch", "using", "lock", "fixed",
                    ],
                ) && CSHARP_FUNCTION_RE.is_match(line)
            }
            Self::Dts => {
                !line.contains(&b';') && !line.contains(&b'=') && DTS_FUNCTION_RE.is_match(line)
            }
            Self::Elixir => ELIXIR_FUNCTION_RE.is_match(line),
            Self::Go => GO_FUNCTION_RE.is_match(line),
            Self::Html => HTML_FUNCTION_RE.is_match(line),
            Self::Ini => INI_FUNCTION_RE.is_match(line),
            Self::Java => {
                !starts_with_keyword(
                    line,
                    &[
                        "catch",
                        "do",
                        "for",
                        "if",
                        "instanceof",
                        "new",
                        "return",
                        "switch",
                        "throw",
                        "while",
                    ],
                ) && JAVA_FUNCTION_RE.is_match(line)
            }
            Self::JavaScript => {
                !starts_with_keyword(line, &["catch", "for", "if", "switch", "while", "with"])
                    && JAVASCRIPT_FUNCTION_RE.is_match(line)
            }
            Self::Kotlin => KOTLIN_FUNCTION_RE.is_match(line),
            Self::Markdown => MARKDOWN_FUNCTION_RE.is_match(line),
            Self::Matlab => MATLAB_FUNCTION_RE.is_match(line),
            Self::MatlabOrObjC => {
                Self::Matlab.is_source_symbol(line) || Self::ObjC.is_source_symbol(line)
            }
            Self::ObjC => {
                !starts_with_keyword(
                    line,
                    &["do", "for", "if", "else", "return", "switch", "while"],
                ) && OBJC_FUNCTION_RE.is_match(line)
            }
            Self::CLikeOrObjC => {
                Self::CLike.is_source_symbol(line) || Self::ObjC.is_source_symbol(trim_line(line))
            }
            Self::Pascal => PASCAL_FUNCTION_RE.is_match(line),
            Self::Perl => PERL_FUNCTION_RE.is_match(line),
            Self::Php => PHP_FUNCTION_RE.is_match(line),
            Self::Python => PYTHON_FUNCTION_RE.is_match(line),
            Self::R => R_FUNCTION_RE.is_match(line),
            Self::Ruby => RUBY_FUNCTION_RE.is_match(line),
            Self::Rust => RUST_FUNCTION_RE.is_match(line),
            Self::Scheme => SCHEME_FUNCTION_RE.is_match(line),
            Self::Shell => SHELL_FUNCTION_RE.is_match(line),
            Self::Tex => TEX_FUNCTION_RE.is_match(line),
        }
    }
}

fn starts_with_keyword(line: &[u8], keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| {
        line.strip_prefix(keyword.as_bytes()).is_some_and(|rest| {
            rest.first()
                .is_none_or(|byte| byte.is_ascii_whitespace() || matches!(byte, b'(' | b'{' | b';'))
        })
    })
}

fn strip_line_ending(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn trim_line(line: &[u8]) -> &[u8] {
    strip_line_ending(line).trim_ascii()
}

fn is_c_style_comment_line(line: &[u8]) -> bool {
    // A leading `*` can also belong to a pointer-returning function declaration.
    line.starts_with(b"/*") || line.starts_with(b"//")
}

fn is_comment_line(line: &[u8], language: SourceLanguage) -> bool {
    match language {
        SourceLanguage::CLike
        | SourceLanguage::CLikeOrObjC
        | SourceLanguage::CSharp
        | SourceLanguage::Go
        | SourceLanguage::Java
        | SourceLanguage::JavaScript
        | SourceLanguage::Kotlin
        | SourceLanguage::ObjC
        | SourceLanguage::Php
        | SourceLanguage::Rust => is_c_style_comment_line(line),
        SourceLanguage::MatlabOrObjC => is_c_style_comment_line(line),
        SourceLanguage::Elixir
        | SourceLanguage::Perl
        | SourceLanguage::Python
        | SourceLanguage::R
        | SourceLanguage::Ruby
        | SourceLanguage::Shell => line.starts_with(b"#"),
        SourceLanguage::Tex => line.starts_with(b"%"),
        SourceLanguage::BibTeX
        | SourceLanguage::Dts
        | SourceLanguage::Html
        | SourceLanguage::Ini
        | SourceLanguage::Markdown
        | SourceLanguage::Matlab
        | SourceLanguage::Pascal
        | SourceLanguage::Scheme => false,
    }
}

fn source_symbols(
    content: &BStr,
    language: SourceLanguage,
) -> impl Iterator<Item = (usize, &[u8])> {
    content
        .split_inclusive(|byte| *byte == b'\n')
        .enumerate()
        .filter_map(move |(line_number, line)| {
            let line = strip_line_ending(line);
            let trimmed_line = trim_line(line);
            let line_to_match = match language {
                // Keep leading whitespace so the C-like matcher excludes indented
                // calls/control flow inside a function. This heuristic intentionally
                // also excludes indented C++ class methods and namespace members.
                SourceLanguage::CLike | SourceLanguage::CLikeOrObjC => line,
                _ => trimmed_line,
            };
            (!is_comment_line(trimmed_line, language) && language.is_source_symbol(line_to_match))
                .then_some((line_number, trimmed_line))
        })
}

/// Finds source symbols near line ranges in one version of a file.
///
/// A scanner operates on a single file content so it can also be used for
/// diffs with more than two sides.
pub(crate) struct SourceSymbolScanner<'a> {
    symbols: Vec<(usize, &'a [u8])>,
}

impl<'a> SourceSymbolScanner<'a> {
    pub(crate) fn new(language: SourceLanguage, content: &'a BStr) -> Self {
        Self {
            symbols: source_symbols(content, language).collect(),
        }
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

    fn collect_source_symbols(content: &BStr, language: SourceLanguage) -> Vec<(usize, &[u8])> {
        source_symbols(content, language).collect()
    }

    #[test]
    fn test_source_symbol_patterns() {
        let samples: &[(SourceLanguage, &[&str])] = &[
            (
                SourceLanguage::BibTeX,
                &[
                    "@article{example,",
                    "@string { name = value }",
                    "plain text",
                ],
            ),
            (
                SourceLanguage::CLike,
                &[
                    "static int example(int value)",
                    "void *pointer(void)",
                    "    call(value)",
                    "if (condition)",
                    "label: call()",
                ],
            ),
            (
                SourceLanguage::CSharp,
                &[
                    "public class Example",
                    "namespace Example.Core",
                    "public async Task<int> Run(int value)",
                    "if (condition)",
                ],
            ),
            (
                SourceLanguage::Dts,
                &["/ {", "device_node {", "property = <1>;"],
            ),
            (
                SourceLanguage::Elixir,
                &["defmodule Example do", "defp run(value) do", "if ready do"],
            ),
            (
                SourceLanguage::Go,
                &[
                    "func Example(value int) int {",
                    "type Example struct {",
                    "var example = func() {}",
                ],
            ),
            (
                SourceLanguage::Html,
                &[
                    "<h2 class=\"title\">Example</h2>",
                    "<H1>Title</H1>",
                    "<p>text</p>",
                ],
            ),
            (
                SourceLanguage::Ini,
                &["[section]", "[section.subsection] value", "key = value"],
            ),
            (
                SourceLanguage::Java,
                &[
                    "public class Example",
                    "public static String run(int value)",
                    "if (condition)",
                ],
            ),
            (
                SourceLanguage::JavaScript,
                &[
                    "export async function example() {",
                    "class Example {",
                    "handler = async (event) => {",
                    "if (condition) {",
                ],
            ),
            (
                SourceLanguage::Kotlin,
                &[
                    "public fun example(value: Int)",
                    "data class Example",
                    "if (condition)",
                ],
            ),
            (
                SourceLanguage::Markdown,
                &["# Title", "###### Detail", "plain text", "#missing-space"],
            ),
            (
                SourceLanguage::Matlab,
                &[
                    "function y = example(x)",
                    "classdef Example",
                    "%% Section",
                    "y = x + 1;",
                ],
            ),
            (
                SourceLanguage::ObjC,
                &[
                    "- (void)example:(id)value",
                    "@interface Example : NSObject",
                    "if (condition)",
                ],
            ),
            (
                SourceLanguage::Pascal,
                &[
                    "function Example(Value: Integer): Integer;",
                    "TExample = class(TObject)",
                    "var Value: Integer;",
                ],
            ),
            (
                SourceLanguage::Perl,
                &[
                    "package Example;",
                    "sub example : prototype($) {",
                    "BEGIN {",
                    "=head1 Documentation",
                    "my $value = 1;",
                ],
            ),
            (
                SourceLanguage::Php,
                &[
                    "public static function example($value) {",
                    "final class Example {",
                    "if ($condition) {",
                ],
            ),
            (
                SourceLanguage::Python,
                &[
                    "async def example(value):",
                    "class Example:",
                    "if condition:",
                ],
            ),
            (
                SourceLanguage::R,
                &[
                    "example <- function(value) {",
                    "example.name = function(value) {",
                    "if (condition) {",
                ],
            ),
            (
                SourceLanguage::Ruby,
                &["module Example", "def example(value)", "if condition"],
            ),
            (
                SourceLanguage::Rust,
                &[
                    "pub(crate) async fn example(value: i32) -> i32 {",
                    "impl<T> Example<T> {",
                    "macro_rules! example {",
                    "if condition {",
                ],
            ),
            (
                SourceLanguage::Scheme,
                &[
                    "(define (example value)",
                    "(module example scheme",
                    "(display value)",
                    "display value",
                ],
            ),
            (
                SourceLanguage::Shell,
                &["example() {", "function example {", "if condition; then"],
            ),
            (
                SourceLanguage::Tex,
                &[
                    "\\section{Example}",
                    "\\subsubsection*{Detail}",
                    "plain text",
                ],
            ),
        ];

        let results = samples
            .iter()
            .flat_map(|(language, lines)| {
                lines.iter().map(move |line| {
                    format!(
                        "{language:?}: {:?} = {}",
                        BStr::new(line),
                        language.is_source_symbol(line.as_bytes())
                    )
                })
            })
            .collect::<Vec<_>>();
        insta::assert_debug_snapshot!(results, @r#######"
        [
            "BibTeX: \"@article{example,\" = true",
            "BibTeX: \"@string { name = value }\" = true",
            "BibTeX: \"plain text\" = false",
            "CLike: \"static int example(int value)\" = true",
            "CLike: \"void *pointer(void)\" = true",
            "CLike: \"    call(value)\" = false",
            "CLike: \"if (condition)\" = false",
            "CLike: \"label: call()\" = false",
            "CSharp: \"public class Example\" = true",
            "CSharp: \"namespace Example.Core\" = true",
            "CSharp: \"public async Task<int> Run(int value)\" = true",
            "CSharp: \"if (condition)\" = false",
            "Dts: \"/ {\" = true",
            "Dts: \"device_node {\" = true",
            "Dts: \"property = <1>;\" = false",
            "Elixir: \"defmodule Example do\" = true",
            "Elixir: \"defp run(value) do\" = true",
            "Elixir: \"if ready do\" = false",
            "Go: \"func Example(value int) int {\" = true",
            "Go: \"type Example struct {\" = true",
            "Go: \"var example = func() {}\" = false",
            "Html: \"<h2 class=\\\"title\\\">Example</h2>\" = true",
            "Html: \"<H1>Title</H1>\" = true",
            "Html: \"<p>text</p>\" = false",
            "Ini: \"[section]\" = true",
            "Ini: \"[section.subsection] value\" = true",
            "Ini: \"key = value\" = false",
            "Java: \"public class Example\" = true",
            "Java: \"public static String run(int value)\" = true",
            "Java: \"if (condition)\" = false",
            "JavaScript: \"export async function example() {\" = true",
            "JavaScript: \"class Example {\" = true",
            "JavaScript: \"handler = async (event) => {\" = true",
            "JavaScript: \"if (condition) {\" = false",
            "Kotlin: \"public fun example(value: Int)\" = true",
            "Kotlin: \"data class Example\" = true",
            "Kotlin: \"if (condition)\" = false",
            "Markdown: \"# Title\" = true",
            "Markdown: \"###### Detail\" = true",
            "Markdown: \"plain text\" = false",
            "Markdown: \"#missing-space\" = false",
            "Matlab: \"function y = example(x)\" = true",
            "Matlab: \"classdef Example\" = true",
            "Matlab: \"%% Section\" = true",
            "Matlab: \"y = x + 1;\" = false",
            "ObjC: \"- (void)example:(id)value\" = true",
            "ObjC: \"@interface Example : NSObject\" = true",
            "ObjC: \"if (condition)\" = false",
            "Pascal: \"function Example(Value: Integer): Integer;\" = true",
            "Pascal: \"TExample = class(TObject)\" = true",
            "Pascal: \"var Value: Integer;\" = false",
            "Perl: \"package Example;\" = true",
            "Perl: \"sub example : prototype($) {\" = true",
            "Perl: \"BEGIN {\" = true",
            "Perl: \"=head1 Documentation\" = true",
            "Perl: \"my $value = 1;\" = false",
            "Php: \"public static function example($value) {\" = true",
            "Php: \"final class Example {\" = true",
            "Php: \"if ($condition) {\" = false",
            "Python: \"async def example(value):\" = true",
            "Python: \"class Example:\" = true",
            "Python: \"if condition:\" = false",
            "R: \"example <- function(value) {\" = true",
            "R: \"example.name = function(value) {\" = true",
            "R: \"if (condition) {\" = false",
            "Ruby: \"module Example\" = true",
            "Ruby: \"def example(value)\" = true",
            "Ruby: \"if condition\" = false",
            "Rust: \"pub(crate) async fn example(value: i32) -> i32 {\" = true",
            "Rust: \"impl<T> Example<T> {\" = true",
            "Rust: \"macro_rules! example {\" = true",
            "Rust: \"if condition {\" = false",
            "Scheme: \"(define (example value)\" = true",
            "Scheme: \"(module example scheme\" = true",
            "Scheme: \"(display value)\" = false",
            "Scheme: \"display value\" = false",
            "Shell: \"example() {\" = true",
            "Shell: \"function example {\" = true",
            "Shell: \"if condition; then\" = false",
            "Tex: \"\\\\section{Example}\" = true",
            "Tex: \"\\\\subsubsection*{Detail}\" = true",
            "Tex: \"plain text\" = false",
        ]
        "#######);
    }

    #[test]
    fn test_collect_source_symbols_ignores_c_comments_and_indentation() {
        let content = BStr::new(
            b"void outer(void)\n/*\n * misleading(comment)\n */\n    call(value);\nextern int actual(int value);\n",
        );
        let symbols = collect_source_symbols(content, SourceLanguage::CLike)
            .into_iter()
            .map(|(line, symbol)| (line, BStr::new(symbol)))
            .collect::<Vec<_>>();
        insta::assert_debug_snapshot!(symbols, @r#"
        [
            (
                0,
                "void outer(void)",
            ),
            (
                5,
                "extern int actual(int value);",
            ),
        ]
        "#);
    }

    #[test]
    fn test_source_symbol_scanner_finds_pointer_returning_c_function() {
        let content = BStr::new(b"int\n*foo(void) {\n    return 0;\n}\n");
        for language in [SourceLanguage::CLike, SourceLanguage::CLikeOrObjC] {
            let scanner = SourceSymbolScanner::new(language, content);
            assert_eq!(scanner.find(&(2..3)), Some(&b"*foo(void) {"[..]));
        }
    }

    #[test]
    fn test_source_symbol_scanner_uses_first_symbol_in_range() {
        let scanner = SourceSymbolScanner::new(
            SourceLanguage::Rust,
            BStr::new(b"preamble\n\nfn first() {\n}\n\nfn second() {\n}\n"),
        );

        assert_eq!(scanner.find(&(1..7)), Some(&b"fn first() {"[..]));
        assert_eq!(scanner.find(&(1..2)), None);
    }

    #[test]
    fn test_source_symbol_scanner_range_start() {
        let scanner = SourceSymbolScanner::new(
            SourceLanguage::Rust,
            BStr::new(b"fn first() {\n}\n\nfn second() {\n}\n"),
        );

        // An exact match takes precedence over the preceding symbol.
        assert_eq!(scanner.find(&(3..4)), Some(&b"fn second() {"[..]));
        // Empty ranges exclude the symbol at their start.
        assert_eq!(scanner.find(&(3..3)), Some(&b"fn first() {"[..]));
        assert_eq!(scanner.find(&(0..0)), None);
        // A preceding symbol takes precedence over a later symbol in the range.
        assert_eq!(scanner.find(&(2..5)), Some(&b"fn first() {"[..]));
    }

    #[test]
    fn test_source_symbol_scanner_handles_ambiguous_objc_extension() {
        let objc_scanner = SourceSymbolScanner::new(
            SourceLanguage::MatlabOrObjC,
            BStr::new(b"@implementation Example\n- (void)method {\n    value++;\n}\n@end\n"),
        );
        assert_eq!(objc_scanner.find(&(2..3)), Some(&b"- (void)method {"[..]));

        let matlab_scanner = SourceSymbolScanner::new(
            SourceLanguage::MatlabOrObjC,
            BStr::new(b"%% Section\nfunction y = example(x)\ny = x + 1;\nend\n"),
        );
        assert_eq!(matlab_scanner.find(&(0..1)), Some(&b"%% Section"[..]));
        assert_eq!(
            matlab_scanner.find(&(2..3)),
            Some(&b"function y = example(x)"[..])
        );
    }
}
