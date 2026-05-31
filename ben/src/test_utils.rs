//! Test helpers shared across unit and integration tests.
//!
//! This module is always-compiled (not `#[cfg(test)]`) so integration tests in `ben/tests/` — which
//! are separate crates — can reuse the same helpers as unit tests inside `ben/src/.../tests.rs`. It
//! is `#[doc(hidden)]` and is not part of the stable public API.

use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use std::ops::Range;

use crate::codec::encode::encode_jsonl_to_ben;
use crate::io::bundle::format::{AssignmentFormat, DIRECTORY_ENTRY_HEADER_SIZE};
use crate::io::bundle::BendlWriter;
use crate::BenVariant;

/// Return a unique temp path of the form `binary-ensemble-{name}-{nonce}` in the system temp
/// directory. The nonce is the current monotonic-ish time in nanoseconds, sufficient to avoid
/// collisions between parallel test runs.
pub fn unique_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("binary-ensemble-{name}-{nonce}"))
}

/// Build a JSONL byte buffer from a sequence of assignment vectors, numbering samples from 1.
pub fn jsonl_from_assignments(assignments: &[Vec<u16>]) -> Vec<u8> {
    let mut buf = Vec::new();
    for (i, a) in assignments.iter().enumerate() {
        writeln!(&mut buf, "{}", json!({"assignment": a, "sample": i + 1})).unwrap();
    }
    buf
}

/// Expand an RLE sequence `(value, length)` into a flat assignment vector, truncating at `cap`.
pub fn expand_rle(rle: &[(u16, u16)], cap: usize) -> Vec<u16> {
    let mut v = Vec::with_capacity(cap);
    for &(val, len) in rle {
        let take = (len as usize).min(cap.saturating_sub(v.len()));
        v.extend(std::iter::repeat_n(val, take));
        if v.len() >= cap {
            break;
        }
    }
    v
}

/// Encode the given JSONL bytes as a BEN byte vector, including the 17-byte banner. Panics on
/// encoder error; intended only for fixture construction.
pub fn sample_ben_bytes(jsonl: &[u8], variant: BenVariant) -> Vec<u8> {
    let mut out = Vec::new();
    encode_jsonl_to_ben(jsonl, &mut out, variant).unwrap();
    out
}

/// Build a minimal finalized `.bendl` byte vector containing the given pre-encoded assignment
/// stream bytes. Panics on writer error; intended only for fixture construction.
pub fn sample_bendl_bytes(stream: &[u8], format: AssignmentFormat) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let writer = BendlWriter::new(Cursor::new(&mut buf), format).unwrap();
        let mut session = writer.into_stream_session().unwrap();
        session.write_all(stream).unwrap();
        let writer = session.finish_into_writer(1);
        writer.finish().unwrap();
    }
    buf
}

/// A field of the fixed `.bendl` header, identified by name rather than by raw byte offset.
///
/// The associated [`HeaderField::range`] is the field's byte range inside the 64-byte header, the
/// single source of truth that adversarial fixtures patch against instead of hard-coding offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderField {
    /// `alignment_padding` (`u16`).
    AlignmentPadding,
    /// `flags` (`u32`).
    Flags,
    /// `stream_checksum` (`u32`).
    StreamChecksum,
    /// `directory_offset` (`u64`).
    DirectoryOffset,
    /// `directory_len` (`u64`).
    DirectoryLen,
    /// `stream_offset` (`u64`).
    StreamOffset,
    /// `stream_len` (`u64`).
    StreamLen,
    /// `sample_count` (`i64`).
    SampleCount,
}

impl HeaderField {
    /// Byte range this field occupies within the fixed 64-byte header.
    pub fn range(self) -> Range<usize> {
        match self {
            HeaderField::AlignmentPadding => 14..16,
            HeaderField::Flags => 16..20,
            HeaderField::StreamChecksum => 20..24,
            HeaderField::DirectoryOffset => 24..32,
            HeaderField::DirectoryLen => 32..40,
            HeaderField::StreamOffset => 40..48,
            HeaderField::StreamLen => 48..56,
            HeaderField::SampleCount => 56..64,
        }
    }
}

