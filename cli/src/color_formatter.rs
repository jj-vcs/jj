// Copyright 2020 The Jujutsu Authors
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

use std::collections::HashMap;
use std::io;
use std::io::Error;
use std::io::Write;
use std::mem;
use std::sync::Arc;

use crossterm::queue;
use crossterm::style::Attribute;
use crossterm::style::Color;
use crossterm::style::SetAttribute;
use crossterm::style::SetBackgroundColor;
use crossterm::style::SetForegroundColor;
use itertools::Itertools as _;
use jj_lib::config::ConfigGetError;
use jj_lib::config::StackedConfig;
use jj_lib::formatter::Formatter;
use jj_lib::formatter::PlainTextFormatter;
use serde::de::Deserialize as _;
use serde::de::Error as _;
use serde::de::IntoDeserializer as _;

type Rules = Vec<(Vec<String>, Style)>;

/// Creates `Formatter` instances with preconfigured parameters.
#[derive(Clone, Debug)]
pub struct FormatterFactory {
    kind: FormatterFactoryKind,
}

#[derive(Clone, Debug)]
enum FormatterFactoryKind {
    PlainText,
    Sanitized,
    Color { rules: Arc<Rules>, debug: bool },
}

impl FormatterFactory {
    pub fn plain_text() -> Self {
        let kind = FormatterFactoryKind::PlainText;
        Self { kind }
    }

    pub fn sanitized() -> Self {
        let kind = FormatterFactoryKind::Sanitized;
        Self { kind }
    }

    pub fn color(config: &StackedConfig, debug: bool) -> Result<Self, ConfigGetError> {
        let rules = Arc::new(rules_from_config(config)?);
        let kind = FormatterFactoryKind::Color { rules, debug };
        Ok(Self { kind })
    }

    pub fn new_formatter<'output, W: Write + 'output>(
        &self,
        output: W,
    ) -> Box<dyn Formatter + 'output> {
        match &self.kind {
            FormatterFactoryKind::PlainText => Box::new(PlainTextFormatter::new(output)),
            FormatterFactoryKind::Sanitized => Box::new(SanitizingFormatter::new(output)),
            FormatterFactoryKind::Color { rules, debug } => {
                Box::new(ColorFormatter::new(output, rules.clone(), *debug))
            }
        }
    }

    pub fn maybe_color(&self) -> bool {
        matches!(self.kind, FormatterFactoryKind::Color { .. })
    }
}

pub struct SanitizingFormatter<W> {
    output: W,
}

impl<W> SanitizingFormatter<W> {
    pub fn new(output: W) -> Self {
        Self { output }
    }
}

impl<W: Write> Write for SanitizingFormatter<W> {
    fn write(&mut self, data: &[u8]) -> Result<usize, Error> {
        write_sanitized(&mut self.output, data)?;
        Ok(data.len())
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.output.flush()
    }
}

