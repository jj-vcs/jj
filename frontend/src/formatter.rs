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

//! Trait for writing or buffering labeled strings.

use std::fmt;
use std::io;
use std::io::Error;
use std::io::Write;
use std::ops::Deref;
use std::ops::DerefMut;
use std::ops::Range;

/// Lets the caller label strings and translates the labels to colors
pub trait Formatter: Write {
    /// Returns the backing `Write`. This is useful for writing data that is
    /// already formatted, such as in the graphical log.
    fn raw(&mut self) -> io::Result<Box<dyn Write + '_>>;

    /// Pushes a label to the formatter.
    fn push_label(&mut self, label: &str);

    /// Pops the last pushed label from the formatter.
    fn pop_label(&mut self);

    /// Returns whether this formatter supports colors.
    fn maybe_color(&self) -> bool;
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

    fn maybe_color(&self) -> bool {
        <T as Formatter>::maybe_color(self)
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

    fn maybe_color(&self) -> bool {
        <T as Formatter>::maybe_color(self)
    }
}

/// [`Formatter`] adapters.
pub trait FormatterExt: Formatter {
    /// Creates a new `LabeledScope` for the given `label`.
    fn labeled(&mut self, label: &str) -> LabeledScope<&mut Self> {
        LabeledScope::new(self, label)
    }

    /// Converts this formatter into a `LabelScope`.
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
    /// Creates a new `LabeledScope` with the given `label` and `formattr`.
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
    /// Creates a new `HeadingLabeledWritter` for the given `formatter` and
    /// `heading`.
    pub fn new(formatter: LabeledScope<T>, heading: H) -> Self {
        Self {
            formatter,
            heading: Some(heading),
        }
    }
}

impl<T: Formatter, H: fmt::Display> HeadingLabeledWriter<T, H> {
    /// Writes the given `args` to the underlying with the given heading if one
    /// exists.
    pub fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> io::Result<()> {
        if let Some(heading) = self.heading.take() {
            write!(self.formatter.labeled("heading"), "{heading}")?;
        }
        self.formatter.write_fmt(args)
    }
}

/// A Formatter which only outputs plain-text.
pub struct PlainTextFormatter<W> {
    output: W,
}

impl<W> PlainTextFormatter<W> {
    /// Creates a new `PlainTextFormatter` with the given `output` as a sink.
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

    fn maybe_color(&self) -> bool {
        false
    }
}

/// Like buffered formatter, but records `push`/`pop_label()` calls.
///
/// This allows you to manipulate the recorded data without losing labels.
/// The recorded data and labels can be written to another formatter. If
/// the destination formatter has already been labeled, the recorded labels
/// will be stacked on top of the existing labels, and the subsequent data
/// may be colorized differently.
#[derive(Clone, Debug)]
pub struct FormatRecorder {
    data: Vec<u8>,
    ops: Vec<(usize, FormatOp)>,
    maybe_color: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FormatOp {
    PushLabel(String),
    PopLabel,
    RawEscapeSequence(Vec<u8>),
}

impl FormatRecorder {
    /// Creates a new `FormatRecorder` with `maybe_color` indicating that it
    /// supports colored output.
    pub fn new(maybe_color: bool) -> Self {
        Self {
            data: vec![],
            ops: vec![],
            maybe_color,
        }
    }

    /// Creates new buffer containing the given `data`.
    pub fn with_data(data: impl Into<Vec<u8>>) -> Self {
        Self {
            data: data.into(),
            ops: vec![],
            maybe_color: false,
        }
    }

    /// Returns a reference to the data of the buffer we own.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Pushes an `op` to this recorder.
    fn push_op(&mut self, op: FormatOp) {
        self.ops.push((self.data.len(), op));
    }

    /// Replays all the recorded data into `formatter`.
    pub fn replay(&self, formatter: &mut dyn Formatter) -> io::Result<()> {
        self.replay_with(formatter, |formatter, range| {
            formatter.write_all(&self.data[range])
        })
    }

    /// Replays this FormatRecorder on the `formatter` with the given
    /// `write_data` function.
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

    fn maybe_color(&self) -> bool {
        self.maybe_color
    }
}

#[cfg(test)]
mod tests {
    use bstr::BString;

    use super::*;
    use crate::tests::TestResult;

