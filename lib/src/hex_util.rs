// Copyright 2023 The Jujutsu Authors
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

//! Hex string helpers.

const REVERSE_HEX_CHARS: &[u8; 16] = b"zyxwvutsrqponmlk";

fn reverse_hex_value(b: u8) -> Option<u8> {
    match b {
        b'k'..=b'z' => Some(b'z' - b),
        b'K'..=b'Z' => Some(b'Z' - b),
        _ => None,
    }
}

/// Decodes `reverse_hex` as hex string using `z-k` "digits".
pub fn decode_reverse_hex(reverse_hex: impl AsRef<[u8]>) -> Option<Vec<u8>> {
    decode_hex_inner(reverse_hex.as_ref(), reverse_hex_value)
}

/// Decodes `reverse_hex` as hex string prefix using `z-k` "digits". The output
/// may have odd-length byte. Returns `(bytes, has_odd_byte)`.
pub fn decode_reverse_hex_prefix(reverse_hex: impl AsRef<[u8]>) -> Option<(Vec<u8>, bool)> {
    decode_hex_prefix_inner(reverse_hex.as_ref(), reverse_hex_value)
}

fn decode_hex_inner(reverse_hex: &[u8], hex_value: impl Fn(u8) -> Option<u8>) -> Option<Vec<u8>> {
    if reverse_hex.len() % 2 != 0 {
        return None;
    }
    let (decoded, _) = decode_hex_prefix_inner(reverse_hex, hex_value)?;
    Some(decoded)
}

fn decode_hex_prefix_inner(
    reverse_hex: &[u8],
    hex_value: impl Fn(u8) -> Option<u8>,
) -> Option<(Vec<u8>, bool)> {
    let mut decoded = Vec::with_capacity(usize::div_ceil(reverse_hex.len(), 2));
    let mut chunks = reverse_hex.chunks_exact(2);
    for chunk in &mut chunks {
        let [hi, lo] = chunk.try_into().unwrap();
        decoded.push(hex_value(hi)? << 4 | hex_value(lo)?);
    }
    if let &[hi] = chunks.remainder() {
        decoded.push(hex_value(hi)? << 4);
        Some((decoded, true))
    } else {
        Some((decoded, false))
    }
}

/// Encodes `data` as hex string using `z-k` "digits".
pub fn encode_reverse_hex(data: &[u8]) -> String {
    let chars = REVERSE_HEX_CHARS;
    let encoded = data
        .iter()
        .flat_map(|b| [chars[usize::from(b >> 4)], chars[usize::from(b & 0xf)]])
        .collect();
    String::from_utf8(encoded).unwrap()
}

/// Calculates common prefix length of two byte sequences. The length
/// to be returned is a number of hexadecimal digits.
pub fn common_hex_len(bytes_a: &[u8], bytes_b: &[u8]) -> usize {
    std::iter::zip(bytes_a, bytes_b)
        .enumerate()
        .find_map(|(i, (a, b))| match a ^ b {
            0 => None,
            d if d & 0xf0 == 0 => Some(i * 2 + 1),
            _ => Some(i * 2),
        })
        .unwrap_or_else(|| bytes_a.len().min(bytes_b.len()) * 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reverse_hex() {
        // Empty string
        assert_eq!(decode_reverse_hex(""), Some(vec![]));
        assert_eq!(decode_reverse_hex_prefix(""), Some((vec![], false)));
        assert_eq!(encode_reverse_hex(b""), "".to_string());

        // Single digit
        assert_eq!(decode_reverse_hex("z"), None);
        assert_eq!(decode_reverse_hex_prefix("k"), Some((vec![0xf0], true)));

        // All digits
        assert_eq!(
            decode_reverse_hex("zyxwvutsRQPONMLK"),
            Some(b"\x01\x23\x45\x67\x89\xab\xcd\xef".to_vec())
        );
        assert_eq!(
            decode_reverse_hex_prefix("ZYXWVUTSrqponmlk"),
            Some((b"\x01\x23\x45\x67\x89\xab\xcd\xef".to_vec(), false))
        );
        assert_eq!(
            encode_reverse_hex(b"\x01\x23\x45\x67\x89\xab\xcd\xef"),
            "zyxwvutsrqponmlk".to_string()
        );

        // Invalid digit
        assert_eq!(decode_reverse_hex("jj"), None);
        assert_eq!(decode_reverse_hex_prefix("jj"), None);
    }
}
