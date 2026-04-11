//! Binary header and directory definitions for the `.bendl` container.
//!
//! This module is the pure format layer: it defines the on-disk byte
//! layout, the associated constants, and the encode/decode helpers that
//! convert between in-memory Rust structs and their on-disk representation.
//! There is no I/O orchestration here — higher layers (`reader`, `writer`)
//! combine these primitives with seekable files.
//!
//! All multi-byte integers in the `.bendl` format are little-endian.

use std::io::{self, Read, Write};

use thiserror::Error;

// ---------------------------------------------------------------------------
// Magic, version, and header layout
// ---------------------------------------------------------------------------

/// Magic bytes at offset 0 of every `.bendl` file.
pub const BENDL_MAGIC: [u8; 8] = *b"BENDL\0\0\x01";

/// Current major version produced by this implementation.
pub const BENDL_MAJOR_VERSION: u16 = 1;
/// Current minor version produced by this implementation.
pub const BENDL_MINOR_VERSION: u16 = 0;

/// Size of the fixed header in bytes.
pub const HEADER_SIZE: usize = 64;

/// `complete` flag value for incomplete (unfinalized) bundles.
pub const COMPLETE_NO: u8 = 0;
/// `complete` flag value for finalized bundles.
pub const COMPLETE_YES: u8 = 1;

// ---------------------------------------------------------------------------
// Assignment format identifiers
// ---------------------------------------------------------------------------

/// Assignment format identifier: embedded BEN stream.
pub const ASSIGNMENT_FORMAT_BEN: u8 = 1;
/// Assignment format identifier: embedded XBEN stream.
pub const ASSIGNMENT_FORMAT_XBEN: u8 = 2;

/// Container format of the embedded assignment stream.
///
/// The BEN *variant* (`Standard`, `MkvChain`, `TwoDelta`) is carried by
/// the 17-byte banner at the start of the embedded stream and is not
/// duplicated in the bundle header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentFormat {
    /// Uncompressed BEN byte stream.
    Ben,
    /// XBEN byte stream (xz-compressed BEN).
    Xben,
}

impl AssignmentFormat {
    /// Raw wire encoding of this format.
    pub fn to_u8(self) -> u8 {
        match self {
            AssignmentFormat::Ben => ASSIGNMENT_FORMAT_BEN,
            AssignmentFormat::Xben => ASSIGNMENT_FORMAT_XBEN,
        }
    }

    /// Parse a raw byte into an `AssignmentFormat`.
    pub fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            ASSIGNMENT_FORMAT_BEN => Some(AssignmentFormat::Ben),
            ASSIGNMENT_FORMAT_XBEN => Some(AssignmentFormat::Xben),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Asset types, flags, canonical names
// ---------------------------------------------------------------------------

/// Asset type id for `metadata.json`.
pub const ASSET_TYPE_METADATA: u16 = 1;
/// Asset type id for `graph.json`.
pub const ASSET_TYPE_GRAPH: u16 = 2;
/// Asset type id for `relabel_map.json`.
pub const ASSET_TYPE_RELABEL_MAP: u16 = 3;
/// Asset type id for a custom user asset (name chosen by writer).
pub const ASSET_TYPE_CUSTOM: u16 = 4;

/// Canonical name for the `metadata.json` asset.
pub const CANONICAL_NAME_METADATA: &str = "metadata.json";
/// Canonical name for the `graph.json` asset.
pub const CANONICAL_NAME_GRAPH: &str = "graph.json";
/// Canonical name for the `relabel_map.json` asset.
pub const CANONICAL_NAME_RELABEL_MAP: &str = "relabel_map.json";

/// Return the canonical name reserved for a known singleton asset type,
/// or `None` for custom or unknown types.
pub fn canonical_name_for(asset_type: u16) -> Option<&'static str> {
    match asset_type {
        ASSET_TYPE_METADATA => Some(CANONICAL_NAME_METADATA),
        ASSET_TYPE_GRAPH => Some(CANONICAL_NAME_GRAPH),
        ASSET_TYPE_RELABEL_MAP => Some(CANONICAL_NAME_RELABEL_MAP),
        _ => None,
    }
}