    /// Appends "[EOF]" marker to the output text.
    ///
    /// This is a workaround for https://github.com/mitsuhiko/insta/issues/384.
    fn to_snapshot_string(output: impl Into<Vec<u8>>) -> BString {
        let mut output = output.into();
        output.extend_from_slice(b"[EOF]\n");
        BString::new(output)
    }

    #[test]
    fn test_plaintext_formatter() -> TestResult {
        // Test that PlainTextFormatter ignores labels.
        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        formatter.push_label("warning");
        write!(formatter, "hello")?;
        formatter.pop_label();
        insta::assert_snapshot!(to_snapshot_string(output), @"hello[EOF]");
        Ok(())
    }

    #[test]
    fn test_plaintext_formatter_ansi_codes_in_text() -> TestResult {
        // Test that ANSI codes in the input text are NOT escaped.
        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        write!(formatter, "\x1b[1mactually bold\x1b[0m")?;
        insta::assert_snapshot!(to_snapshot_string(output), @"[1mactually bold[0m[EOF]");
        Ok(())
    }

    #[test]
    fn test_labeled_scope() -> TestResult {
        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        writeln!(formatter.labeled("outer"), "outer")?;
        writeln!(formatter.labeled("outer").labeled("inner"), "outer-inner")?;
        writeln!(formatter.labeled("inner"), "inner")?;
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"
        outer
        outer-inner
        inner
        [EOF]
        ");
        Ok(())
    }

    #[test]
    fn test_heading_labeled_writer() -> TestResult {
        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        formatter.labeled("inner").with_heading("Should be noop: ");
        let mut writer = formatter.labeled("inner").with_heading("Heading: ");
        write!(writer, "Message")?;
        writeln!(writer, " continues")?;
        drop(writer);
        drop(formatter);
        insta::assert_snapshot!(to_snapshot_string(output), @"
        Heading: Message continues
        [EOF]
        ");
        Ok(())
    }

    #[test]
    fn test_heading_labeled_writer_empty_string() -> TestResult {
        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        let mut writer = formatter.labeled("inner").with_heading("Heading: ");
        // write_fmt() is called even if the format string is empty. I don't
        // know if that's guaranteed, but let's record the current behavior.
        write!(writer, "")?;
        write!(writer, "")?;
        drop(writer);
        insta::assert_snapshot!(to_snapshot_string(output), @"Heading: [EOF]");
        Ok(())
    }

    #[test]
    fn test_format_recorder() -> TestResult {
        let mut recorder = FormatRecorder::new(false);
        write!(recorder, " outer1 ")?;
        recorder.push_label("inner");
        write!(recorder, " inner1 ")?;
        write!(recorder, " inner2 ")?;
        recorder.pop_label();
        write!(recorder, " outer2 ")?;

        insta::assert_snapshot!(
            to_snapshot_string(recorder.data()),
            @" outer1  inner1  inner2  outer2 [EOF]");

        // Replayed output should preserve content.
        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        recorder.replay(&mut formatter)?;
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @" outer1  inner1  inner2  outer2 [EOF]");

        // Replayed output should be split at push/pop_label() call.
        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        recorder.replay_with(&mut formatter, |formatter, range| {
            let data = &recorder.data()[range];
            write!(formatter, "<<{}>>", str::from_utf8(data).unwrap())
        })?;
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output),
            @"<< outer1 >><< inner1  inner2 >><< outer2 >>[EOF]");
        Ok(())
    }

    #[test]
    fn test_raw_format_recorder() -> TestResult {
        // Note: similar to test_format_recorder above
        let mut recorder = FormatRecorder::new(false);
        write!(recorder.raw()?, " outer1 ")?;
        recorder.push_label("inner");
        write!(recorder.raw()?, " inner1 ")?;
        write!(recorder.raw()?, " inner2 ")?;
        recorder.pop_label();
        write!(recorder.raw()?, " outer2 ")?;

        // Replayed raw escape sequences pass through.
        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        recorder.replay(&mut formatter)?;
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @" outer1  inner1  inner2  outer2 [EOF]");

        let mut output: Vec<u8> = vec![];
        let mut formatter = PlainTextFormatter::new(&mut output);
        recorder.replay_with(&mut formatter, |_formatter, range| {
            panic!(
                "Called with {:?} when all output should be raw",
                str::from_utf8(&recorder.data()[range]).unwrap()
            );
        })?;
        drop(formatter);
        insta::assert_snapshot!(
            to_snapshot_string(output), @" outer1  inner1  inner2  outer2 [EOF]");
        Ok(())
    }
}