/// A field of a directory entry's fixed header, identified by name rather than by raw byte offset.
///
/// The associated [`DirectoryEntryField::range`] is the field's byte range *relative to the start
/// of the entry* (the entry begins after the directory's leading `u32` entry count).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryEntryField {
    /// `asset_type` (`u16`).
    AssetType,
    /// `asset_flags` (`u16`).
    AssetFlags,
    /// `name_len` (`u16`).
    NameLen,
    /// `payload_offset` (`u64`).
    PayloadOffset,
    /// `payload_len` (`u64`).
    PayloadLen,
    /// `checksum_len` (`u32`).
    ChecksumLen,
}

impl DirectoryEntryField {
    /// Byte range this field occupies relative to the start of a directory entry.
    pub fn range(self) -> Range<usize> {
        match self {
            DirectoryEntryField::AssetType => 0..2,
            DirectoryEntryField::AssetFlags => 2..4,
            DirectoryEntryField::NameLen => 4..6,
            DirectoryEntryField::PayloadOffset => 8..16,
            DirectoryEntryField::PayloadLen => 16..24,
            DirectoryEntryField::ChecksumLen => 24..28,
        }
    }
}

/// A mutable wrapper over raw `.bendl` bytes for building adversarial fixtures by *named field*
/// instead of by magic byte offset.
///
/// The builder methods (`with_*`, `corrupt_*`) consume `self` and return it so patches can be
/// chained; the reader methods (`header_u64`, `entry_count`) inspect the current bytes so a fixture
/// can patch a field whose location depends on another (e.g. walking entries from
/// `directory_offset`). Field locations come from [`HeaderField`] / [`DirectoryEntryField`], so the
/// on-disk layout is named in exactly one place.
#[derive(Clone, Debug)]
pub struct BendlBytes {
    bytes: Vec<u8>,
}

impl BendlBytes {
    /// Wrap an existing byte vector (typically a valid bundle seed) for patching.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Borrow the current bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consume the builder and return the patched bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Read a header field as a little-endian `u64` (reading only the field's own width).
    pub fn header_u64(&self, field: HeaderField) -> u64 {
        let range = field.range();
        let mut buf = [0u8; 8];
        buf[..range.len()].copy_from_slice(&self.bytes[range]);
        u64::from_le_bytes(buf)
    }

    /// Patch a header field to the low bytes of `value` (the field's own width), returning `self`
    /// for chaining. Works for the `u16`/`u32`/`u64` header fields alike: only
    /// `field.range().len()` little-endian bytes are written, so e.g. patching
    /// `AlignmentPadding` writes two bytes.
    pub fn with_header_u64(mut self, field: HeaderField, value: u64) -> Self {
        let range = field.range();
        let width = range.len();
        self.bytes[range].copy_from_slice(&value.to_le_bytes()[..width]);
        self
    }

    /// The directory's leading `u32` entry count.
    pub fn entry_count(&self) -> u32 {
        let dir = self.header_u64(HeaderField::DirectoryOffset) as usize;
        u32::from_le_bytes(self.bytes[dir..dir + 4].try_into().unwrap())
    }

    /// Patch the directory's leading `u32` entry count, returning `self` for chaining.
    pub fn with_entry_count(mut self, count: u32) -> Self {
        let dir = self.header_u64(HeaderField::DirectoryOffset) as usize;
        self.bytes[dir..dir + 4].copy_from_slice(&count.to_le_bytes());
        self
    }