/// Return whether a given asset type should default to xz compression
/// when the writer is not given an explicit compression option.
pub fn default_compresses_by_type(asset_type: u16) -> bool {
    matches!(asset_type, ASSET_TYPE_GRAPH)
}

/// Asset flag bit: the decoded payload is UTF-8 JSON.
pub const ASSET_FLAG_JSON: u16 = 1 << 0;
/// Asset flag bit: the stored payload is xz-compressed. The `payload_len`
/// directory field refers to the compressed size on disk.
pub const ASSET_FLAG_XZ: u16 = 1 << 1;
/// Asset flag bit: the entry carries a trailing checksum.
pub const ASSET_FLAG_CHECKSUM: u16 = 1 << 2;

/// Default xz preset level used when compressing asset payloads.
///
/// Level 6 matches the `xz` CLI's own default and `xz2::XzEncoder::new`'s
/// default, and is a reasonable ratio/speed balance for JSON payloads.
pub const DEFAULT_XZ_PRESET: u32 = 6;

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

/// In-memory representation of the fixed 64-byte `.bendl` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BendlHeader {
    /// Magic bytes; should equal [`BENDL_MAGIC`].
    pub magic: [u8; 8],
    /// Incompatible-change version.
    pub major_version: u16,
    /// Additive backward-compatible version.
    pub minor_version: u16,
    /// `1` if the bundle was successfully finalized, else `0`.
    pub complete: u8,
    /// Container format of the embedded assignment stream.
    pub assignment_format: u8,
    /// Padding after `assignment_format`; writers set to zero, readers ignore.
    pub reserved_0: u16,
    /// Bundle-level feature flags.
    pub flags: u64,
    /// Absolute byte offset of the directory table, or `0` if no directory
    /// has been written yet. In a finalized bundle the directory lives at
    /// the end of the file.
    pub directory_offset: u64,
    /// Byte length of the directory table, or `0` if absent.
    pub directory_len: u64,
    /// Byte offset where the assignment stream begins.
    pub stream_offset: u64,
    /// Byte length of the assignment stream, or `0` if unfinalized.
    pub stream_len: u64,
    /// Number of expanded samples in the assignment stream, or `-1` if
    /// unfinalized.
    pub sample_count: i64,
}

