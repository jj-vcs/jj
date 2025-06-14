// Copyright 2025 The Jujutsu Authors
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

use super::TargetEol;

pub(crate) trait WriteExt: io::Write {
    /// Convert the texts to write to the target EOL, and write to the original
    /// writer, [`self`].
    ///
    /// The texts to write can have any or mixed EOLs, LF or CRLF.
    fn write_with_eol(&mut self, target_eol: TargetEol) -> impl io::Write;

    /// Count the number of bytes consumed through the [`io::Write::write`]
    /// interface.
    fn count_consumed_bytes<'a>(&'a mut self) -> CountConsumedBytes<'a, Self>;
}

impl<T: io::Write> WriteExt for T {
    fn write_with_eol(&mut self, target_eol: TargetEol) -> impl io::Write {
        match target_eol {
            TargetEol::PassThrough => EolWriter::PassThrough(self),
            TargetEol::Crlf => EolWriter::Crlf(CrlfWriter {
                writer: self,
                has_pending_lf: false,
                last_write_ends_with_cr: false,
            }),
            TargetEol::Lf => EolWriter::Lf(LfWriter {
                writer: self,
                has_pending_cr: false,
            }),
        }
    }

    fn count_consumed_bytes<'a>(&'a mut self) -> CountConsumedBytes<'a, Self> {
        CountConsumedBytes {
            writer: self,
            bytes_consumed: 0,
        }
    }
}

enum EolWriter<'a, W: io::Write + ?Sized> {
    PassThrough(&'a mut W),
    Crlf(CrlfWriter<'a, W>),
    Lf(LfWriter<'a, W>),
}

impl<'a, W: io::Write + ?Sized> io::Write for EolWriter<'a, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::PassThrough(writer) => writer.write(buf),
            Self::Crlf(writer) => writer.write(buf),
            Self::Lf(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::PassThrough(writer) => writer.flush(),
            Self::Crlf(writer) => writer.flush(),
            Self::Lf(writer) => writer.flush(),
        }
    }
}

struct CrlfWriter<'a, W: io::Write + ?Sized> {
    writer: &'a mut W,
    /// Whether the last write ends with CR.
    ///
    /// When the last write ends with CR and the current write starts with LF,
    /// we shouldn't translate the LF in the current write to CRLF, because it's
    /// part of CRLF. Therefore, we must remember whether the last write ends
    /// with CR.
    last_write_ends_with_cr: bool,
    /// Whether we have a yet to write LF.
    ///
    /// If the caller writes a LF, we decide to convert it to CRLF, and we can
    /// only write CR to the underlying [`io::Write`] because
    /// [`io::Write::write`] returns a smaller value, we are left with a pending
    /// LF to be written in the next write. This is especially true because we
    /// can't revert the CR we have already written.
    has_pending_lf: bool,
}

