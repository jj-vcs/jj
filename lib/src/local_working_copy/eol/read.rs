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

use smallvec::smallvec;
use smallvec::SmallVec;

use super::TargetEol;

pub(crate) trait ReadExt {
    /// Read texts from the original reader, [`self`], and convert the EOL of
    /// the text to the `target_eol`.
    ///
    /// The texts read can have any or mixed EOLs, LF or CRLF.
    fn read_with_eol(&mut self, target_eol: TargetEol) -> impl io::Read;
}

impl<T: io::Read> ReadExt for T {
    fn read_with_eol(&mut self, target_eol: TargetEol) -> impl io::Read {
        match target_eol {
            TargetEol::PassThrough => EolReader::PassThrough(self),
            TargetEol::Crlf => unimplemented!(
                "EOL reader is used for snapshotting the local working copy to the underlying \
                 store, and the target EOL is decided by the EOL used by the store backend. The \
                 only supported git backend always uses LF when EOL conversion is needed."
            ),
            TargetEol::Lf => EolReader::Lf(LfReader {
                reader: self,
                pending_byte: Default::default(),
            }),
        }
    }
}

enum EolReader<'a, R: io::Read + ?Sized> {
    PassThrough(&'a mut R),
    Lf(LfReader<'a, R>),
}

impl<'a, R: io::Read + ?Sized> io::Read for EolReader<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::PassThrough(reader) => reader.read(buf),
            Self::Lf(reader) => reader.read(buf),
        }
    }
}

struct LfReader<'a, R: io::Read + ?Sized> {
    reader: &'a mut R,
    /// The possible last byte from the last read that we decide to not return
    /// to the caller.
    ///
    /// * If the last read ends with a CR byte, and the next read starts with a
    ///   LF byte, which forms a CRLF from the original reader, the caller
    ///   should receive just one LF byte. We have to store the CR byte and
    ///   shouldn't return that CR byte to the user.
    /// * If the we currently have a pending CR byte, the caller provides with a
    ///   one byte long buffer, and the next read results in a non LF byte, byte
    ///   `x`, the caller should receive the CR byte for this read, and byte `x`
    ///   for the next read. We have to store byte `x` for the next read.
    ///
    /// In conclusion, we may store arbitrary pending byte, so a `bool` is not
    /// enough.
    pending_byte: Option<u8>,
}