impl BendlHeader {
    /// Build a provisional header used before any data has been written.
    pub fn provisional(assignment_format: AssignmentFormat, stream_offset: u64) -> Self {
        BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            complete: COMPLETE_NO,
            assignment_format: assignment_format.to_u8(),
            reserved_0: 0,
            flags: 0,
            directory_offset: 0,
            directory_len: 0,
            stream_offset,
            stream_len: 0,
            sample_count: -1,
        }
    }

    /// Whether the bundle has been finalized.
    pub fn is_complete(&self) -> bool {
        self.complete == COMPLETE_YES
    }

    /// Typed view of the embedded assignment format.
    pub fn assignment_format_typed(&self) -> Option<AssignmentFormat> {
        AssignmentFormat::from_u8(self.assignment_format)
    }

    /// Serialize the header into its fixed-size on-disk byte representation.
    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut out = [0u8; HEADER_SIZE];
        out[0..8].copy_from_slice(&self.magic);
        out[8..10].copy_from_slice(&self.major_version.to_le_bytes());
        out[10..12].copy_from_slice(&self.minor_version.to_le_bytes());
        out[12] = self.complete;
        out[13] = self.assignment_format;
        out[14..16].copy_from_slice(&self.reserved_0.to_le_bytes());
        out[16..24].copy_from_slice(&self.flags.to_le_bytes());
        out[24..32].copy_from_slice(&self.directory_offset.to_le_bytes());
        out[32..40].copy_from_slice(&self.directory_len.to_le_bytes());
        out[40..48].copy_from_slice(&self.stream_offset.to_le_bytes());
        out[48..56].copy_from_slice(&self.stream_len.to_le_bytes());
        out[56..64].copy_from_slice(&self.sample_count.to_le_bytes());
        out
    }

    /// Parse a fixed 64-byte header from its on-disk byte representation.
    pub fn from_bytes(bytes: &[u8; HEADER_SIZE]) -> Result<Self, BendlFormatError> {
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&bytes[0..8]);
        if magic != BENDL_MAGIC {
            return Err(BendlFormatError::InvalidMagic(magic));
        }

        let major_version = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
        let minor_version = u16::from_le_bytes(bytes[10..12].try_into().unwrap());
        if major_version != BENDL_MAJOR_VERSION {
            return Err(BendlFormatError::UnsupportedMajorVersion {
                found: major_version,
                supported: BENDL_MAJOR_VERSION,
            });
        }

        Ok(BendlHeader {
            magic,
            major_version,
            minor_version,
            complete: bytes[12],
            assignment_format: bytes[13],
            reserved_0: u16::from_le_bytes(bytes[14..16].try_into().unwrap()),
            flags: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            directory_offset: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            directory_len: u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
            stream_offset: u64::from_le_bytes(bytes[40..48].try_into().unwrap()),
            stream_len: u64::from_le_bytes(bytes[48..56].try_into().unwrap()),
            sample_count: i64::from_le_bytes(bytes[56..64].try_into().unwrap()),
        })
    }

    /// Read and parse a fixed header from a `Read` source.
    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self, BendlFormatError> {
        let mut buf = [0u8; HEADER_SIZE];
        reader.read_exact(&mut buf)?;
        Self::from_bytes(&buf)
    }

    /// Write the header to a `Write` sink.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(&self.to_bytes())
    }
}

// ---------------------------------------------------------------------------
// Directory entry
// ---------------------------------------------------------------------------

/// Fixed-size header at the start of every directory entry, before the
/// variable-length `name` and optional `checksum` bytes.
pub const DIRECTORY_ENTRY_HEADER_SIZE: usize = 28;

/// In-memory representation of a single directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BendlDirectoryEntry {
    /// Identifies the meaning of the payload (see `ASSET_TYPE_*`).
    pub asset_type: u16,
    /// Encoding/compression flags for this asset.
    pub asset_flags: u16,
    /// UTF-8 asset name. Must match the canonical name for singleton types.
    pub name: String,
    /// Absolute file offset of the asset payload.
    pub payload_offset: u64,
    /// Byte length of the asset payload as stored on disk (post-compression
    /// when the xz flag is set).
    pub payload_len: u64,
    /// Optional trailing checksum bytes. Interpretation depends on flags.
    pub checksum: Option<Vec<u8>>,
}

impl BendlDirectoryEntry {
    /// Total on-disk size of this entry, including name and checksum.
    pub fn encoded_len(&self) -> usize {
        DIRECTORY_ENTRY_HEADER_SIZE
            + self.name.len()
            + self.checksum.as_ref().map(|c| c.len()).unwrap_or(0)
    }