impl<'a, W: io::Write + ?Sized> io::Write for CrlfWriter<'a, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.has_pending_lf {
            // If that last time when we try to write CRLF, we can only write the CR, try to
            // write that LF now.
            let n = self.writer.write(b"\n")?;
            if n == 0 {
                // We reach an EOF.
                return Ok(0);
            }
            self.has_pending_lf = false;
        }
        // If we are going to convert a LF to a CRLF, it can't be done in one write, so
        // we just try to write up to the place where we need to insert a CR through
        // another write, and try to write the CRLF. end_idx will stop at the LF byte
        // that needs to convert or at the end of the buffer.
        for end_idx in 0..buf.len() {
            if buf[end_idx] != b'\n' {
                // Skip the byte that is not LF.
                continue;
            }
            if (end_idx > 0) && buf[end_idx - 1] == b'\r' {
                // The current byte is LF, but can form a CRLF with a previous CR in the current
                // buffer, so this LF doesn't need conversion.
                continue;
            }
            if end_idx == 0 && self.last_write_ends_with_cr {
                // The current byte is LF, but can form a CRLF with a previous CR in the
                // previous write, so this LF doesn't need conversion.
                continue;
            }
            // Handle the LF that needs to convert to CRLF.
            let mut ret = 0;
            // We first try to write all the bytes up to this LF subject to conversion.
            let n = self.writer.write(&buf[..end_idx])?;
            ret += n;
            if n < end_idx {
                // We can't write all the bytes, but we are also guaranteed that no LF to CRLF
                // conversion is needed for the bytes written, so the number of bytes consumed
                // by the inner writer is the number of bytes consumed by us.
                if n > 0 {
                    // Check to avoid possible overflow in n - 1 if we can't write anything.
                    self.last_write_ends_with_cr = buf[n - 1] == b'\r';
                }
                return Ok(ret);
            }
            // Try to write the converted CRLF.
            let n = self.writer.write(b"\r\n")?;
            match n {
                0 => {
                    // We can't write any bytes of the CRLF. We tell the caller that we don't
                    // consume the input LF byte.
                    if end_idx > 0 {
                        // If we actually write something, change the last_write_ends_with_cr flag
                        // accordingly.
                        self.last_write_ends_with_cr = buf[end_idx - 1] == b'\r';
                    }
                    return Ok(ret);
                }
                1 => {
                    // We can only write the CR byte of the CRLF. We tell the caller that we consume
                    // the input LF byte, and set the pending LF flag to try to write the LF byte in
                    // the next write.
                    ret += 1;
                    self.has_pending_lf = true;
                    // We are actually handling an input LF in this case, not CR. If the next write
                    // starts with LF, it doesn't form a CRLF with the current write.
                    self.last_write_ends_with_cr = false;
                    return Ok(ret);
                }
                _ => {
                    // We can write the entire CRLF. Clear the pending LF flag and the end with CR
                    // flag.
                    ret += 1;
                    debug_assert!(!self.has_pending_lf);
                    self.last_write_ends_with_cr = false;
                    return Ok(ret);
                }
            }
        }
        // We don't hit a LF byte that needs conversion. Write the entire buffer as is.
        let ret = self.writer.write(buf)?;
        if ret > 0 {
            // If we actually write something, change the last_write_ends_with_cr flag
            // accordingly.
            self.last_write_ends_with_cr = buf[ret - 1] == b'\r';
        }
        Ok(ret)
    }

    fn flush(&mut self) -> io::Result<()> {
        // If we have a pending LF to write, we should try it in flush, and report an
        // error if we can't write the pending LF.
        if self.has_pending_lf && self.writer.write(b"\n")? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write the pending LF",
            ));
        }
        // We ensure that the pending LF byte is written if there exists one.
        self.has_pending_lf = false;
        self.writer.flush()
    }
}

struct LfWriter<'a, W: io::Write + ?Sized> {
    writer: &'a mut W,
    // Whether we have a CR to consume.
    //
    // If the last byte the user tries to write is CR, we shouldn't just write it, because it can
    // form a CRLF with the next write. In this case, we should just write one LF byte and skip the
    // CR byte.
    has_pending_cr: bool,
}