impl<'a, R: io::Read + ?Sized> io::Read for LfReader<'a, R> {
    fn read(&mut self, mut buf: &mut [u8]) -> io::Result<usize> {
        match buf.len() {
            0 => Ok(0),
            1 => {
                // The one byte long buffer case is special:
                //
                // * If there is a pending CR byte, and the current read results in a non-CR
                //   byte, we can only return the CR byte and store this byte instead of
                //   returning the byte to the caller.
                // * If other cases(buffer length >= 2 bytes) end up needing to read exactly one
                //   byte, it is handled here. This happens if we don't have a pending byte and
                //   the read operation only returns us one single CR byte. We can't just return
                //   this CR byte to the caller, because the next read may form a CRLF. We can't
                //   store the CR byte and return Ok(0), because it's certain that the caller
                //   certainly hasn't reached the EOF - there is at least a pending CR.
                if let Some(pending_byte) = self.pending_byte.take() {
                    if pending_byte != b'\r' {
                        buf[0] = pending_byte;
                        return Ok(1);
                    }
                    // We have a pending CR, need to look forward another byte in case a CRLF can be
                    // formed, in which case, we need to return a LF instead of a CR.
                    let n = self.reader.read(buf)?;
                    if n == 0 {
                        // We reach EOF, so we don't bother if the pending CR can form a CRLF with
                        // the next byte.
                        buf[0] = pending_byte;
                        return Ok(1);
                    }
                    if buf[0] == b'\n' {
                        // A CRLF is formed. Return LF to the caller.
                        buf[0] = b'\n';
                        return Ok(1);
                    }
                    // The pending CR can't form CRLF, so the pending CR should be returned to the
                    // caller. The buf can store only one byte, which is filed by the pending CR
                    // byte, the extra byte read is stored.
                    let next_byte = buf[0];
                    buf[0] = pending_byte;
                    debug_assert!(self.pending_byte.is_none());
                    self.pending_byte = Some(next_byte);
                    return Ok(1);
                }
                // Handle the case where we don't have a pending byte.
                let n = self.reader.read(buf)?;
                if n == 0 {
                    // We don't have a pending byte and we reach the EOF. There is nothing extra for
                    // the caller to read for certain.
                    return Ok(0);
                }
                if buf[0] == b'\r' {
                    // This CR may form a CRLF, so we can't just return the CR byte to the caller.
                    // However, we can't just store the byte and return Ok(0), because the caller
                    // hasn't reached EOF yet.
                    debug_assert!(self.pending_byte.is_none());
                    self.pending_byte = Some(b'\r');
                    // Jump to the case where we handle pending byte with a single byte buffer.
                    let ret = self.read(buf)?;
                    debug_assert_ne!(
                        ret, 0,
                        "We haven't reached EOF, so we shouldn't return Ok(0)."
                    );
                    return Ok(ret);
                }
                // We read a non-CR byte, just return it to the caller.
                Ok(1)
            }
            _ => {
                // Handle the case where buffer length is at least 2 bytes.
                let mut len = 0;
                if let Some(pending_byte) = self.pending_byte.take() {
                    buf[0] = pending_byte;
                    len += 1;
                    // We can't return Ok(1) and the pending byte to the caller,
                    // if the pending byte is CR and forms CRLF with the next
                    // read.
                }
                // We put the pending byte with the extra bytes read in a single slice.
                let n = self.reader.read(&mut buf[len..])?;
                len += n;
                buf = &mut buf[..len];
                // We use 2 pointer to convert the EOL in place. The write_idx points to the
                // next byte of the bytes that completes the EOL conversion. The read_idx points
                // to the current byte we are processing.
                let mut write_idx = 0;
                // When the read_idx meets a CR, we can only decide what to write when we check
                // the next byte. This flag indicates whether the last byte read_idx points to
                // is CR.
                let mut has_pending_cr = false;
                for read_idx in 0..len {
                    let current_byte = buf[read_idx];
                    let to_write: SmallVec<[u8; 2]> = match (has_pending_cr, current_byte) {
                        // The pending CR forms a CRLF. Write LF back.
                        (true, b'\n') => smallvec![b'\n'],
                        // The pending CR doesn't form a CRLF. Write CR back. The current byte is
                        // CR. Do not write back.
                        (true, b'\r') => smallvec![b'\r'],
                        // The pending CR doesn't form a CRLF. Write CR back. The current byte is
                        // not CR. Write back the current byte.
                        (true, _) => smallvec![b'\r', current_byte],
                        // No pending CR. The current byte is CR. Write back nothing.
                        (false, b'\r') => smallvec![],
                        // No pending CR. The current byte is not CR. Write back the current byte.
                        (false, _) => smallvec![current_byte],
                    };

                    // Write back the bytes and update write_idx.
                    let next_write_idx = write_idx + to_write.len();
                    buf[write_idx..next_write_idx].copy_from_slice(&to_write);
                    write_idx = next_write_idx;

                    // Update the pending CR flag.
                    has_pending_cr = current_byte == b'\r';
                }
                if has_pending_cr {
                    // The current read ends with CR. We can't directly return it to the caller in
                    // case the next read forms a CRLF. Store the CR byte.
                    debug_assert!(self.pending_byte.is_none());
                    self.pending_byte = Some(b'\r');
                }
                if n == 0 {
                    // We reach an EOF. When io::Read::read returns Ok(0), it either means the
                    // passed in buffer is empty or an EOF is hit. However, the passed in buffer is
                    // not empty, because when we call self.reader.read(buf[len..]), len is at most
                    // 1, buf is at least 2 bytes long. buf[len..], therefore, is at least 1 byte
                    // long. Hence the Ok(0) returned by io::Read indicates the EOF.
                    match self.pending_byte.take() {
                        Some(pending_byte) => {
                            // We reach EOF, so we don't bother if the pending byte can form a CRLF
                            // with the next byte.
                            debug_assert!(
                                write_idx < buf.len(),
                                "We can't be out of range, because the total number of bytes read \
                                 including the pending byte, doesn't exceed the buffer size."
                            );
                            buf[write_idx] = pending_byte;
                            return Ok(write_idx + 1);
                        }
                        // We reach EOF, and we don't have pending bytes. Even if write_idx is 0, we
                        // can report EOF to the user by returning Ok(0) now.
                        None => return Ok(write_idx),
                    }
                }
                // Handle the case where we don't reach the EOF.
                if write_idx == 0 {
                    // We haven't reached EOF, but we haven't written anything to buf. We can't
                    // return Ok(0) here.
                    debug_assert!(
                        self.pending_byte.is_some(),
                        "If we have read some data(n != 0), but we haven't written that data to \
                         the buf, it must be in the pending_byte."
                    );
                    // Read exactly one byte by jumping to the case where we handle pending byte
                    // with a single byte buffer. Given that we have pending bytes, if we call
                    // self.read with a single byte buffer, we are guaranteed to not return Ok(0).
                    let ret = self.read(&mut buf[..1])?;
                    debug_assert_ne!(
                        ret, 0,
                        "We haven't reached EOF, so we shouldn't return Ok(0)."
                    );
                    return Ok(ret);
                }
                Ok(write_idx)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use test_case::test_case;

    use super::*;

    #[test_case(TargetEol::Lf; "targeting LF")]
    #[test_case(TargetEol::PassThrough; "without EOL conversion")]
    fn test_eol_reader_empty_buf(target_eol: TargetEol) {
        let source = b"test text";
        let mut source = source.as_slice();
        let mut reader = source.read_with_eol(target_eol);
        assert_eq!(
            reader
                .read(&mut [])
                .expect("read into an empty buffer should succeed"),
            0
        );
    }

    /// A reader that allows controlling how many bytes are read at once.
    /// Different elements of `data` are guaranteed to not be read in a single
    /// read operation.
    struct ControlledReader {
        data: Vec<Vec<u8>>,
    }

    impl Read for ControlledReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.data.is_empty() {
                return Ok(0);
            }
            self.data.retain(|data| !data.is_empty());
            let data = match self.data.first_mut() {
                Some(data) => {
                    assert!(!data.is_empty());
                    data
                }
                None => return Ok(0),
            };
            let copy_len = data.len().min(buf.len());
            buf[..copy_len].copy_from_slice(&data[..copy_len]);
            data.drain(..copy_len);
            Ok(copy_len)
        }
    }

    #[test]
    fn test_eol_lf_reader_crlf_separated_in_2_read() {
        let mut source = ControlledReader {
            data: vec![b"before cr\r".to_vec(), b"\nafter lf".to_vec()],
        };
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut actual_data = vec![];
        reader
            .read_to_end(&mut actual_data)
            .expect("read to end should succeed");
        assert_eq!(&actual_data, b"before cr\nafter lf");

        let mut source = ControlledReader {
            data: vec![b"1\r".to_vec(), b"\nabcd".to_vec()],
        };
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut actual_data = vec![0; 2];
        reader
            .read_exact(&mut actual_data)
            .expect("read exact should succeed");
        assert_eq!(&actual_data, b"1\n");
        reader
            .read_to_end(&mut actual_data)
            .expect("read to end should succeed");
        assert_eq!(&actual_data, b"1\nabcd");
    }

    #[test]
    fn test_eol_lf_reader_data_separated_in_2_read_with_cr_as_the_last_byte_in_first_read() {
        let mut source = ControlledReader {
            data: vec![b"1\r".to_vec(), b"abcd".to_vec()],
        };
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut actual_data = vec![0; 2];
        reader
            .read_exact(&mut actual_data)
            .expect("read exact should succeed");
        assert_eq!(&actual_data, b"1\r");
        reader
            .read_to_end(&mut actual_data)
            .expect("read to end should succeed");
        assert_eq!(&actual_data, b"1\rabcd");
    }

    #[test]
    fn test_eol_lf_reader_inner_read_ends_with_pending_cr() {
        let data = b"12345\r";
        let mut source = data.as_slice();
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut buf = vec![];
        reader
            .read_to_end(&mut buf)
            .expect("read to end should succeed");
        assert_eq!(&buf, data);
    }

    #[test]
    fn test_eol_reader_inner_read_ends_without_pending_cr() {
        let data = b"abcdefg";
        let mut source = data.as_slice();
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut buf = vec![0; data.len()];
        reader
            .read_exact(&mut buf)
            .expect("read exact should succeed");
        assert_eq!(&buf, data);
        assert_eq!(reader.read(&mut buf).expect("read should succeed"), 0);
        assert_eq!(reader.read(&mut buf).expect("read should succeed"), 0);
    }

    #[test]
    fn test_eol_reader_single_cr_read_should_not_return_0() {
        let mut source = ControlledReader {
            data: vec![b"\r".to_vec(), b"after CR".to_vec()],
        };
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut buf = vec![];
        reader
            .read_to_end(&mut buf)
            .expect("read to end should succeed");
        assert_eq!(&buf, b"\rafter CR");

        let mut source = ControlledReader {
            data: vec![b"\r".to_vec(), b"\nafter LF".to_vec()],
        };
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut buf = vec![];
        reader
            .read_to_end(&mut buf)
            .expect("read to end should succeed");
        assert_eq!(&buf, b"\nafter LF");
    }

    #[test]
    fn test_eol_crlf_reader_read_one_last_non_crlf_byte() {
        let mut source = b"1".as_slice();
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut buf = [0u8];
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            1
        );
        assert_eq!(&buf, b"1");
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );
    }

    #[test]
    fn test_eol_crlf_reader_read_one_last_crlf_byte() {
        let mut source = b"\r\n".as_slice();
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut buf = [0u8];
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            1
        );
        assert_eq!(&buf, b"\n");
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );
    }

    #[test]
    fn test_eol_reader_read_one_last_non_crlf_byte_with_pending_byte() {
        let mut source = b"\r1".as_slice();
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut buf = [0u8];
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            1
        );
        assert_eq!(buf[0], b'\r');
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            1
        );
        assert_eq!(buf[0], b'1');
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );

        let mut source = b"\r1".as_slice();
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut buf = [0u8];
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            1
        );
        assert_eq!(buf[0], b'\r');
        // Try the second read with a larger buffer.
        let mut buf = [0u8; 20];
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            1
        );
        assert_eq!(buf[0], b'1');
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );
    }

    #[test]
    fn test_eol_reader_read_one_last_cr_byte_with_pending_cr_byte() {
        let mut source = b"\r\r".as_slice();
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut buf = [0u8];
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            1
        );
        assert_eq!(buf[0], b'\r');
        // Clear the buffer to make sure the the read function actually writes to the
        // buffer.
        buf[0] = 0;
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            1
        );
        assert_eq!(buf[0], b'\r');
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );

        let mut source = b"\r\r".as_slice();
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut buf = [0u8];
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            1
        );
        assert_eq!(buf[0], b'\r');
        // Try the second read with a larger buffer.
        let mut buf = [0u8; 20];
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            1
        );
        assert_eq!(buf[0], b'\r');
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );
    }

    #[test]
    fn test_eol_reader_read_one_cr_byte() {
        let mut source = b"\r".as_slice();
        let mut reader = source.read_with_eol(TargetEol::Lf);
        let mut buf = [0u8];
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            1
        );
        assert_eq!(&buf, b"\r");
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );
        assert_eq!(
            reader
                .read(buf.as_mut_slice())
                .expect("read should succeed"),
            0
        );
    }

    #[test]
    fn test_eol_reader_crlf_texts() {
        let mut source = b"\r\n\r\n1\r\n\r\na\r\n!\r\n".as_slice();
        let mut buf = Vec::with_capacity(source.len() * 2);
        let mut reader = source.read_with_eol(TargetEol::Lf);
        reader
            .read_to_end(&mut buf)
            .expect("read to end should succeed");
        assert_eq!(buf, b"\n\n1\n\na\n!\n");
    }

    #[test]
    fn test_eol_reader_cr_not_followed_by_lf() {
        let data = b"\r\r1\r\ra\r!\r";
        let mut source = data.as_slice();
        let mut buf = Vec::with_capacity(data.len() * 2);
        let mut reader = source.read_with_eol(TargetEol::Lf);
        reader
            .read_to_end(&mut buf)
            .expect("read to end should succeed");
        assert_eq!(buf, data);
    }
}
