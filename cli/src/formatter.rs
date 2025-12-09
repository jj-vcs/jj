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
use std::fmt;
use std::io;
use std::io::Error;
use std::io::Write;
use std::mem;
use std::ops::Deref;
use std::ops::DerefMut;
use std::ops::Range;
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
use serde::de::Deserialize as _;
use serde::de::Error as _;
use serde::de::IntoDeserializer as _;

// Lets the caller label strings and translates the labels to colors
pub trait Formatter: Write {
    /// Returns the backing `Write`. This is useful for writing data that is
    /// already formatted, such as in the graphical log.
    fn raw(&mut self) -> io::Result<Box<dyn Write + '_>>;

    fn push_label(&mut self, label: &str);

    fn pop_label(&mut self);
}

impl<T: Formatter + ?Sized> Formatter for &mut T {
    fn raw(&mut self) -> io::Result<Box<dyn Write + '_>> {
        <T as Formatter>::raw(self)
    }

    fn push_label(&mut self, label: &str) {
        <T as Formatter>::push_label(self, label);
    }

    fn pop_label(&mut self) {
        <T as Formatter>::pop_label(self);
    }
}

impl<T: Formatter + ?Sized> Formatter for Box<T> {
    fn raw(&mut self) -> io::Result<Box<dyn Write + '_>> {
        <T as Formatter>::raw(self)
    }

    fn push_label(&mut self, label: &str) {
        <T as Formatter>::push_label(self, label);
    }

    fn pop_label(&mut self) {
        <T as Formatter>::pop_label(self);
    }
}

/// [`Formatter`] adapters.
pub trait FormatterExt: Formatter {
    fn labeled(&mut self, label: &str) -> LabeledScope<&mut Self> {
        LabeledScope::new(self, label)
    }

    fn into_labeled(self, label: &str) -> LabeledScope<Self>
    where
        Self: Sized,
    {
        LabeledScope::new(self, label)
    }
}

impl<T: Formatter + ?Sized> FormatterExt for T {}

/// [`Formatter`] wrapper to apply a label within a lexical scope.
#[must_use]
pub struct LabeledScope<T: Formatter> {
    formatter: T,
}

impl<T: Formatter> LabeledScope<T> {
    pub fn new(mut formatter: T, label: &str) -> Self {
        formatter.push_label(label);
        Self { formatter }
    }

    // TODO: move to FormatterExt?
    /// Turns into writer that prints labeled message with the `heading`.
    pub fn with_heading<H>(self, heading: H) -> HeadingLabeledWriter<T, H> {
        HeadingLabeledWriter::new(self, heading)
    }
}

impl<T: Formatter> Drop for LabeledScope<T> {
    fn drop(&mut self) {
        self.formatter.pop_label();
    }
}

impl<T: Formatter> Deref for LabeledScope<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.formatter
    }
}

impl<T: Formatter> DerefMut for LabeledScope<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.formatter
    }
}

// There's no `impl Formatter for LabeledScope<T>` so nested .labeled() calls
// wouldn't construct `LabeledScope<LabeledScope<T>>`.

/// [`Formatter`] wrapper that prints the `heading` once.
///
/// The `heading` will be printed within the first `write!()` or `writeln!()`
/// invocation, which is handy because `io::Error` can be handled there.
pub struct HeadingLabeledWriter<T: Formatter, H> {
    formatter: LabeledScope<T>,
    heading: Option<H>,
}

impl<T: Formatter, H> HeadingLabeledWriter<T, H> {
    pub fn new(formatter: LabeledScope<T>, heading: H) -> Self {
        Self {
            formatter,
            heading: Some(heading),
        }
    }
}

impl<T: Formatter, H: fmt::Display> HeadingLabeledWriter<T, H> {
    pub fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> io::Result<()> {
        if let Some(heading) = self.heading.take() {
            write!(self.formatter.labeled("heading"), "{heading}")?;
        }
        self.formatter.write_fmt(args)
    }
}

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

    pub fn is_color(&self) -> bool {
        matches!(self.kind, FormatterFactoryKind::Color { .. })
    }
}

pub struct PlainTextFormatter<W> {
    output: W,
}

impl<W> PlainTextFormatter<W> {
    pub fn new(output: W) -> Self {
        Self { output }
    }
}

impl<W: Write> Write for PlainTextFormatter<W> {
    fn write(&mut self, data: &[u8]) -> Result<usize, Error> {
        self.output.write(data)
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.output.flush()
    }
}