impl<'a, W: io::Write + ?Sized> io::Write for LfWriter<'a, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.has_pending_cr && buf[0] != b'\n' {
            // We have a CR byte to consume from the last write, and it doesn't form a CRLF
            // with the starting byte of the current write. Write the CR byte as is.
            let n = self.writer.write(b"\r")?;
            if n == 0 {
                return Ok(0);
            }
            // If we write the CR successfully, we can't just return Ok(0), because we
            // haven't reached EOF yet.
            self.has_pending_cr = false;
        }
        // Advance end_idx, so that it points to the LF byte of the first CRLF in buf,
        // where an EOL conversion is needed. If no CRLF byte is found, points to the
        // end of buf. We will then write up to the previous byte pointed by end_idx but
        // skip the possible ending CR byte.
        let mut end_idx = buf.len();
        for i in 0..buf.len() {
            if buf[i] != b'\n' {
                continue;
            }
            if i == 0 {
                continue;
            }
            if buf[i - 1] != b'\r' {
                continue;
            }
            end_idx = i;
            break;
        }
        // end_idx is either buf.len() or points to the LF byte of a CRLF sequence.
        // * If end_idx is buf.len(), buf is not empty, end_idx isn't 0.
        // * If end_idx points to the LF byte of a CRLF sequence, end_idx must follow a
        //   CR byte, so end_idx can't be 0.
        debug_assert_ne!(end_idx, 0);
        let (to_write, has_pending_cr) = if buf[end_idx - 1] == b'\r' {
            // If the buffer to write ends with CR, we don't write that CR byte.
            (&buf[..(end_idx - 1)], true)
        } else {
            (buf, false)
        };
        let n = self.writer.write(to_write)?;
        if n < to_write.len() {
            if n == 0 {
                // We reach the EOF, and we haven't consumed any bytes in buf.
                return Ok(0);
            }
            if to_write[n - 1] != b'\r' {
                // The last byte we write is not CR, so the last byte we write can't form a CRLF
                // with the next write.
                self.has_pending_cr = false;
                return Ok(n);
            }

            // The last byte we write is CR, to prevent next write from forming a CRLF with
            // this we have to consume the next byte.
            if to_write[n] == b'\r' {
                // The next byte is CR, we just consume it by storing it internally instead of
                // an actual write, because we don't know if this CR can form CRLF with the next
                // write.
                debug_assert!(!self.has_pending_cr);
                self.has_pending_cr = true;
                return Ok(n + 1);
            }
            // The next byte is not CR, write it, so that the next write can't form a CRLF.
            if self.writer.write(&to_write[n..(n + 1)])? == 0 {
                // We reach an EOF, so we can't accept the next byte, which means the last CR we
                // write is the last byte we accept and won't be able to form a CRLF.
                self.has_pending_cr = false;
                return Ok(n);
            }

            // We successfully write some bytes ending with CR, and another non CR byte.
            self.has_pending_cr = false;
            return Ok(n + 1);
        }

        // We either successfully write the entire buffer, or up to the first CR byte
        // that needs to skip due to the CRLF to LF EOL conversion.
        self.has_pending_cr = has_pending_cr;
        if has_pending_cr {
            // We consume the CR byte by setting the pending CR flag.
            Ok(n + 1)
        } else {
            Ok(n)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

impl<'a, W: io::Write + ?Sized> Drop for LfWriter<'a, W> {
    fn drop(&mut self) {
        if self.has_pending_cr {
            let _ = self.writer.write(b"\r");
        }
    }
}

pub(crate) struct CountConsumedBytes<'a, W: io::Write + ?Sized> {
    writer: &'a mut W,
    bytes_consumed: usize,
}

impl<'a, W: io::Write + ?Sized> CountConsumedBytes<'a, W> {
    pub fn bytes_consumed(&self) -> usize {
        self.bytes_consumed
    }
}