impl<W: Write> Formatter for SanitizingFormatter<W> {
    fn raw(&mut self) -> io::Result<Box<dyn Write + '_>> {
        Ok(Box::new(self.output.by_ref()))
    }

    fn push_label(&mut self, _label: &str) {}

    fn pop_label(&mut self) {}

    fn maybe_color(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Style {
    #[serde(deserialize_with = "deserialize_color_opt")]
    pub fg: Option<Color>,
    #[serde(deserialize_with = "deserialize_color_opt")]
    pub bg: Option<Color>,
    pub bold: Option<bool>,
    pub dim: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub reverse: Option<bool>,
}

impl Style {
    fn merge(&mut self, other: &Self) {
        self.fg = other.fg.or(self.fg);
        self.bg = other.bg.or(self.bg);
        self.bold = other.bold.or(self.bold);
        self.dim = other.dim.or(self.dim);
        self.italic = other.italic.or(self.italic);
        self.underline = other.underline.or(self.underline);
        self.reverse = other.reverse.or(self.reverse);
    }
}

#[derive(Clone, Debug)]
pub struct ColorFormatter<W: Write> {
    output: W,
    rules: Arc<Rules>,
    /// The stack of currently applied labels. These determine the desired
    /// style.
    labels: Vec<String>,
    cached_styles: HashMap<Vec<String>, Style>,
    /// The style we last wrote to the output.
    current_style: Style,
    /// The debug string (space-separated labels) we last wrote to the output.
    /// Initialize to None to turn debug strings off.
    current_debug: Option<String>,
}

impl<W: Write> ColorFormatter<W> {
    pub fn new(output: W, rules: Arc<Rules>, debug: bool) -> Self {
        Self {
            output,
            rules,
            labels: vec![],
            cached_styles: HashMap::new(),
            current_style: Style::default(),
            current_debug: debug.then(String::new),
        }
    }

    pub fn for_config(
        output: W,
        config: &StackedConfig,
        debug: bool,
    ) -> Result<Self, ConfigGetError> {
        let rules = rules_from_config(config)?;
        Ok(Self::new(output, Arc::new(rules), debug))
    }

    fn requested_style(&mut self) -> Style {
        if let Some(cached) = self.cached_styles.get(&self.labels) {
            cached.clone()
        } else {
            // We use the reverse list of matched indices as a measure of how well the rule
            // matches the actual labels. For example, for rule "a d" and the actual labels
            // "a b c d", we'll get [3,0]. We compare them by Rust's default Vec comparison.
            // That means "a d" will trump both rule "d" (priority [3]) and rule
            // "a b c" (priority [2,1,0]).
            let mut matched_styles = vec![];
            for (labels, style) in self.rules.as_ref() {
                let mut labels_iter = self.labels.iter().enumerate();
                // The indexes in the current label stack that match the required label.
                let mut matched_indices = vec![];
                for required_label in labels {
                    for (label_index, label) in &mut labels_iter {
                        if label == required_label {
                            matched_indices.push(label_index);
                            break;
                        }
                    }
                }
                if matched_indices.len() == labels.len() {
                    matched_indices.reverse();
                    matched_styles.push((style, matched_indices));
                }
            }
            matched_styles.sort_by_key(|(_, indices)| indices.clone());

            let mut style = Style::default();
            for (matched_style, _) in matched_styles {
                style.merge(matched_style);
            }
            self.cached_styles
                .insert(self.labels.clone(), style.clone());
            style
        }
    }

    fn write_new_style(&mut self) -> io::Result<()> {
        let new_debug = match &self.current_debug {
            Some(current) => {
                let joined = self.labels.join(" ");
                if joined == *current {
                    None
                } else {
                    if !current.is_empty() {
                        write!(self.output, ">>")?;
                    }
                    Some(joined)
                }
            }
            None => None,
        };
        let new_style = self.requested_style();
        if new_style != self.current_style {
            // Bold and Dim change intensity, and NormalIntensity would reset
            // both. Also, NoBold results in double underlining on some
            // terminals. Therefore, we use Reset instead. However, that resets
            // other attributes as well, so we reset our record of the current
            // style so we re-apply the other attributes below. Maybe we can use
            // NormalIntensity instead of Reset, but let's simply reset all
            // attributes to work around potential terminal incompatibility.
            let new_bold = new_style.bold.unwrap_or_default();
            let new_dim = new_style.dim.unwrap_or_default();
            if (new_style.bold != self.current_style.bold && !new_bold)
                || (new_style.dim != self.current_style.dim && !new_dim)
            {
                queue!(self.output, SetAttribute(Attribute::Reset))?;
                self.current_style = Style::default();
            }
            if new_style.bold != self.current_style.bold && new_bold {
                queue!(self.output, SetAttribute(Attribute::Bold))?;
            }
            if new_style.dim != self.current_style.dim && new_dim {
                queue!(self.output, SetAttribute(Attribute::Dim))?;
            }

            if new_style.italic != self.current_style.italic {
                if new_style.italic.unwrap_or_default() {
                    queue!(self.output, SetAttribute(Attribute::Italic))?;
                } else {
                    queue!(self.output, SetAttribute(Attribute::NoItalic))?;
                }
            }
            if new_style.underline != self.current_style.underline {
                if new_style.underline.unwrap_or_default() {
                    queue!(self.output, SetAttribute(Attribute::Underlined))?;
                } else {
                    queue!(self.output, SetAttribute(Attribute::NoUnderline))?;
                }
            }
            if new_style.reverse != self.current_style.reverse {
                if new_style.reverse.unwrap_or_default() {
                    queue!(self.output, SetAttribute(Attribute::Reverse))?;
                } else {
                    queue!(self.output, SetAttribute(Attribute::NoReverse))?;
                }
            }
            if new_style.fg != self.current_style.fg {
                queue!(
                    self.output,
                    SetForegroundColor(new_style.fg.unwrap_or(Color::Reset))
                )?;
            }
            if new_style.bg != self.current_style.bg {
                queue!(
                    self.output,
                    SetBackgroundColor(new_style.bg.unwrap_or(Color::Reset))
                )?;
            }
            self.current_style = new_style;
        }
        if let Some(d) = new_debug {
            if !d.is_empty() {
                write!(self.output, "<<{d}::")?;
            }
            self.current_debug = Some(d);
        }
        Ok(())
    }
}

fn rules_from_config(config: &StackedConfig) -> Result<Rules, ConfigGetError> {
    config
        .table_keys("colors")
        .map(|key| {
            let labels = key
                .split_whitespace()
                .map(ToString::to_string)
                .collect_vec();
            let style = config.get_value_with(["colors", key], |value| {
                if value.is_str() {
                    Ok(Style {
                        fg: Some(deserialize_color(value.into_deserializer())?),
                        bg: None,
                        bold: None,
                        dim: None,
                        italic: None,
                        underline: None,
                        reverse: None,
                    })
                } else if value.is_inline_table() {
                    Style::deserialize(value.into_deserializer())
                } else {
                    Err(toml_edit::de::Error::custom(format!(
                        "invalid type: {}, expected a color name or a table of styles",
                        value.type_name()
                    )))
                }
            })?;
            Ok((labels, style))
        })
        .collect()
}

fn deserialize_color<'de, D>(deserializer: D) -> Result<Color, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let color_str = String::deserialize(deserializer)?;
    color_for_string(&color_str).map_err(D::Error::custom)
}