impl<W: Write> Formatter for PlainTextFormatter<W> {
    fn raw(&mut self) -> io::Result<Box<dyn Write + '_>> {
        Ok(Box::new(self.output.by_ref()))
    }

    fn push_label(&mut self, _label: &str) {}

    fn pop_label(&mut self) {}
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
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Style {
    #[serde(deserialize_with = "deserialize_color_opt")]
    pub fg: Option<Color>,
    #[serde(deserialize_with = "deserialize_color_opt")]
    pub bg: Option<Color>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub reverse: Option<bool>,
}

impl Style {
    fn merge(&mut self, other: &Self) {
        self.fg = other.fg.or(self.fg);
        self.bg = other.bg.or(self.bg);
        self.bold = other.bold.or(self.bold);
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
            if new_style.bold != self.current_style.bold {
                if new_style.bold.unwrap_or_default() {
                    queue!(self.output, SetAttribute(Attribute::Bold))?;
                } else {
                    // NoBold results in double underlining on some terminals, so we use reset
                    // instead. However, that resets other attributes as well, so we reset
                    // our record of the current style so we re-apply the other attributes
                    // below.
                    queue!(self.output, SetAttribute(Attribute::Reset))?;
                    self.current_style = Style::default();
                }
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
}

impl<W: Write> Drop for ColorFormatter<W> {
    fn drop(&mut self) {
        // If a `ColorFormatter` was dropped without flushing, let's try to
        // reset any currently active style.
        self.labels.clear();
        self.write_new_style().ok();
    }
}

/// Like buffered formatter, but records `push`/`pop_label()` calls.
///
/// This allows you to manipulate the recorded data without losing labels.
/// The recorded data and labels can be written to another formatter. If
/// the destination formatter has already been labeled, the recorded labels
/// will be stacked on top of the existing labels, and the subsequent data
/// may be colorized differently.
#[derive(Clone, Debug, Default)]
pub struct FormatRecorder {
    data: Vec<u8>,
    ops: Vec<(usize, FormatOp)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FormatOp {
    PushLabel(String),
    PopLabel,
    RawEscapeSequence(Vec<u8>),
}

impl FormatRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates new buffer containing the given `data`.
    pub fn with_data(data: impl Into<Vec<u8>>) -> Self {
        Self {
            data: data.into(),
            ops: vec![],
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    fn push_op(&mut self, op: FormatOp) {
        self.ops.push((self.data.len(), op));
    }

    pub fn replay(&self, formatter: &mut dyn Formatter) -> io::Result<()> {
        self.replay_with(formatter, |formatter, range| {
            formatter.write_all(&self.data[range])
        })
    }

    pub fn replay_with(
        &self,
        formatter: &mut dyn Formatter,
        mut write_data: impl FnMut(&mut dyn Formatter, Range<usize>) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut last_pos = 0;
        let mut flush_data = |formatter: &mut dyn Formatter, pos| -> io::Result<()> {
            if last_pos != pos {
                write_data(formatter, last_pos..pos)?;
                last_pos = pos;
            }
            Ok(())
        };
        for (pos, op) in &self.ops {
            flush_data(formatter, *pos)?;
            match op {
                FormatOp::PushLabel(label) => formatter.push_label(label),
                FormatOp::PopLabel => formatter.pop_label(),
                FormatOp::RawEscapeSequence(raw_escape_sequence) => {
                    formatter.raw()?.write_all(raw_escape_sequence)?;
                }
            }
        }
        flush_data(formatter, self.data.len())
    }
}

impl Write for FormatRecorder {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.data.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct RawEscapeSequenceRecorder<'a>(&'a mut FormatRecorder);

impl Write for RawEscapeSequenceRecorder<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.push_op(FormatOp::RawEscapeSequence(data.to_vec()));
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Formatter for FormatRecorder {
    fn raw(&mut self) -> io::Result<Box<dyn Write + '_>> {
        Ok(Box::new(RawEscapeSequenceRecorder(self)))
    }

    fn push_label(&mut self, label: &str) {
        self.push_op(FormatOp::PushLabel(label.to_owned()));
    }

    fn pop_label(&mut self) {
        self.push_op(FormatOp::PopLabel);
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
    fn test_plaintext_formatter() {
        // Test that PlainTextFormatter ignores labels.
        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        formatter.push_label("warning");
        write!(formatter, "hello").unwrap();
        formatter.pop_label();
        insta::assert_snapshot!(to_snapshot_string(output), @"hello[EOF]");
    }

    #[test]
    fn test_plaintext_formatter_ansi_codes_in_text() {
        // Test that ANSI codes in the input text are NOT escaped.
        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        write!(formatter, "\x1b[1mactually bold\x1b[0m").unwrap();
        insta::assert_snapshot!(to_snapshot_string(output), @"[1mactually bold[0m[EOF]");
    }

    #[test]
    fn test_sanitizing_formatter_ansi_codes_in_text() {
        // Test that ANSI codes in the input text are escaped.
        let mut output: Vec<u8> = vec![];
        let mut formatter = SanitizingFormatter::new(&mut output);
        write!(formatter, "\x1b[1mnot actually bold\x1b[0m").unwrap();
        insta::assert_snapshot!(to_snapshot_string(output), @"␛[1mnot actually bold␛[0m[EOF]");
    }

    #[test]
    fn test_color_formatter_color_codes() {
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
        let colors: IndexMap<String, String> = config.get("colors").unwrap();
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        for (label, color) in &colors {
            formatter.push_label(label);
            write!(formatter, " {color} ").unwrap();
            formatter.pop_label();
            writeln!(formatter).unwrap();
        }
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @r"
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
    }

    #[test]
    fn test_color_for_ansi256_index() {
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
    }

    #[test]
    fn test_color_formatter_ansi256() {
        let config = config_from_string(
            r#"
        [colors]
        purple-bg = { fg = "ansi-color-15", bg = "ansi-color-93" }
        gray = "ansi-color-244"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        formatter.push_label("purple-bg");
        write!(formatter, " purple background ").unwrap();
        formatter.pop_label();
        writeln!(formatter).unwrap();
        formatter.push_label("gray");
        write!(formatter, " gray ").unwrap();
        formatter.pop_label();
        writeln!(formatter).unwrap();
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @r"
        [38;5;15m[48;5;93m purple background [39m[49m
        [38;5;244m gray [39m
        [EOF]
        ");
    }

    #[test]
    fn test_color_formatter_hex_colors() {
        // Test the color code for each color.
        let config = config_from_string(indoc! {"
            [colors]
            black = '#000000'
            white = '#ffffff'
            pastel-blue = '#AFE0D9'
        "});
        let colors: IndexMap<String, String> = config.get("colors").unwrap();
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        for label in colors.keys() {
            formatter.push_label(&label.replace(' ', "-"));
            write!(formatter, " {label} ").unwrap();
            formatter.pop_label();
            writeln!(formatter).unwrap();
        }
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @r"
        [38;2;0;0;0m black [39m
        [38;2;255;255;255m white [39m
        [38;2;175;224;217m pastel-blue [39m
        [EOF]
        ");
    }

    #[test]
    fn test_color_formatter_single_label() {
        // Test that a single label can be colored and that the color is reset
        // afterwards.
        let config = config_from_string(
            r#"
        colors.inside = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        write!(formatter, " before ").unwrap();
        formatter.push_label("inside");
        write!(formatter, " inside ").unwrap();
        formatter.pop_label();
        write!(formatter, " after ").unwrap();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @" before [38;5;2m inside [39m after [EOF]");
    }

    #[test]
    fn test_color_formatter_attributes() {
        // Test that each attribute of the style can be set and that they can be
        // combined in a single rule or by using multiple rules.
        let config = config_from_string(
            r#"
        colors.red_fg = { fg = "red" }
        colors.blue_bg = { bg = "blue" }
        colors.bold_font = { bold = true }
        colors.italic_text = { italic = true }
        colors.underlined_text = { underline = true }
        colors.reversed_colors = { reverse = true }
        colors.multiple = { fg = "green", bg = "yellow", bold = true, italic = true, underline = true, reverse = true }
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        formatter.push_label("red_fg");
        write!(formatter, " fg only ").unwrap();
        formatter.pop_label();
        writeln!(formatter).unwrap();
        formatter.push_label("blue_bg");
        write!(formatter, " bg only ").unwrap();
        formatter.pop_label();
        writeln!(formatter).unwrap();
        formatter.push_label("bold_font");
        write!(formatter, " bold only ").unwrap();
        formatter.pop_label();
        writeln!(formatter).unwrap();
        formatter.push_label("italic_text");
        write!(formatter, " italic only ").unwrap();
        formatter.pop_label();
        writeln!(formatter).unwrap();
        formatter.push_label("underlined_text");
        write!(formatter, " underlined only ").unwrap();
        formatter.pop_label();
        writeln!(formatter).unwrap();
        formatter.push_label("reversed_colors");
        write!(formatter, " reverse only ").unwrap();
        formatter.pop_label();
        writeln!(formatter).unwrap();
        formatter.push_label("multiple");
        write!(formatter, " single rule ").unwrap();
        formatter.pop_label();
        writeln!(formatter).unwrap();
        formatter.push_label("red_fg");
        formatter.push_label("blue_bg");
        write!(formatter, " two rules ").unwrap();
        formatter.pop_label();
        formatter.pop_label();
        writeln!(formatter).unwrap();
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @r"
        [38;5;1m fg only [39m
        [48;5;4m bg only [49m
        [1m bold only [0m
        [3m italic only [23m
        [4m underlined only [24m
        [7m reverse only [27m
        [1m[3m[4m[7m[38;5;2m[48;5;3m single rule [0m
        [38;5;1m[48;5;4m two rules [39m[49m
        [EOF]
        ");
    }

    #[test]
    fn test_color_formatter_bold_reset() {
        // Test that we don't lose other attributes when we reset the bold attribute.
        let config = config_from_string(
            r#"
        colors.not_bold = { fg = "red", bg = "blue", italic = true, underline = true }
        colors.bold_font = { bold = true }
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        formatter.push_label("not_bold");
        write!(formatter, " not bold ").unwrap();
        formatter.push_label("bold_font");
        write!(formatter, " bold ").unwrap();
        formatter.pop_label();
        write!(formatter, " not bold again ").unwrap();
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @"[3m[4m[38;5;1m[48;5;4m not bold [1m bold [0m[3m[4m[38;5;1m[48;5;4m not bold again [23m[24m[39m[49m[EOF]");
    }

    #[test]
    fn test_color_formatter_reset_on_flush() {
        let config = config_from_string("colors.red = 'red'");
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        formatter.push_label("red");
        write!(formatter, "foo").unwrap();
        formatter.pop_label();

        // without flush()
        insta::assert_snapshot!(
            to_snapshot_string(formatter.output.clone()), @"[38;5;1mfoo[EOF]");

        // flush() should emit the reset sequence.
        formatter.flush().unwrap();
        insta::assert_snapshot!(
            to_snapshot_string(formatter.output.clone()), @"[38;5;1mfoo[39m[EOF]");

        // New color sequence should be emitted as the state was reset.
        formatter.push_label("red");
        write!(formatter, "bar").unwrap();
        formatter.pop_label();

        // drop() should emit the reset sequence.
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @"[38;5;1mfoo[39m[38;5;1mbar[39m[EOF]");
    }

    #[test]
    fn test_color_formatter_no_space() {
        // Test that two different colors can touch.
        let config = config_from_string(
            r#"
        colors.red = "red"
        colors.green = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        write!(formatter, "before").unwrap();
        formatter.push_label("red");
        write!(formatter, "first").unwrap();
        formatter.pop_label();
        formatter.push_label("green");
        write!(formatter, "second").unwrap();
        formatter.pop_label();
        write!(formatter, "after").unwrap();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @"before[38;5;1mfirst[38;5;2msecond[39mafter[EOF]");
    }

    #[test]
    fn test_color_formatter_ansi_codes_in_text() {
        // Test that ANSI codes in the input text are escaped.
        let config = config_from_string(
            r#"
        colors.red = "red"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        formatter.push_label("red");
        write!(formatter, "\x1b[1mnot actually bold\x1b[0m").unwrap();
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @"[38;5;1m␛[1mnot actually bold␛[0m[39m[EOF]");
    }

    #[test]
    fn test_color_formatter_nested() {
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
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        write!(formatter, " before outer ").unwrap();
        formatter.push_label("outer");
        write!(formatter, " before inner ").unwrap();
        formatter.push_label("inner");
        write!(formatter, " inside inner ").unwrap();
        formatter.pop_label();
        write!(formatter, " after inner ").unwrap();
        formatter.pop_label();
        write!(formatter, " after outer ").unwrap();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @" before outer [38;5;4m before inner [38;5;2m inside inner [38;5;4m after inner [39m after outer [EOF]");
    }

    #[test]
    fn test_color_formatter_partial_match() {
        // A partial match doesn't count
        let config = config_from_string(
            r#"
        colors."outer inner" = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        formatter.push_label("outer");
        write!(formatter, " not colored ").unwrap();
        formatter.push_label("inner");
        write!(formatter, " colored ").unwrap();
        formatter.pop_label();
        write!(formatter, " not colored ").unwrap();
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @" not colored [38;5;2m colored [39m not colored [EOF]");
    }

    #[test]
    fn test_color_formatter_unrecognized_color() {
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
    }

    #[test]
    fn test_color_formatter_unrecognized_ansi256_color() {
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
    }

    #[test]
    fn test_color_formatter_unrecognized_hex_color() {
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
    }

    #[test]
    fn test_color_formatter_invalid_type_of_color() {
        let config = config_from_string("colors.foo = []");
        let err = ColorFormatter::for_config(&mut Vec::new(), &config, false).unwrap_err();
        insta::assert_snapshot!(err, @"Invalid type or value for colors.foo");
        insta::assert_snapshot!(
            err.source().unwrap(),
            @"invalid type: array, expected a color name or a table of styles");
    }

    #[test]
    fn test_color_formatter_invalid_type_of_style() {
        let config = config_from_string("colors.foo = { bold = 1 }");
        let err = ColorFormatter::for_config(&mut Vec::new(), &config, false).unwrap_err();
        insta::assert_snapshot!(err, @"Invalid type or value for colors.foo");
        insta::assert_snapshot!(err.source().unwrap(), @r"
        invalid type: integer `1`, expected a boolean
        in `bold`
        ");
    }

    #[test]
    fn test_color_formatter_normal_color() {
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
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        formatter.push_label("outer");
        write!(formatter, "Blue on yellow, ").unwrap();
        formatter.push_label("default_fg");
        write!(formatter, " default fg, ").unwrap();
        formatter.pop_label();
        write!(formatter, " and back.\nBlue on yellow, ").unwrap();
        formatter.push_label("default_bg");
        write!(formatter, " default bg, ").unwrap();
        formatter.pop_label();
        write!(formatter, " and back.").unwrap();
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @r"
        [38;5;4m[48;5;3mBlue on yellow, [39m default fg, [38;5;4m and back.[39m[49m
        [38;5;4m[48;5;3mBlue on yellow, [49m default bg, [48;5;3m and back.[39m[49m[EOF]
        ");
    }

    #[test]
    fn test_color_formatter_sibling() {
        // A partial match on one rule does not eliminate other rules.
        let config = config_from_string(
            r#"
        colors."outer1 inner1" = "red"
        colors.inner2 = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        formatter.push_label("outer1");
        formatter.push_label("inner2");
        write!(formatter, " hello ").unwrap();
        formatter.pop_label();
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"[38;5;2m hello [39m[EOF]");
    }

    #[test]
    fn test_color_formatter_reverse_order() {
        // Rules don't match labels out of order
        let config = config_from_string(
            r#"
        colors."inner outer" = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        formatter.push_label("outer");
        formatter.push_label("inner");
        write!(formatter, " hello ").unwrap();
        formatter.pop_label();
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @" hello [EOF]");
    }

    #[test]
    fn test_color_formatter_innermost_wins() {
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
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        formatter.push_label("a");
        write!(formatter, " a1 ").unwrap();
        formatter.push_label("b");
        write!(formatter, " b1 ").unwrap();
        formatter.push_label("c");
        write!(formatter, " c ").unwrap();
        formatter.pop_label();
        write!(formatter, " b2 ").unwrap();
        formatter.pop_label();
        write!(formatter, " a2 ").unwrap();
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @"[38;5;1m a1 [38;5;2m b1 [38;5;3m c [38;5;2m b2 [38;5;1m a2 [39m[EOF]");
    }

    #[test]
    fn test_color_formatter_dropped() {
        // Test that the style gets reset if the formatter is dropped without popping
        // all labels.
        let config = config_from_string(
            r#"
        colors.outer = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        formatter.push_label("outer");
        formatter.push_label("inner");
        write!(formatter, " inside ").unwrap();
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"[38;5;2m inside [39m[EOF]");
    }

    #[test]
    fn test_color_formatter_debug() {
        // Behaves like the color formatter, but surrounds each write with <<...>>,
        // adding the active labels before the actual content separated by a ::.
        let config = config_from_string(
            r#"
        colors.outer = "green"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, true).unwrap();
        formatter.push_label("outer");
        formatter.push_label("inner");
        write!(formatter, " inside ").unwrap();
        formatter.pop_label();
        formatter.pop_label();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @"[38;5;2m<<outer inner:: inside >>[39m[EOF]");
    }

    #[test]
    fn test_labeled_scope() {
        let config = config_from_string(indoc! {"
            [colors]
            outer = 'blue'
            inner = 'red'
            'outer inner' = 'green'
        "});
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        writeln!(formatter.labeled("outer"), "outer").unwrap();
        writeln!(formatter.labeled("outer").labeled("inner"), "outer-inner").unwrap();
        writeln!(formatter.labeled("inner"), "inner").unwrap();
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @r"
        [38;5;4mouter[39m
        [38;5;2mouter-inner[39m
        [38;5;1minner[39m
        [EOF]
        ");
    }

    #[test]
    fn test_heading_labeled_writer() {
        let config = config_from_string(
            r#"
        colors.inner = "green"
        colors."inner heading" = "red"
        "#,
        );
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        formatter.labeled("inner").with_heading("Should be noop: ");
        let mut writer = formatter.labeled("inner").with_heading("Heading: ");
        write!(writer, "Message").unwrap();
        writeln!(writer, " continues").unwrap();
        drop(writer);
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @r"
        [38;5;1mHeading: [38;5;2mMessage continues[39m
        [EOF]
        ");
    }

    #[test]
    fn test_heading_labeled_writer_empty_string() {
        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        let mut writer = formatter.labeled("inner").with_heading("Heading: ");
        // write_fmt() is called even if the format string is empty. I don't
        // know if that's guaranteed, but let's record the current behavior.
        write!(writer, "").unwrap();
        write!(writer, "").unwrap();
        drop(writer);
        insta::assert_snapshot!(to_snapshot_string(output), @"Heading: [EOF]");
    }

    #[test]
    fn test_format_recorder() {
        let mut recorder = FormatRecorder::new();
        write!(recorder, " outer1 ").unwrap();
        recorder.push_label("inner");
        write!(recorder, " inner1 ").unwrap();
        write!(recorder, " inner2 ").unwrap();
        recorder.pop_label();
        write!(recorder, " outer2 ").unwrap();

        insta::assert_snapshot!(
            to_snapshot_string(recorder.data()),
            @" outer1  inner1  inner2  outer2 [EOF]");

        // Replayed output should be labeled.
        let config = config_from_string(r#" colors.inner = "red" "#);
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        recorder.replay(&mut formatter).unwrap();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @" outer1 [38;5;1m inner1  inner2 [39m outer2 [EOF]");

        // Replayed output should be split at push/pop_label() call.
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        recorder
            .replay_with(&mut formatter, |formatter, range| {
                let data = &recorder.data()[range];
                write!(formatter, "<<{}>>", str::from_utf8(data).unwrap())
            })
            .unwrap();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @"<< outer1 >>[38;5;1m<< inner1  inner2 >>[39m<< outer2 >>[EOF]");
    }

    #[test]
    fn test_raw_format_recorder() {
        // Note: similar to test_format_recorder above
        let mut recorder = FormatRecorder::new();
        write!(recorder.raw().unwrap(), " outer1 ").unwrap();
        recorder.push_label("inner");
        write!(recorder.raw().unwrap(), " inner1 ").unwrap();
        write!(recorder.raw().unwrap(), " inner2 ").unwrap();
        recorder.pop_label();
        write!(recorder.raw().unwrap(), " outer2 ").unwrap();

        // Replayed raw escape sequences are labeled.
        let config = config_from_string(r#" colors.inner = "red" "#);
        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        recorder.replay(&mut formatter).unwrap();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @" outer1 [38;5;1m inner1  inner2 [39m outer2 [EOF]");

        let mut output: Vec<u8> = vec![];
        let mut formatter = ColorFormatter::for_config(&mut output, &config, false).unwrap();
        recorder
            .replay_with(&mut formatter, |_formatter, range| {
                panic!(
                    "Called with {:?} when all output should be raw",
                    str::from_utf8(&recorder.data()[range]).unwrap()
                );
            })
            .unwrap();
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @" outer1 [38;5;1m inner1  inner2 [39m outer2 [EOF]");
    }
}