impl<'a, W: io::Write + ?Sized> io::Write for CountConsumedBytes<'a, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.writer.write(buf)?;
        self.bytes_consumed += n;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::Cursor;
    use std::io::Write;

    use test_case::test_case;

    use super::*;

    #[test_case(TargetEol::Crlf; "targeting CRLF")]
    #[test_case(TargetEol::Lf; "targeting LF")]
    #[test_case(TargetEol::PassThrough; "without EOL conversion")]
    fn test_eol_writer_empty_buffer(target_eol: TargetEol) {
        let mut result = vec![];
        let mut writer = result.write_with_eol(target_eol);
        assert_eq!(
            writer
                .write(&[])
                .expect("write an empty buffer should succeed"),
            0
        );
        drop(writer);
        assert_eq!(result, Vec::<u8>::new());
    }

    // A writer that guarantees to stop writing at the interval of the given number
    // of bytes.
    struct ControlledWriter {
        limited_writers: VecDeque<Cursor<Box<[u8]>>>,
        unlimited_writer: Option<Vec<u8>>,
        data: Vec<u8>,
    }

    impl ControlledWriter {
        fn without_limit(read_segments: impl AsRef<[usize]>) -> Self {
            Self {
                unlimited_writer: Some(Default::default()),
                ..Self::with_limit(read_segments)
            }
        }

        fn with_limit(read_segments: impl AsRef<[usize]>) -> Self {
            Self {
                limited_writers: read_segments
                    .as_ref()
                    .iter()
                    .map(|size| Cursor::new(vec![0; *size].into_boxed_slice()))
                    .collect::<VecDeque<_>>(),
                unlimited_writer: None,
                data: Default::default(),
            }
        }

        fn data(&self) -> Vec<u8> {
            let mut res = self.data.clone();
            if let Some(limited_writer) = self.limited_writers.front() {
                res.extend_from_slice(
                    &limited_writer.get_ref()[..(limited_writer.position() as usize)],
                );
            }
            if let Some(unlimited_writer) = &self.unlimited_writer {
                res.extend_from_slice(unlimited_writer);
            }
            res
        }
    }

    impl Write for ControlledWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            while self
                .limited_writers
                .front()
                .is_some_and(|writer| (writer.position() as usize) == writer.get_ref().len())
            {
                let written_bytes = self.limited_writers.pop_front().unwrap().into_inner();
                self.data.extend_from_slice(&written_bytes);
            }
            if let Some(limited_writer) = self.limited_writers.front_mut() {
                limited_writer.write(buf)
            } else if let Some(unlimited_writer) = &mut self.unlimited_writer {
                unlimited_writer.write(buf)
            } else {
                Ok(0)
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_eol_crlf_writer_cant_write_crlf_in_one_write() {
        let mut dest = ControlledWriter::without_limit([1, 1, 1]);
        let mut writer = dest.write_with_eol(TargetEol::Crlf);
        writer.write_all(b"\n1").expect("write all should succeed");
        drop(writer);
        assert_eq!(dest.data(), b"\r\n1");

        let mut dest = ControlledWriter::without_limit([1, 2]);
        let mut writer = dest.write_with_eol(TargetEol::Crlf);
        assert_eq!(writer.write(b"\r").expect("write should succeed"), 1);
        writer.write_all(b"a\n").expect("write all should succeed");
        assert_eq!(writer.write(b"\n").expect("write should succeed"), 1);
        drop(writer);
        assert_eq!(dest.data(), b"\ra\r\n\r\n");
    }

    #[test_case(TargetEol::Crlf => b"1\r\n2\r\n3\r\n".to_vec(); "targeting CRLF")]
    #[test_case(TargetEol::Lf => b"1\n2\n3\n".to_vec(); "targeting LF")]
    fn test_eol_writer_lf_texts(target_eol: TargetEol) -> Vec<u8> {
        let data = b"1\n2\n3\n";
        let mut dest = vec![];
        let mut writer = dest.write_with_eol(target_eol);
        writer.write_all(data).expect("write all should succeed");
        drop(writer);
        dest
    }

    #[test_case(TargetEol::Crlf => b"1\r\n2\r\n3\r\n".to_vec(); "targeting CRLF")]
    #[test_case(TargetEol::Lf => b"1\n2\n3\n".to_vec(); "targeting LF")]
    fn test_eol_writer_crlf_texts(target_eol: TargetEol) -> Vec<u8> {
        let data = b"1\r\n2\r\n3\r\n";
        let mut dest = vec![];
        let mut writer = dest.write_with_eol(target_eol);
        writer.write_all(data).expect("write all should succeed");
        drop(writer);
        dest
    }

    #[test_case(TargetEol::Crlf => b"\r\n".to_vec(); "targeting CRLF")]
    #[test_case(TargetEol::Lf => b"\n".to_vec(); "targeting LF")]
    fn test_eol_writer_write_single_lf(target_eol: TargetEol) -> Vec<u8> {
        let data = b"\n";
        let mut dest = vec![];
        let mut writer = dest.write_with_eol(target_eol);
        writer.write_all(data).expect("write all should succeed");
        drop(writer);
        dest
    }

    #[test_case(TargetEol::Crlf; "targeting CRLF")]
    #[test_case(TargetEol::Lf; "targeting LF")]
    fn test_eol_writer_single_cr_shouldnt_return_0(target_eol: TargetEol) {
        let data = b"\r";
        let mut res = [0u8; 1];
        let mut dest = res.as_mut_slice();
        let mut writer = dest.write_with_eol(target_eol);
        assert_eq!(writer.write(data).expect("write all should succeed"), 1);
        drop(writer);
        assert_eq!(&res, data);
    }

    #[test_case(TargetEol::Crlf, vec![
        b"\r\n1".to_vec(),
        b"\r\n123".to_vec(),
        b"\r\n123\r\n".to_vec(),
    ]; "targeting CRLF")]
    #[test_case(TargetEol::Lf, vec![
        b"\n1".to_vec(),
        b"\n123".to_vec(),
        b"\n123\n".to_vec(),
    ]; "targeting LF")]
    fn test_eol_writer_write_crlf_separately(target_eol: TargetEol, expected_values: Vec<Vec<u8>>) {
        let mut expected_values = VecDeque::from(expected_values);

        let mut dest = vec![];
        let mut writer = dest.write_with_eol(target_eol);
        assert_eq!(writer.write(b"\r").expect("write should succeed"), 1);
        // LF writer shouldn't write the pending CR, because flush doesn't mean that the
        // caller completes writing.
        writer.flush().expect("flush should succeed");
        assert_eq!(writer.write(b"\n").expect("write should succeed"), 1);
        assert_eq!(writer.write(b"1").expect("write should succeed"), 1);
        drop(writer);
        assert_eq!(dest, expected_values.pop_front().unwrap());

        let mut dest = vec![];
        let mut writer = dest.write_with_eol(target_eol);
        assert_eq!(writer.write(b"\r").expect("write should succeed"), 1);
        writer
            .write_all(b"\n123")
            .expect("write all should succeed");
        drop(writer);
        assert_eq!(dest, expected_values.pop_front().unwrap());

        let mut dest = vec![];
        let mut writer = dest.write_with_eol(target_eol);
        assert_eq!(writer.write(b"\r").expect("write should succeed"), 1);
        writer
            .write_all(b"\n123\n")
            .expect("write all should succeed");
        drop(writer);
        assert_eq!(dest, expected_values.pop_front().unwrap());
    }

    #[test_case(TargetEol::Crlf; "targeting CRLF")]
    #[test_case(TargetEol::Lf; "targeting LF")]
    fn test_eol_writer_texts_without_eol(target_eol: TargetEol) {
        let data = b"123abc";
        let mut dest = vec![];
        let mut writer = dest.write_with_eol(target_eol);
        writer.write_all(data).expect("write all should succeed");
        drop(writer);
        assert_eq!(dest, data);
    }

    #[test_case(TargetEol::Crlf; "targeting CRLF")]
    #[test_case(TargetEol::Lf; "targeting LF")]
    fn test_eol_writer_should_not_touch_single_cr(target_eol: TargetEol) {
        let data = b"123\rabc";
        let mut dest = vec![];
        let mut writer = dest.write_with_eol(target_eol);
        writer.write_all(data).expect("write all should succeed");
        drop(writer);
        assert_eq!(dest, data);
    }

    #[test_case(TargetEol::Crlf; "targeting CRLF")]
    #[test_case(TargetEol::Lf; "targeting LF")]
    fn test_eol_writer_inner_writer_cant_complete_the_line_content_write(target_eol: TargetEol) {
        let mut dest = ControlledWriter::with_limit([1]);
        let mut writer = dest.write_with_eol(target_eol);
        assert_eq!(writer.write(b"1234\n").expect("write should succeed"), 1);
        drop(writer);
        assert_eq!(dest.data(), b"1");
    }

    #[test]
    fn test_eol_crlf_writer_should_write_pending_lf_on_flush() {
        let mut dest = ControlledWriter::without_limit([1]);
        let mut writer = dest.write_with_eol(TargetEol::Crlf);
        assert_eq!(writer.write(b"\n").expect("write should succeed"), 1);
        writer.flush().expect("flush should succeed");
        // Further flush shouldn't write anything.
        writer.flush().expect("flush should succeed");
        writer.flush().expect("flush should succeed");
        drop(writer);
        assert_eq!(dest.data(), b"\r\n");
    }

    #[test]
    fn test_eol_crlf_writer_flush_cant_write_pending_lf() {
        let mut res = [0u8; 1];
        let mut dest = res.as_mut_slice();
        let mut writer = dest.write_with_eol(TargetEol::Crlf);
        assert_eq!(writer.write(b"\n").expect("write should succeed"), 1);
        let e = writer.flush().expect_err("flush should fail");
        drop(writer);
        assert_eq!(e.kind(), std::io::ErrorKind::WriteZero);
        assert_eq!(&res, b"\r");
    }

    #[test]
    fn test_eol_crlf_writer_incomplete_write_ends_with_cr_should_remember() {
        let mut dest = ControlledWriter::without_limit([1, 1]);
        let mut writer = dest.write_with_eol(TargetEol::Crlf);
        assert_eq!(writer.write(b"\r").expect("write should succeed"), 1);
        assert_eq!(writer.write(b"\ra\n").expect("write should succeed"), 1);
        assert_eq!(writer.write(b"\n").expect("write should succeed"), 1);
        drop(writer);
        assert_eq!(dest.data(), b"\r\r\n");
    }

    #[test]
    fn test_eol_crlf_writer_cant_write_line_to_end() {
        let mut dest = ControlledWriter::without_limit([1, 1]);
        let mut writer = dest.write_with_eol(TargetEol::Crlf);
        assert_eq!(writer.write(b"\r").expect("write should succeed"), 1);
        assert_eq!(writer.write(b"ab\n").expect("write should succeed"), 1);
        assert_eq!(writer.write(b"\n").expect("write should succeed"), 1);
        drop(writer);
        assert_eq!(dest.data(), b"\ra\r\n");
    }

    #[test]
    fn test_eol_crlf_writer_write_line_to_end() {
        let mut dest = ControlledWriter::without_limit([1, 3]);
        let mut writer = dest.write_with_eol(TargetEol::Crlf);
        assert_eq!(writer.write(b"\r").expect("write should succeed"), 1);
        writer.write_all(b"a\n").expect("write all should succeed");
        assert_eq!(writer.write(b"\n").expect("write should succeed"), 1);
        drop(writer);
        assert_eq!(dest.data(), b"\ra\r\n\r\n");
    }

    #[test]
    fn test_eol_crlf_writer_pending_crlf_cant_be_written() {
        let mut dest = ControlledWriter::with_limit([1]);
        let mut writer = dest.write_with_eol(TargetEol::Crlf);
        assert_eq!(writer.write(b"\n").expect("write should succeed"), 1);
        assert_eq!(writer.write(b"1").expect("write should succeed"), 0);
        assert_eq!(writer.write(b"1").expect("write should succeed"), 0);
        drop(writer);
        assert_eq!(dest.data(), b"\r");

        let mut dest = ControlledWriter::with_limit([1]);
        let mut writer = dest.write_with_eol(TargetEol::Crlf);
        assert_eq!(writer.write(b"\n").expect("write should succeed"), 1);
        drop(writer);
        assert_eq!(dest.data(), b"\r");
    }

    #[test]
    fn test_eol_lf_writer_write_single_cr_multiple_times() {
        let mut dest = vec![];
        let mut writer = dest.write_with_eol(TargetEol::Lf);
        assert_eq!(writer.write(b"\r").expect("write should succeed"), 1);
        assert_eq!(writer.write(b"\r").expect("write should succeed"), 1);
        assert_eq!(writer.write(b"\r").expect("write should succeed"), 1);
        drop(writer);
        assert_eq!(dest, b"\r\r\r");
    }

    #[test]
    fn test_eol_lf_writer_incomplete_write_ends_with_cr() {
        let mut dest = ControlledWriter::without_limit([1, 1]);
        let mut writer = dest.write_with_eol(TargetEol::Lf);
        writer
            .write_all(b"\ra\n")
            .expect("write all should succeed");
        drop(writer);
        assert_eq!(dest.data(), b"\ra\n");

        let mut dest = ControlledWriter::without_limit([1, 1]);
        let mut writer = dest.write_with_eol(TargetEol::Lf);
        assert_eq!(writer.write(b"\r").expect("write should succeed"), 1);
        // LF writer shouldn't write the pending CR, because flush doesn't mean that the
        // caller completes writing.
        writer.flush().expect("flush should succeed");
        writer.write_all(b"\r1").expect("write all should succeed");
        drop(writer);
        assert_eq!(dest.data(), b"\r\r1");

        let mut dest = ControlledWriter::without_limit([1, 1]);
        let mut writer = dest.write_with_eol(TargetEol::Lf);
        assert_eq!(
            writer.write(b"\r\r\r").expect("write all should succeed"),
            2
        );
        assert_eq!(writer.write(b"\n").expect("wrie should succeed"), 1);
        drop(writer);
        assert_eq!(dest.data(), b"\r\n");
    }

    #[test]
    fn test_eol_lf_writer_hit_eof_when_writing_pending_cr() {
        let mut result = [0; 0];
        let mut dest = result.as_mut_slice();
        let mut writer = dest.write_with_eol(TargetEol::Lf);
        assert_eq!(writer.write(b"\r").expect("write should succeed"), 1);
        assert_eq!(writer.write(b"a").expect("write should succeed"), 0);
    }

    #[test]
    fn test_eol_lf_writer_hit_eof_when_writing_cr_that_isnt_part_of_crlf() {
        let mut result = [0; 1];
        let mut dest = result.as_mut_slice();
        let mut writer = dest.write_with_eol(TargetEol::Lf);
        assert_eq!(writer.write(b"\ra").expect("write should succeed"), 1);
        drop(writer);
        assert_eq!(&result, b"\r");
    }
}