    /// Byte offset where directory entry `index` begins, walking the variable-length entries from
    /// `directory_offset` using each entry's own `name_len` / `checksum_len`.
    fn directory_entry_offset(&self, index: usize) -> usize {
        let dir = self.header_u64(HeaderField::DirectoryOffset) as usize;
        let mut cursor = dir + 4; // skip the u32 entry count
        for _ in 0..index {
            let name_len = self.entry_field_u64(cursor, DirectoryEntryField::NameLen) as usize;
            let checksum_len =
                self.entry_field_u64(cursor, DirectoryEntryField::ChecksumLen) as usize;
            cursor += DIRECTORY_ENTRY_HEADER_SIZE + name_len + checksum_len;
        }
        cursor
    }

    /// Read a directory-entry field (relative to `entry_start`) as a little-endian `u64`.
    fn entry_field_u64(&self, entry_start: usize, field: DirectoryEntryField) -> u64 {
        let range = field.range();
        let mut buf = [0u8; 8];
        buf[..range.len()]
            .copy_from_slice(&self.bytes[entry_start + range.start..entry_start + range.end]);
        u64::from_le_bytes(buf)
    }

    /// Patch a field of directory entry `index` to the low bytes of `value` (the field's own
    /// width), returning `self` for chaining.
    pub fn with_directory_entry_field(
        mut self,
        index: usize,
        field: DirectoryEntryField,
        value: u64,
    ) -> Self {
        let base = self.directory_entry_offset(index);
        let range = field.range();
        let width = range.len();
        self.bytes[base + range.start..base + range.start + width]
            .copy_from_slice(&value.to_le_bytes()[..width]);
        self
    }

    /// Flip a byte of directory entry `index`'s on-disk payload, simulating payload corruption that
    /// a stored CRC should catch. `byte_within_payload` indexes from the entry's `payload_offset`.
    pub fn corrupt_asset_payload(mut self, index: usize, byte_within_payload: usize) -> Self {
        let base = self.directory_entry_offset(index);
        let payload_offset =
            self.entry_field_u64(base, DirectoryEntryField::PayloadOffset) as usize;
        self.bytes[payload_offset + byte_within_payload] ^= 0xFF;
        self
    }

    /// Flip the first byte of directory entry `index`'s stored trailing checksum, simulating a
    /// corrupt stored CRC. The entry must carry a trailing checksum (the bytes after its name).
    pub fn corrupt_stored_asset_crc(mut self, index: usize) -> Self {
        let base = self.directory_entry_offset(index);
        let name_len = self.entry_field_u64(base, DirectoryEntryField::NameLen) as usize;
        let checksum_start = base + DIRECTORY_ENTRY_HEADER_SIZE + name_len;
        self.bytes[checksum_start] ^= 0xFF;
        self
    }
}