fn deserialize_color_opt<'de, D>(deserializer: D) -> Result<Option<Color>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_color(deserializer).map(Some)
}

fn color_for_string(color_str: &str) -> Result<Color, String> {
    match color_str {
        "default" => Ok(Color::Reset),
        "black" => Ok(Color::Black),
        "red" => Ok(Color::DarkRed),
        "green" => Ok(Color::DarkGreen),
        "yellow" => Ok(Color::DarkYellow),
        "blue" => Ok(Color::DarkBlue),
        "magenta" => Ok(Color::DarkMagenta),
        "cyan" => Ok(Color::DarkCyan),
        "white" => Ok(Color::Grey),
        "bright black" => Ok(Color::DarkGrey),
        "bright red" => Ok(Color::Red),
        "bright green" => Ok(Color::Green),
        "bright yellow" => Ok(Color::Yellow),
        "bright blue" => Ok(Color::Blue),
        "bright magenta" => Ok(Color::Magenta),
        "bright cyan" => Ok(Color::Cyan),
        "bright white" => Ok(Color::White),
        _ => color_for_ansi256_index(color_str)
            .or_else(|| color_for_hex(color_str))
            .ok_or_else(|| format!("Invalid color: {color_str}")),
    }
}

fn color_for_ansi256_index(color: &str) -> Option<Color> {
    color
        .strip_prefix("ansi-color-")
        .filter(|s| *s == "0" || !s.starts_with('0'))
        .and_then(|n| n.parse::<u8>().ok())
        .map(Color::AnsiValue)
}

fn color_for_hex(color: &str) -> Option<Color> {
    if color.len() == 7
        && color.starts_with('#')
        && color[1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        let r = u8::from_str_radix(&color[1..3], 16);
        let g = u8::from_str_radix(&color[3..5], 16);
        let b = u8::from_str_radix(&color[5..7], 16);
        match (r, g, b) {
            (Ok(r), Ok(g), Ok(b)) => Some(Color::Rgb { r, g, b }),
            _ => None,
        }
    } else {
        None
    }
}

impl<W: Write> Write for ColorFormatter<W> {
    fn write(&mut self, data: &[u8]) -> Result<usize, Error> {
        /*
        We clear the current style at the end of each line, and then we re-apply the style
        after the newline. There are several reasons for this:

         * We can more easily skip styling a trailing blank line, which other
           internal code then can correctly detect as having a trailing
           newline.

         * Some tools (like `less -R`) add an extra newline if the final
           character is not a newline (e.g. if there's a color reset after
           it), which led to an annoying blank line after the diff summary in
           e.g. `jj status`.

         * Since each line is styled independently, you get all the necessary
           escapes even when grepping through the output.

         * Some terminals extend background color to the end of the terminal
           (i.e. past the newline character), which is probably not what the
           user wanted.

         * Some tools (like `less -R`) get confused and lose coloring of lines
           after a newline.
         */

        for line in data.split_inclusive(|b| *b == b'\n') {
            if line.ends_with(b"\n") {
                self.write_new_style()?;
                write_sanitized(&mut self.output, &line[..line.len() - 1])?;
                let labels = mem::take(&mut self.labels);
                self.write_new_style()?;
                self.output.write_all(b"\n")?;
                self.labels = labels;
            } else {
                self.write_new_style()?;
                write_sanitized(&mut self.output, line)?;
            }
        }

        Ok(data.len())
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.write_new_style()?;
        self.output.flush()
    }
}