    /// Serialize the entry into a byte vector.
    pub fn to_bytes(&self) -> Result<Vec<u8>, BendlFormatError> {
        let name_bytes = self.name.as_bytes();
        let name_len: u16 =
            name_bytes
                .len()
                .try_into()
                .map_err(|_| BendlFormatError::NameTooLong {
                    length: name_bytes.len(),
                })?;
        let checksum_bytes = self.checksum.as_deref().unwrap_or(&[]);
        let checksum_len: u32 =
            checksum_bytes
                .len()
                .try_into()
                .map_err(|_| BendlFormatError::ChecksumTooLong {
                    length: checksum_bytes.len(),
                })?;

        let mut out = Vec::with_capacity(self.encoded_len());
        out.extend_from_slice(&self.asset_type.to_le_bytes());
        out.extend_from_slice(&self.asset_flags.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved
        out.extend_from_slice(&self.payload_offset.to_le_bytes());
        out.extend_from_slice(&self.payload_len.to_le_bytes());
        out.extend_from_slice(&checksum_len.to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(checksum_bytes);
        Ok(out)
    }

    /// Read one directory entry from a `Read` source.
    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self, BendlFormatError> {
        let mut header = [0u8; DIRECTORY_ENTRY_HEADER_SIZE];
        reader.read_exact(&mut header)?;

        let asset_type = u16::from_le_bytes(header[0..2].try_into().unwrap());
        let asset_flags = u16::from_le_bytes(header[2..4].try_into().unwrap());
        let name_len = u16::from_le_bytes(header[4..6].try_into().unwrap()) as usize;
        // header[6..8] reserved; ignored
        let payload_offset = u64::from_le_bytes(header[8..16].try_into().unwrap());
        let payload_len = u64::from_le_bytes(header[16..24].try_into().unwrap());
        let checksum_len = u32::from_le_bytes(header[24..28].try_into().unwrap()) as usize;

        let mut name_buf = vec![0u8; name_len];
        reader.read_exact(&mut name_buf)?;
        let name = String::from_utf8(name_buf).map_err(|_| BendlFormatError::NameNotUtf8)?;

        let checksum = if checksum_len == 0 {
            None
        } else {
            let mut buf = vec![0u8; checksum_len];
            reader.read_exact(&mut buf)?;
            Some(buf)
        };

        Ok(BendlDirectoryEntry {
            asset_type,
            asset_flags,
            name,
            payload_offset,
            payload_len,
            checksum,
        })
    }
}

// ---------------------------------------------------------------------------
// Directory table
// ---------------------------------------------------------------------------

/// Read the full directory table from a `Read` source.
///
/// The source should be positioned at the first byte of the directory
/// table (i.e. at `header.directory_offset`) and is expected to contain
/// exactly `entry_count` entries followed by no trailing bytes within the
/// directory region.
pub fn read_directory<R: Read>(
    reader: &mut R,
) -> Result<Vec<BendlDirectoryEntry>, BendlFormatError> {
    let mut count_buf = [0u8; 4];
    reader.read_exact(&mut count_buf)?;
    let entry_count = u32::from_le_bytes(count_buf) as usize;

    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        entries.push(BendlDirectoryEntry::read_from(reader)?);
    }
    Ok(entries)
}

/// Serialize a directory table into a byte vector.
pub fn encode_directory(entries: &[BendlDirectoryEntry]) -> Result<Vec<u8>, BendlFormatError> {
    let entry_count: u32 =
        entries
            .len()
            .try_into()
            .map_err(|_| BendlFormatError::TooManyEntries {
                length: entries.len(),
            })?;

    let body_len: usize = entries.iter().map(|e| e.encoded_len()).sum();
    let mut out = Vec::with_capacity(4 + body_len);
    out.extend_from_slice(&entry_count.to_le_bytes());
    for entry in entries {
        out.extend_from_slice(&entry.to_bytes()?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the `.bendl` format layer.
#[derive(Debug, Error)]
pub enum BendlFormatError {
    /// The file's leading magic bytes did not match [`BENDL_MAGIC`].
    #[error("invalid bendl magic: {0:02X?}")]
    InvalidMagic([u8; 8]),

    /// The file's major version is not supported by this implementation.
    #[error("unsupported bendl major version {found}: this implementation supports {supported}")]
    UnsupportedMajorVersion {
        /// Version actually found in the file.
        found: u16,
        /// Maximum major version this implementation can handle.
        supported: u16,
    },

    /// A directory entry's name exceeded the `u16` length limit.
    #[error("directory entry name is {length} bytes which exceeds the u16 length limit")]
    NameTooLong {
        /// The offending length in bytes.
        length: usize,
    },

    /// A directory entry's checksum exceeded the `u32` length limit.
    #[error("directory entry checksum is {length} bytes which exceeds the u32 length limit")]
    ChecksumTooLong {
        /// The offending length in bytes.
        length: usize,
    },

    /// A directory table exceeded the `u32` entry count limit.
    #[error("directory has {length} entries which exceeds the u32 entry count limit")]
    TooManyEntries {
        /// The offending entry count.
        length: usize,
    },

    /// A directory entry name was not valid UTF-8.
    #[error("directory entry name is not valid UTF-8")]
    NameNotUtf8,

    /// A directory table contained bytes after the declared entries.
    #[error("directory table has {remaining} trailing byte(s) after declared entries")]
    TrailingDirectoryBytes {
        /// Number of unread bytes left in the bounded directory region.
        remaining: u64,
    },

    /// A directory table violated bundle-level validation rules.
    #[error("malformed directory: {0}")]
    MalformedDirectory(String),

    /// An I/O error occurred while reading or writing the format layer.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

impl From<BendlFormatError> for io::Error {
    fn from(err: BendlFormatError) -> Self {
        match err {
            BendlFormatError::Io(e) => e,
            other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_is_eight_bytes_and_matches_spec() {
        assert_eq!(BENDL_MAGIC.len(), 8);
        assert_eq!(&BENDL_MAGIC[..5], b"BENDL");
    }

    #[test]
    fn canonical_name_lookup() {
        assert_eq!(
            canonical_name_for(ASSET_TYPE_METADATA),
            Some("metadata.json")
        );
        assert_eq!(canonical_name_for(ASSET_TYPE_GRAPH), Some("graph.json"));
        assert_eq!(
            canonical_name_for(ASSET_TYPE_RELABEL_MAP),
            Some("relabel_map.json")
        );
        assert_eq!(canonical_name_for(ASSET_TYPE_CUSTOM), None);
        assert_eq!(canonical_name_for(9999), None);
    }

    #[test]
    fn default_compression_policy() {
        assert!(default_compresses_by_type(ASSET_TYPE_GRAPH));
        assert!(!default_compresses_by_type(ASSET_TYPE_METADATA));
        assert!(!default_compresses_by_type(ASSET_TYPE_RELABEL_MAP));
        assert!(!default_compresses_by_type(ASSET_TYPE_CUSTOM));
    }

    #[test]
    fn assignment_format_roundtrip() {
        for fmt in [AssignmentFormat::Ben, AssignmentFormat::Xben] {
            assert_eq!(AssignmentFormat::from_u8(fmt.to_u8()), Some(fmt));
        }
        assert_eq!(AssignmentFormat::from_u8(0), None);
        assert_eq!(AssignmentFormat::from_u8(255), None);
    }

    #[test]
    fn header_is_exactly_64_bytes() {
        let header = BendlHeader::provisional(AssignmentFormat::Ben, 64);
        assert_eq!(header.to_bytes().len(), HEADER_SIZE);
        assert_eq!(HEADER_SIZE, 64);
    }

    #[test]
    fn header_round_trip_provisional() {
        let header = BendlHeader::provisional(AssignmentFormat::Xben, 64);
        let decoded = BendlHeader::from_bytes(&header.to_bytes()).unwrap();
        assert_eq!(header, decoded);
        assert!(!decoded.is_complete());
        assert_eq!(
            decoded.assignment_format_typed(),
            Some(AssignmentFormat::Xben)
        );
        assert_eq!(decoded.sample_count, -1);
        assert_eq!(decoded.stream_len, 0);
        assert_eq!(decoded.directory_offset, 0);
    }

    #[test]
    fn header_round_trip_finalized() {
        let header = BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            complete: COMPLETE_YES,
            assignment_format: ASSIGNMENT_FORMAT_BEN,
            reserved_0: 0,
            flags: 0x0000_0000_0000_000F,
            directory_offset: 1_000_000,
            directory_len: 256,
            stream_offset: 64,
            stream_len: 999_936,
            sample_count: 4242,
        };
        let bytes = header.to_bytes();
        let decoded = BendlHeader::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, header);
        assert!(decoded.is_complete());
    }

    #[test]
    fn header_rejects_invalid_magic() {
        let mut header = BendlHeader::provisional(AssignmentFormat::Ben, 64);
        header.magic = *b"NOTABEND";
        let err = BendlHeader::from_bytes(&header.to_bytes()).unwrap_err();
        assert!(matches!(err, BendlFormatError::InvalidMagic(_)));
    }

    #[test]
    fn header_rejects_unsupported_major_version() {
        let mut bytes = BendlHeader::provisional(AssignmentFormat::Ben, 64).to_bytes();
        bytes[8..10].copy_from_slice(&999u16.to_le_bytes());
        let err = BendlHeader::from_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            BendlFormatError::UnsupportedMajorVersion { found: 999, .. }
        ));
    }

    #[test]
    fn directory_entry_round_trip_no_checksum() {
        let entry = BendlDirectoryEntry {
            asset_type: ASSET_TYPE_GRAPH,
            asset_flags: ASSET_FLAG_JSON | ASSET_FLAG_XZ,
            name: CANONICAL_NAME_GRAPH.to_string(),
            payload_offset: 128,
            payload_len: 4096,
            checksum: None,
        };
        let bytes = entry.to_bytes().unwrap();
        assert_eq!(bytes.len(), entry.encoded_len());
        let mut cursor = &bytes[..];
        let decoded = BendlDirectoryEntry::read_from(&mut cursor).unwrap();
        assert_eq!(decoded, entry);
    }

    #[test]
    fn directory_entry_round_trip_with_checksum() {
        let entry = BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: ASSET_FLAG_CHECKSUM,
            name: "custom_blob".to_string(),
            payload_offset: 2048,
            payload_len: 512,
            checksum: Some(vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE]),
        };
        let bytes = entry.to_bytes().unwrap();
        let mut cursor = &bytes[..];
        let decoded = BendlDirectoryEntry::read_from(&mut cursor).unwrap();
        assert_eq!(decoded, entry);
        assert_eq!(
            decoded.checksum.unwrap(),
            vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE]
        );
    }