impl From<BendlBytes> for Vec<u8> {
    fn from(b: BendlBytes) -> Vec<u8> {
        b.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_path_includes_name_and_is_unique() {
        let a = unique_path("hello");
        let b = unique_path("hello");
        assert!(a.file_name().unwrap().to_string_lossy().contains("hello"));
        assert_ne!(a, b);
    }

    #[test]
    fn jsonl_from_assignments_emits_one_line_per_sample() {
        let out = jsonl_from_assignments(&[vec![1, 2, 3], vec![2, 1, 3]]);
        let s = std::str::from_utf8(&out).unwrap();
        assert_eq!(s.lines().count(), 2);
        assert!(s.contains("\"sample\":1"));
        assert!(s.contains("\"sample\":2"));
        assert!(s.contains("[1,2,3]"));
    }

    #[test]
    fn expand_rle_truncates_at_cap() {
        let v = expand_rle(&[(1, 5), (2, 5)], 7);
        assert_eq!(v, vec![1, 1, 1, 1, 1, 2, 2]);
    }

    #[test]
    fn expand_rle_handles_zero_cap() {
        let v = expand_rle(&[(1, 5)], 0);
        assert!(v.is_empty());
    }

    #[test]
    fn sample_ben_bytes_round_trips_via_decode() {
        use crate::codec::decode::decode_ben_to_jsonl;
        let jsonl = jsonl_from_assignments(&[vec![1, 2, 3]]);
        let ben = sample_ben_bytes(&jsonl, BenVariant::Standard);
        let mut decoded = Vec::new();
        decode_ben_to_jsonl(ben.as_slice(), &mut decoded).unwrap();
        let s = String::from_utf8(decoded).unwrap();
        assert!(s.contains("[1,2,3]"));
    }

    #[test]
    fn sample_bendl_bytes_yields_complete_bundle() {
        use crate::io::bundle::BendlReader;
        use std::io::BufReader;

        let bytes = sample_bendl_bytes(b"STANDARD BEN FILE\x00fake", AssignmentFormat::Ben);
        let reader = BendlReader::open(BufReader::new(Cursor::new(bytes))).unwrap();
        assert!(reader.is_finalized());
    }

    #[test]
    fn bendl_bytes_reads_and_patches_named_fields() {
        use crate::io::bundle::format::ASSET_TYPE_CUSTOM;
        use crate::io::bundle::writer::AddAssetOptions;
        use crate::io::bundle::BendlReader;

        let mut buf = Vec::new();
        {
            let mut writer =
                BendlWriter::new(Cursor::new(&mut buf), AssignmentFormat::Ben).unwrap();
            writer
                .add_asset(
                    ASSET_TYPE_CUSTOM,
                    "first.bin",
                    b"first payload",
                    AddAssetOptions::defaults().raw(),
                )
                .unwrap();
            writer
                .add_asset(
                    ASSET_TYPE_CUSTOM,
                    "second.bin",
                    b"second payload bytes",
                    AddAssetOptions::defaults().raw(),
                )
                .unwrap();
            let mut session = writer.into_stream_session().unwrap();
            session.write_all(b"STANDARD BEN FILE\x00fake").unwrap();
            let writer = session.finish_into_writer(1);
            writer.finish().unwrap();
        }

        let reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
        let entries = reader.assets().to_vec();
        drop(reader);
        assert_eq!(entries.len(), 2);

        let bb = BendlBytes::new(buf);
        assert_eq!(bb.entry_count(), 2);

        // Field reads agree with the parsed directory for every entry, proving the entry walk.
        for (i, entry) in entries.iter().enumerate() {
            let base = bb.directory_entry_offset(i);
            assert_eq!(
                bb.entry_field_u64(base, DirectoryEntryField::PayloadOffset),
                entry.payload_offset
            );
            assert_eq!(
                bb.entry_field_u64(base, DirectoryEntryField::PayloadLen),
                entry.payload_len
            );
            assert_eq!(
                bb.entry_field_u64(base, DirectoryEntryField::NameLen) as usize,
                entry.name.len()
            );
        }

        // with_header_u64 round-trips through header_u64.
        let relabeled = bb.clone().with_header_u64(HeaderField::DirectoryLen, 4242);
        assert_eq!(relabeled.header_u64(HeaderField::DirectoryLen), 4242);

        // corrupt_asset_payload flips exactly one byte, at the chosen entry's payload offset.
        let original = bb.as_bytes().to_vec();
        let payload_corrupted = bb.clone().corrupt_asset_payload(1, 0).into_bytes();
        let second_payload_offset = entries[1].payload_offset as usize;
        assert_eq!(
            payload_corrupted[second_payload_offset],
            original[second_payload_offset] ^ 0xFF
        );
        assert_eq!(count_differing_bytes(&original, &payload_corrupted), 1);

        // corrupt_stored_asset_crc also flips exactly one byte (the entry's stored CRC start),
        // which a default-written entry carries as four trailing bytes.
        let crc_corrupted = bb.corrupt_stored_asset_crc(1).into_bytes();
        assert_eq!(count_differing_bytes(&original, &crc_corrupted), 1);
    }

    fn count_differing_bytes(a: &[u8], b: &[u8]) -> usize {
        assert_eq!(a.len(), b.len(), "fixtures should be the same length");
        a.iter().zip(b.iter()).filter(|(x, y)| x != y).count()
    }
}