impl<W: Write> Formatter for ColorFormatter<W> {
    fn raw(&mut self) -> io::Result<Box<dyn Write + '_>> {
        self.write_new_style()?;
        Ok(Box::new(self.output.by_ref()))
    }

    fn push_label(&mut self, label: &str) {
        self.labels.push(label.to_owned());
    }

    fn pop_label(&mut self) {
        self.labels.pop();
    }

    fn maybe_color(&self) -> bool {
        true
    }
}

impl<W: Write> Drop for ColorFormatter<W> {
    fn drop(&mut self) {
        // If a `ColorFormatter` was dropped without flushing, let's try to
        // reset any currently active style.
        self.labels.clear();
        self.write_new_style().ok();
    }
}

fn write_sanitized(output: &mut impl Write, buf: &[u8]) -> Result<(), Error> {
    if buf.contains(&b'\x1b') {
        let mut sanitized = Vec::with_capacity(buf.len());
        for b in buf {
            if *b == b'\x1b' {
                sanitized.extend_from_slice("␛".as_bytes());
            } else {
                sanitized.push(*b);
            }
        }
        output.write_all(&sanitized)
    } else {
        output.write_all(buf)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use bstr::BString;
    use indexmap::IndexMap;
    use indoc::indoc;
    use jj_lib::config::ConfigLayer;
    use jj_lib::config::ConfigSource;
    use testutils::TestResult;

    use super::*;

    fn config_from_string(text: &str) -> StackedConfig {
        let mut config = StackedConfig::empty();
        config.add_layer(ConfigLayer::parse(ConfigSource::User, text).unwrap());
        config
    }

    /// Appends "[EOF]" marker to the output text.
    ///
    /// This is a workaround for https://github.com/mitsuhiko/insta/issues/384.
    fn to_snapshot_string(output: impl Into<Vec<u8>>) -> BString {
        let mut output = output.into();
        output.extend_from_slice(b"[EOF]\n");
        BString::new(output)
    }

    #[test]
    fn test_sanitizing_formatter_ansi_codes_in_text() -> TestResult {
        // Test that ANSI codes in the input text are escaped.
        let mut output: Vec<u8> = vec![];
        let mut formatter = SanitizingFormatter::new(&mut output);
        write!(formatter, "\x1b[1mnot actually bold\x1b[0m")?;
        insta::assert_snapshot!(to_snapshot_string(output), @"␛[1mnot actually bold␛[0m[EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_color_codes() -> TestResult {
        // Test the color code for each color.
        // Use the color name as the label.
        let config = config_from_string(indoc! {"
            [colors]
            black = 'black'
            red = 'red'
            green = 'green'
            yellow = 'yellow'
            blue = 'blue'
            magenta = 'magenta'
            cyan = 'cyan'
            white = 'white'
            bright-black = 'bright black'
            bright-red = 'bright red'
            bright-green = 'bright green'
            bright-yellow = 'bright yellow'
            bright-blue = 'bright blue'
            bright-magenta = 'bright magenta'
            bright-cyan = 'bright cyan'
            bright-white = 'bright white'
        "});
        let colors: IndexMap<String, String> = config.get("colors")?;
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        for (label, color) in &colors {
            formatter.push_label(label);
            write!(formatter, " {color} ")?;
            formatter.pop_label();
            writeln!(formatter)?;
        }
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"
        [38;5;0m black [39m
        [38;5;1m red [39m
        [38;5;2m green [39m
        [38;5;3m yellow [39m
        [38;5;4m blue [39m
        [38;5;5m magenta [39m
        [38;5;6m cyan [39m
        [38;5;7m white [39m
        [38;5;8m bright black [39m
        [38;5;9m bright red [39m
        [38;5;10m bright green [39m
        [38;5;11m bright yellow [39m
        [38;5;12m bright blue [39m
        [38;5;13m bright magenta [39m
        [38;5;14m bright cyan [39m
        [38;5;15m bright white [39m
        [EOF]
        ");
        Ok(())
    }

    #[test]
    fn test_color_for_ansi256_index() -> TestResult {
        assert_eq!(
            color_for_ansi256_index("ansi-color-0"),
            Some(Color::AnsiValue(0))
        );
        assert_eq!(
            color_for_ansi256_index("ansi-color-10"),
            Some(Color::AnsiValue(10))
        );
        assert_eq!(
            color_for_ansi256_index("ansi-color-255"),
            Some(Color::AnsiValue(255))
        );
        assert_eq!(color_for_ansi256_index("ansi-color-256"), None);

        assert_eq!(color_for_ansi256_index("ansi-color-00"), None);
        assert_eq!(color_for_ansi256_index("ansi-color-010"), None);
        assert_eq!(color_for_ansi256_index("ansi-color-0255"), None);
        Ok(())
    }

    #[test]
    fn test_color_for_hex() -> TestResult {
        assert_eq!(
            color_for_hex("#000000"),
            Some(Color::Rgb { r: 0, g: 0, b: 0 })
        );
        assert_eq!(
            color_for_hex("#fab123"),
            Some(Color::Rgb {
                r: 0xfa,
                g: 0xb1,
                b: 0x23
            })
        );
        assert_eq!(
            color_for_hex("#F00D13"),
            Some(Color::Rgb {
                r: 0xf0,
                g: 0x0d,
                b: 0x13
            })
        );
        assert_eq!(
            color_for_hex("#ffffff"),
            Some(Color::Rgb {
                r: 255,
                g: 255,
                b: 255
            })
        );

        assert_eq!(color_for_hex("000000"), None);
        assert_eq!(color_for_hex("0000000"), None);
        assert_eq!(color_for_hex("#00000g"), None);
        assert_eq!(color_for_hex("#á00000"), None);
        Ok(())
    }

    #[test]
    fn test_color_formatter_ansi256() -> TestResult {
        let config = config_from_string(
            r#"
        [colors]
        purple-bg = { fg = "ansi-color-15", bg = "ansi-color-93" }
        gray = "ansi-color-244"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("purple-bg");
        write!(formatter, " purple background ")?;
        formatter.pop_label();
        writeln!(formatter)?;
        formatter.push_label("gray");
        write!(formatter, " gray ")?;
        formatter.pop_label();
        writeln!(formatter)?;
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"
        [38;5;15m[48;5;93m purple background [39m[49m
        [38;5;244m gray [39m
        [EOF]
        ");
        Ok(())
    }

    #[test]
    fn test_color_formatter_hex_colors() -> TestResult {
        // Test the color code for each color.
        let config = config_from_string(indoc! {"
            [colors]
            black = '#000000'
            white = '#ffffff'
            pastel-blue = '#AFE0D9'
        "});
        let colors: IndexMap<String, String> = config.get("colors")?;
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        for label in colors.keys() {
            formatter.push_label(&label.replace(' ', "-"));
            write!(formatter, " {label} ")?;
            formatter.pop_label();
            writeln!(formatter)?;
        }
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"
        [38;2;0;0;0m black [39m
        [38;2;255;255;255m white [39m
        [38;2;175;224;217m pastel-blue [39m
        [EOF]
        ");
        Ok(())
    }

    #[test]
    fn test_color_formatter_single_label() -> TestResult {
        // Test that a single label can be colored and that the color is reset
        // afterwards.
        let config = config_from_string(
            r#"
        colors.inside = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        write!(formatter, " before ")?;
        formatter.push_label("inside");
        write!(formatter, " inside ")?;
        formatter.pop_label();
        write!(formatter, " after ")?;
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @" before [38;5;2m inside [39m after [EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_attributes() -> TestResult {
        // Test that each attribute of the style can be set and that they can be
        // combined in a single rule or by using multiple rules.
        let config = config_from_string(
            r#"
        colors.red_fg = { fg = "red" }
        colors.blue_bg = { bg = "blue" }
        colors.bold_font = { bold = true }
        colors.dim_font = { dim = true }
        colors.italic_text = { italic = true }
        colors.underlined_text = { underline = true }
        colors.reversed_colors = { reverse = true }
        colors.multiple = { fg = "green", bg = "yellow", bold = true, italic = true, underline = true, reverse = true }
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("red_fg");
        write!(formatter, " fg only ")?;
        formatter.pop_label();
        writeln!(formatter)?;
        formatter.push_label("blue_bg");
        write!(formatter, " bg only ")?;
        formatter.pop_label();
        writeln!(formatter)?;
        formatter.push_label("bold_font");
        write!(formatter, " bold only ")?;
        formatter.pop_label();
        writeln!(formatter)?;
        formatter.push_label("dim_font");
        write!(formatter, " dim only ")?;
        formatter.pop_label();
        writeln!(formatter)?;
        formatter.push_label("italic_text");
        write!(formatter, " italic only ")?;
        formatter.pop_label();
        writeln!(formatter)?;
        formatter.push_label("underlined_text");
        write!(formatter, " underlined only ")?;
        formatter.pop_label();
        writeln!(formatter)?;
        formatter.push_label("reversed_colors");
        write!(formatter, " reverse only ")?;
        formatter.pop_label();
        writeln!(formatter)?;
        formatter.push_label("multiple");
        write!(formatter, " single rule ")?;
        formatter.pop_label();
        writeln!(formatter)?;
        formatter.push_label("red_fg");
        formatter.push_label("blue_bg");
        write!(formatter, " two rules ")?;
        formatter.pop_label();
        formatter.pop_label();
        writeln!(formatter)?;
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"
        [38;5;1m fg only [39m
        [48;5;4m bg only [49m
        [1m bold only [0m
        [2m dim only [0m
        [3m italic only [23m
        [4m underlined only [24m
        [7m reverse only [27m
        [1m[3m[4m[7m[38;5;2m[48;5;3m single rule [0m
        [38;5;1m[48;5;4m two rules [39m[49m
        [EOF]
        ");
        Ok(())
    }

    #[test]
    fn test_color_formatter_bold_reset() -> TestResult {
        // Test that we don't lose other attributes when we reset the bold attribute.
        let config = config_from_string(indoc! {"
            [colors]
            not_bold = { fg = 'red', bg = 'blue', italic = true, underline = true }
            bold_font = { bold = true }
            stop_bold = { bold = false }
        "});
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("not_bold");
        write!(formatter, " not bold ")?;
        formatter.push_label("bold_font");
        write!(formatter, " bold ")?;
        formatter.push_label("stop_bold");
        write!(formatter, " stop bold ")?;
        formatter.pop_label();
        write!(formatter, " bold again ")?;
        formatter.pop_label();
        write!(formatter, " not bold again ")?;
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @"[3m[4m[38;5;1m[48;5;4m not bold [1m bold [0m[3m[4m[38;5;1m[48;5;4m stop bold [1m bold again [0m[3m[4m[38;5;1m[48;5;4m not bold again [23m[24m[39m[49m[EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_dim_reset() -> TestResult {
        // Test that we don't lose other attributes when we reset the dim attribute.
        let config = config_from_string(indoc! {"
            [colors]
            not_dim = { fg = 'red', bg = 'blue', italic = true, underline = true }
            dim_font = { dim = true }
            stop_dim = { dim = false }
        "});
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("not_dim");
        write!(formatter, " not dim ")?;
        formatter.push_label("dim_font");
        write!(formatter, " dim ")?;
        formatter.push_label("stop_dim");
        write!(formatter, " stop dim ")?;
        formatter.pop_label();
        write!(formatter, " dim again ")?;
        formatter.pop_label();
        write!(formatter, " not dim again ")?;
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @"[3m[4m[38;5;1m[48;5;4m not dim [2m dim [0m[3m[4m[38;5;1m[48;5;4m stop dim [2m dim again [0m[3m[4m[38;5;1m[48;5;4m not dim again [23m[24m[39m[49m[EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_bold_to_dim() -> TestResult {
        // Test that we don't lose bold when we reset the dim attribute.
        let config = config_from_string(indoc! {"
            [colors]
            bold_font = { bold = true }
            dim_font = { dim = true }
        "});
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("bold_font");
        write!(formatter, " bold ")?;
        formatter.push_label("dim_font");
        write!(formatter, " bold&dim ")?;
        formatter.pop_label();
        write!(formatter, " bold again ")?;
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @"[1m bold [2m bold&dim [0m[1m bold again [0m[EOF]");
        Ok(())
    }

    #[test]
    fn test_formatter_reset_on_flush() -> TestResult {
        let config = config_from_string("colors.red = 'red'");
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("red");
        write!(formatter, "foo")?;
        formatter.pop_label();

        // without flush()
        insta::assert_snapshot!(
            to_snapshot_string(formatter.output.clone()), @"[38;5;1mfoo[EOF]");

        // flush() should emit the reset sequence.
        formatter.flush()?;
        insta::assert_snapshot!(
            to_snapshot_string(formatter.output.clone()), @"[38;5;1mfoo[39m[EOF]");

        // New color sequence should be emitted as the state was reset.
        formatter.push_label("red");
        write!(formatter, "bar")?;
        formatter.pop_label();

        // drop() should emit the reset sequence.
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @"[38;5;1mfoo[39m[38;5;1mbar[39m[EOF]");

        // plaintext and sanitizing formatters produce no special behavior
        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        formatter.push_label("red");
        write!(formatter, "foo")?;
        formatter.pop_label();
        formatter.flush()?;
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"foo[EOF]");

        let mut output: Vec<u8> = vec![];
        let mut formatter = SanitizingFormatter::new(&mut output);
        formatter.push_label("red");
        write!(formatter, "foo")?;
        formatter.pop_label();
        formatter.flush()?;
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"foo[EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_no_space() -> TestResult {
        // Test that two different colors can touch.
        let config = config_from_string(
            r#"
        colors.red = "red"
        colors.green = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        write!(formatter, "before")?;
        formatter.push_label("red");
        write!(formatter, "first")?;
        formatter.pop_label();
        formatter.push_label("green");
        write!(formatter, "second")?;
        formatter.pop_label();
        write!(formatter, "after")?;
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @"before[38;5;1mfirst[38;5;2msecond[39mafter[EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_ansi_codes_in_text() -> TestResult {
        // Test that ANSI codes in the input text are escaped.
        let config = config_from_string(
            r#"
        colors.red = "red"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("red");
        write!(formatter, "\x1b[1mnot actually bold\x1b[0m")?;
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @"[38;5;1m␛[1mnot actually bold␛[0m[39m[EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_nested() -> TestResult {
        // A color can be associated with a combination of labels. A more specific match
        // overrides a less specific match. After the inner label is removed, the outer
        // color is used again (we don't reset).
        let config = config_from_string(
            r#"
        colors.outer = "blue"
        colors.inner = "red"
        colors."outer inner" = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        write!(formatter, " before outer ")?;
        formatter.push_label("outer");
        write!(formatter, " before inner ")?;
        formatter.push_label("inner");
        write!(formatter, " inside inner ")?;
        formatter.pop_label();
        write!(formatter, " after inner ")?;
        formatter.pop_label();
        write!(formatter, " after outer ")?;
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @" before outer [38;5;4m before inner [38;5;2m inside inner [38;5;4m after inner [39m after outer [EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_partial_match() -> TestResult {
        // A partial match doesn't count
        let config = config_from_string(
            r#"
        colors."outer inner" = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("outer");
        write!(formatter, " not colored ")?;
        formatter.push_label("inner");
        write!(formatter, " colored ")?;
        formatter.pop_label();
        write!(formatter, " not colored ")?;
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @" not colored [38;5;2m colored [39m not colored [EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_unrecognized_color() -> TestResult {
        // An unrecognized color causes an error.
        let config = config_from_string(
            r#"
        colors."outer" = "red"
        colors."outer inner" = "bloo"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let err = ColorFormatter::for_config(&mut output, &config, false).unwrap_err();
        insta::assert_snapshot!(err, @r#"Invalid type or value for colors."outer inner""#);
        insta::assert_snapshot!(err.source().unwrap(), @"Invalid color: bloo");
        Ok(())
    }

    #[test]
    fn test_color_formatter_unrecognized_ansi256_color() -> TestResult {
        // An unrecognized ANSI color causes an error.
        let config = config_from_string(
            r##"
            colors."outer" = "red"
            colors."outer inner" = "ansi-color-256"
            "##,
        );
        let mut output: Vec<u8> = vec![];
        let err = ColorFormatter::for_config(&mut output, &config, false).unwrap_err();
        insta::assert_snapshot!(err, @r#"Invalid type or value for colors."outer inner""#);
        insta::assert_snapshot!(err.source().unwrap(), @"Invalid color: ansi-color-256");
        Ok(())
    }

    #[test]
    fn test_color_formatter_unrecognized_hex_color() -> TestResult {
        // An unrecognized hex color causes an error.
        let config = config_from_string(
            r##"
            colors."outer" = "red"
            colors."outer inner" = "#ffgggg"
            "##,
        );
        let mut output: Vec<u8> = vec![];
        let err = ColorFormatter::for_config(&mut output, &config, false).unwrap_err();
        insta::assert_snapshot!(err, @r#"Invalid type or value for colors."outer inner""#);
        insta::assert_snapshot!(err.source().unwrap(), @"Invalid color: #ffgggg");
        Ok(())
    }

    #[test]
    fn test_color_formatter_invalid_type_of_color() -> TestResult {
        let config = config_from_string("colors.foo = []");
        let err = ColorFormatter::for_config(&mut Vec::new(), &config, false).unwrap_err();
        insta::assert_snapshot!(err, @"Invalid type or value for colors.foo");
        insta::assert_snapshot!(
            err.source().unwrap(),
            @"invalid type: array, expected a color name or a table of styles");
        Ok(())
    }

    #[test]
    fn test_color_formatter_invalid_type_of_style() -> TestResult {
        let config = config_from_string("colors.foo = { bold = 1 }");
        let err = ColorFormatter::for_config(&mut Vec::new(), &config, false).unwrap_err();
        insta::assert_snapshot!(err, @"Invalid type or value for colors.foo");
        insta::assert_snapshot!(err.source().unwrap(), @"
        invalid type: integer `1`, expected a boolean
        in `bold`
        ");
        Ok(())
    }

    #[test]
    fn test_color_formatter_normal_color() -> TestResult {
        // The "default" color resets the color. It is possible to reset only the
        // background or only the foreground.
        let config = config_from_string(
            r#"
        colors."outer" = {bg="yellow", fg="blue"}
        colors."outer default_fg" = "default"
        colors."outer default_bg" = {bg = "default"}
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("outer");
        write!(formatter, "Blue on yellow, ")?;
        formatter.push_label("default_fg");
        write!(formatter, " default fg, ")?;
        formatter.pop_label();
        write!(formatter, " and back.\nBlue on yellow, ")?;
        formatter.push_label("default_bg");
        write!(formatter, " default bg, ")?;
        formatter.pop_label();
        write!(formatter, " and back.")?;
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"
        [38;5;4m[48;5;3mBlue on yellow, [39m default fg, [38;5;4m and back.[39m[49m
        [38;5;4m[48;5;3mBlue on yellow, [49m default bg, [48;5;3m and back.[39m[49m[EOF]
        ");
        Ok(())
    }

    #[test]
    fn test_color_formatter_sibling() -> TestResult {
        // A partial match on one rule does not eliminate other rules.
        let config = config_from_string(
            r#"
        colors."outer1 inner1" = "red"
        colors.inner2 = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("outer1");
        formatter.push_label("inner2");
        write!(formatter, " hello ")?;
        formatter.pop_label();
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"[38;5;2m hello [39m[EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_reverse_order() -> TestResult {
        // Rules don't match labels out of order
        let config = config_from_string(
            r#"
        colors."inner outer" = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("outer");
        formatter.push_label("inner");
        write!(formatter, " hello ")?;
        formatter.pop_label();
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @" hello [EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_innermost_wins() -> TestResult {
        // When two labels match, the innermost one wins.
        let config = config_from_string(
            r#"
        colors."a" = "red"
        colors."b" = "green"
        colors."a c" = "blue"
        colors."b c" = "yellow"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("a");
        write!(formatter, " a1 ")?;
        formatter.push_label("b");
        write!(formatter, " b1 ")?;
        formatter.push_label("c");
        write!(formatter, " c ")?;
        formatter.pop_label();
        write!(formatter, " b2 ")?;
        formatter.pop_label();
        write!(formatter, " a2 ")?;
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @"[38;5;1m a1 [38;5;2m b1 [38;5;3m c [38;5;2m b2 [38;5;1m a2 [39m[EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_dropped() -> TestResult {
        // Test that the style gets reset if the formatter is dropped without popping
        // all labels.
        let config = config_from_string(
            r#"
        colors.outer = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false)?;
        formatter.push_label("outer");
        formatter.push_label("inner");
        write!(formatter, " inside ")?;
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"[38;5;2m inside [39m[EOF]");
        Ok(())
    }

    #[test]
    fn test_color_formatter_debug() -> TestResult {
        // Behaves like the color formatter, but surrounds each write with <<...>>,
        // adding the active labels before the actual content separated by a ::.
        let config = config_from_string(
            r#"
        colors.outer = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, true)?;
        formatter.push_label("outer");
        formatter.push_label("inner");
        write!(formatter, " inside ")?;
        formatter.pop_label();
        formatter.pop_label();
        // Matching debug styles are not separated.
        formatter.push_label("outer");
        formatter.push_label("inner");
        write!(formatter, " inside two ")?;
        formatter.pop_label();
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @"[38;5;2m<<outer inner:: inside  inside two >>[39m[EOF]",
        );
        Ok(())
    }
}