    #[test]
    fn directory_table_round_trip() {
        let entries = vec![
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_GRAPH,
                asset_flags: ASSET_FLAG_JSON | ASSET_FLAG_XZ,
                name: CANONICAL_NAME_GRAPH.to_string(),
                payload_offset: 64,
                payload_len: 2048,
                checksum: None,
            },
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_METADATA,
                asset_flags: ASSET_FLAG_JSON,
                name: CANONICAL_NAME_METADATA.to_string(),
                payload_offset: 2112,
                payload_len: 128,
                checksum: None,
            },
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_CUSTOM,
                asset_flags: 0,
                name: "provenance.bin".to_string(),
                payload_offset: 2240,
                payload_len: 32,
                checksum: None,
            },
        ];

        let encoded = encode_directory(&entries).unwrap();
        let mut cursor = &encoded[..];
        let decoded = read_directory(&mut cursor).unwrap();
        assert_eq!(decoded, entries);
    }

    #[test]
    fn empty_directory_table_round_trip() {
        let encoded = encode_directory(&[]).unwrap();
        assert_eq!(encoded, vec![0, 0, 0, 0]);
        let mut cursor = &encoded[..];
        let decoded = read_directory(&mut cursor).unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn header_and_directory_entry_header_sizes_are_stable() {
        // These sizes are baked into the on-disk format; regressing them
        // would silently break existing bundles.
        assert_eq!(HEADER_SIZE, 64);
        assert_eq!(DIRECTORY_ENTRY_HEADER_SIZE, 28);
    }
}
