//! Binary header and directory definitions for the `.bendl` container.
//!
//! This module is the pure format layer: it defines the on-disk byte layout, the associated
//! constants, and the encode/decode helpers that convert between in-memory Rust structs and their
//! on-disk representation. There is no I/O orchestration here — higher layers (`reader`, `writer`)
//! combine these primitives with seekable files.
//!
//! All multi-byte integers in the `.bendl` format are little-endian.

use std::io::{self, Read, Write};

use thiserror::Error;

// =====================================================================
// Magic, version, and header layout
// =====================================================================

/// Magic bytes at offset 0 of every `.bendl` file.
pub const BENDL_MAGIC: [u8; 8] = *b"BENDL\0\0\x01";

/// Current major version produced by this implementation.
pub const BENDL_MAJOR_VERSION: u16 = 1;
/// Current minor version produced by this implementation.
pub const BENDL_MINOR_VERSION: u16 = 0;

/// Size of the fixed header in bytes.
pub const HEADER_SIZE: usize = 64;

/// `finalized` flag value for incomplete (unfinalized) bundles.
pub const FINALIZED_NO: u8 = 0;
/// `finalized` flag value for finalized bundles.
pub const FINALIZED_YES: u8 = 1;

/// Header flag bit 0: the `stream_checksum` field contains a valid CRC32C over the on-disk
/// assignment stream bytes (`stream_offset..stream_offset + stream_len`). For XBEN streams the CRC
/// covers the compressed bytes, not the decompressed content. Bits 1..31 are reserved; writers set
/// them to zero.
///
/// Library writers always set this flag and write a valid checksum. The clear-flag state exists
/// only for adversarial reader fixtures and partial-recovery flows.
pub const HEADER_FLAG_STREAM_CHECKSUM: u32 = 1 << 0;

// =====================================================================
// Assignment format identifiers
// =====================================================================

/// Assignment format identifier: embedded BEN stream.
pub const ASSIGNMENT_FORMAT_BEN: u8 = 1;
/// Assignment format identifier: embedded XBEN stream.
pub const ASSIGNMENT_FORMAT_XBEN: u8 = 2;

/// Container format of the embedded assignment stream.
///
/// The BEN *variant* (`Standard`, `MkvChain`, `TwoDelta`) is carried by the 17-byte banner at the
/// start of the embedded stream and is not duplicated in the bundle header.
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

// =====================================================================
// Asset types, flags, standardized names
// =====================================================================

/// Asset type id for `metadata.json`.
pub const ASSET_TYPE_METADATA: u16 = 1;
/// Asset type id for `graph.json`.
pub const ASSET_TYPE_GRAPH: u16 = 2;
/// Asset type id for `node_permutation_map.json`.
pub const ASSET_TYPE_NODE_PERMUTATION_MAP: u16 = 3;
/// Asset type id for a custom user asset (name chosen by writer).
pub const ASSET_TYPE_CUSTOM: u16 = 4;

/// Standardized name for the `metadata.json` asset.
pub const STANDARDIZED_NAME_METADATA: &str = "metadata.json";
/// Standardized name for the `graph.json` asset.
pub const STANDARDIZED_NAME_GRAPH: &str = "graph.json";
/// Standardized name for the `node_permutation_map.json` asset.
pub const STANDARDIZED_NAME_NODE_PERMUTATION_MAP: &str = "node_permutation_map.json";

/// Return the standardized name reserved for a known singleton asset type, or `None` for custom or
/// unknown types.
pub fn standardized_name_for(asset_type: u16) -> Option<&'static str> {
    match asset_type {
        ASSET_TYPE_METADATA => Some(STANDARDIZED_NAME_METADATA),
        ASSET_TYPE_GRAPH => Some(STANDARDIZED_NAME_GRAPH),
        ASSET_TYPE_NODE_PERMUTATION_MAP => Some(STANDARDIZED_NAME_NODE_PERMUTATION_MAP),
        _ => None,
    }
}

/// One of the known singleton asset types reserved by the bundle format.
///
/// Each variant carries a fixed `asset_type` integer and a fixed standardized name. Custom assets
/// (writer-chosen name, multiple allowed) are not represented here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnownAssetKind {
    Metadata,
    Graph,
    NodePermutationMap,
}

impl KnownAssetKind {
    /// The asset-type integer reserved for this kind in the bundle format.
    pub fn asset_type(self) -> u16 {
        match self {
            Self::Metadata => ASSET_TYPE_METADATA,
            Self::Graph => ASSET_TYPE_GRAPH,
            Self::NodePermutationMap => ASSET_TYPE_NODE_PERMUTATION_MAP,
        }
    }

    /// The standardized filename reserved for this kind.
    pub fn standardized_name(self) -> &'static str {
        match self {
            Self::Metadata => STANDARDIZED_NAME_METADATA,
            Self::Graph => STANDARDIZED_NAME_GRAPH,
            Self::NodePermutationMap => STANDARDIZED_NAME_NODE_PERMUTATION_MAP,
        }
    }
}

/// Payload size at and above which the writer compresses an asset by default.
///
/// Below this, the xz container overhead (~60–90 bytes) can exceed the savings — a ~100-byte
/// `metadata.json` would *grow* under compression — so small payloads stay raw. At or above it,
/// the JSON/text payloads bundles typically carry (per-plan scores, node maps, provenance)
/// compress well for negligible CPU. An explicit [`AddAssetOptions::raw`] or
/// [`AddAssetOptions::compress`] always overrides the default.
///
/// [`AddAssetOptions::raw`]: super::writer::AddAssetOptions::raw
/// [`AddAssetOptions::compress`]: super::writer::AddAssetOptions::compress
pub const DEFAULT_ASSET_COMPRESSION_THRESHOLD: usize = 1024;

/// Return whether an asset should default to xz compression when the writer is not given an
/// explicit compression option: graphs always compress (they are the bundle's bulkiest JSON and
/// compress extremely well), and any other asset compresses once its payload reaches
/// [`DEFAULT_ASSET_COMPRESSION_THRESHOLD`].
pub fn default_compresses(asset_type: u16, payload_len: usize) -> bool {
    asset_type == ASSET_TYPE_GRAPH || payload_len >= DEFAULT_ASSET_COMPRESSION_THRESHOLD
}

/// Asset flag bit: the decoded payload is UTF-8 JSON.
pub const ASSET_FLAG_JSON: u16 = 1 << 0;
/// Asset flag bit: the stored payload is xz-compressed. The `payload_len` directory field refers to
/// the compressed size on disk.
pub const ASSET_FLAG_XZ: u16 = 1 << 1;
/// Asset flag bit: the entry carries a trailing checksum.
///
/// When set, the trailing checksum is exactly four little-endian bytes containing a CRC32C
/// (Castagnoli polynomial) over the **on-disk payload bytes** (`payload_offset..payload_offset +
/// payload_len`). For an xz-compressed asset the CRC is over the compressed bytes, not the
/// decompressed content — verification happens before decompression. Library writer paths always
/// set this flag with `checksum_len == [`ASSET_CHECKSUM_LEN`]`; readers reject any entry where the
/// flag and `checksum_len` are inconsistent (see [`BendlDirectoryEntry::read_from`]).
pub const ASSET_FLAG_CHECKSUM: u16 = 1 << 2;

/// On-disk byte width of an asset-payload CRC32C.
pub const ASSET_CHECKSUM_LEN: u32 = 4;

/// Default xz preset level used when compressing asset payloads.
///
/// Level 6 matches the `xz` CLI's own default and `xz2::XzEncoder::new`'s default, and is a
/// reasonable ratio/speed balance for JSON payloads.
pub const DEFAULT_XZ_PRESET: u32 = 6;

// =====================================================================
// Header
// =====================================================================

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
    pub finalized: u8,
    /// Container format of the embedded assignment stream.
    pub assignment_format: u8,
    /// Alignment padding after `assignment_format` that keeps the following 8-byte fields at
    /// offset ≥ 24 8-byte aligned. Writers set this to zero; readers ignore non-zero bytes.
    /// This is not a forward-compat slot — new fields must live elsewhere.
    pub alignment_padding: u16,
    /// Bundle-level feature flags (32-bit). See `HEADER_FLAG_*` constants. Bits without a defined
    /// constant are reserved; readers must ignore them and writers must set them to zero.
    pub flags: u32,
    /// CRC32C of the on-disk assignment stream bytes. Valid only when
    /// `HEADER_FLAG_STREAM_CHECKSUM` is set in `flags`. Writers set this to zero while the
    /// bundle is unfinalized and patch it on finalization.
    pub stream_checksum: u32,
    /// Absolute byte offset of the authoritative directory table, or `0` if no directory has been
    /// written yet. Successful finalization writes this directory after the assignment stream; a
    /// failed post-finalize append may leave newer orphaned bytes after the old authoritative
    /// directory until the header is patched.
    pub directory_offset: u64,
    /// Byte length of the directory table, or `0` if absent.
    pub directory_len: u64,
    /// Byte offset where the assignment stream begins.
    pub stream_offset: u64,
    /// Byte length of the assignment stream, or `0` if unfinalized.
    pub stream_len: u64,
    /// Number of expanded samples in the assignment stream, or `-1` if unfinalized.
    pub sample_count: i64,
}

impl BendlHeader {
    /// Build a provisional header used before any data has been written.
    pub fn provisional(assignment_format: AssignmentFormat, stream_offset: u64) -> Self {
        BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            finalized: FINALIZED_NO,
            assignment_format: assignment_format.to_u8(),
            alignment_padding: 0,
            flags: 0,
            stream_checksum: 0,
            directory_offset: 0,
            directory_len: 0,
            stream_offset,
            stream_len: 0,
            sample_count: -1,
        }
    }

    /// Whether the `HEADER_FLAG_STREAM_CHECKSUM` bit is set, indicating the `stream_checksum` field
    /// contains a valid CRC32C over the assignment stream bytes.
    pub fn has_stream_checksum(&self) -> bool {
        self.flags & HEADER_FLAG_STREAM_CHECKSUM != 0
    }

    /// Whether the bundle has been finalized.
    pub fn is_finalized(&self) -> bool {
        self.finalized == FINALIZED_YES
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
        out[12] = self.finalized;
        out[13] = self.assignment_format;
        out[14..16].copy_from_slice(&self.alignment_padding.to_le_bytes());
        out[16..20].copy_from_slice(&self.flags.to_le_bytes());
        out[20..24].copy_from_slice(&self.stream_checksum.to_le_bytes());
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
            finalized: bytes[12],
            assignment_format: bytes[13],
            alignment_padding: u16::from_le_bytes(bytes[14..16].try_into().unwrap()),
            flags: u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            stream_checksum: u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
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

// =====================================================================
// Directory entry
// =====================================================================

/// Fixed-size header at the start of every directory entry, before the variable-length `name` and
/// optional `checksum` bytes.
pub const DIRECTORY_ENTRY_HEADER_SIZE: usize = 28;

/// Upper bound on the number of directory entries a single bundle may declare.
///
/// A real bundle carries only a handful of assets — typically `graph.json`, a node-permutation
/// map, `metadata.json`, and at most a few small custom blobs — so this ceiling sits far above any
/// legitimate use while keeping the worst-case directory read bounded. The assignment stream is
/// stored outside the directory and does not count toward this limit, so a large ensemble does not
/// push against it.
///
/// [`read_directory`] rejects an inflated `entry_count` against this bound **before** allocating,
/// so a corrupt or adversarial header cannot trigger a multi-gigabyte reservation;
/// [`encode_directory`] enforces the same bound on the write side so the library never produces a
/// bundle it would refuse to read back.
pub const MAX_DIRECTORY_ENTRIES: u32 = 256;

/// In-memory representation of a single directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BendlDirectoryEntry {
    /// Identifies the meaning of the payload (see `ASSET_TYPE_*`).
    pub asset_type: u16,
    /// Encoding/compression flags for this asset.
    pub asset_flags: u16,
    /// UTF-8 asset name. Must match the standardized name for singleton types.
    pub name: String,
    /// Absolute file offset of the asset payload.
    pub payload_offset: u64,
    /// Byte length of the asset payload as stored on disk (post-compression when the xz flag is
    /// set).
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
        let checksum_len = checksum_bytes.len() as u32;

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
        let checksum_len_raw = u32::from_le_bytes(header[24..28].try_into().unwrap());

        // Reject (flag, checksum_len) inconsistencies before allocating anything.
        let flag_set = asset_flags & ASSET_FLAG_CHECKSUM != 0;
        match (flag_set, checksum_len_raw) {
            (true, ASSET_CHECKSUM_LEN) => {}
            (false, 0) => {}
            _ => {
                return Err(BendlFormatError::InconsistentChecksumMetadata {
                    flag_set,
                    checksum_len: checksum_len_raw,
                });
            }
        }
        let checksum_len = checksum_len_raw as usize;

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

    /// Return the stored CRC32C as a `u32`, if and only if the entry carries a valid checksum
    /// (flag set, 4 bytes).
    ///
    /// This is the canonical accessor for verification code. Returns `None` for entries with
    /// `ASSET_FLAG_CHECKSUM` clear; entries where the flag and length are inconsistent are rejected
    /// at read time and so cannot reach this method.
    pub fn checksum_u32(&self) -> Option<u32> {
        if self.asset_flags & ASSET_FLAG_CHECKSUM == 0 {
            return None;
        }
        let bytes = self.checksum.as_deref()?;
        if bytes.len() != ASSET_CHECKSUM_LEN as usize {
            return None;
        }
        Some(u32::from_le_bytes(bytes.try_into().ok()?))
    }
}

// =====================================================================
// Directory table
// =====================================================================

/// Read the full directory table from a `Read` source.
///
/// The source should be positioned at the first byte of the directory table (i.e. at
/// `header.directory_offset`) and is expected to contain exactly `entry_count` entries followed by
/// no trailing bytes within the directory region.
pub fn read_directory<R: Read>(
    reader: &mut R,
) -> Result<Vec<BendlDirectoryEntry>, BendlFormatError> {
    let mut count_buf = [0u8; 4];
    reader.read_exact(&mut count_buf)?;
    let entry_count = u32::from_le_bytes(count_buf);

    // Reject an inflated count before allocating: `entry_count` is untrusted on-disk data, and
    // `Vec::with_capacity` would otherwise reserve `entry_count * size_of::<BendlDirectoryEntry>()`
    // bytes up front — a `u32::MAX` count aborts the process on the allocation rather than failing
    // gracefully on the missing entry bytes.
    if entry_count > MAX_DIRECTORY_ENTRIES {
        return Err(BendlFormatError::TooManyDirectoryEntries {
            count: entry_count as u64,
            max: MAX_DIRECTORY_ENTRIES,
        });
    }
    let entry_count = entry_count as usize;

    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        entries.push(BendlDirectoryEntry::read_from(reader)?);
    }
    Ok(entries)
}

/// Serialize a directory table into a byte vector.
pub fn encode_directory(entries: &[BendlDirectoryEntry]) -> Result<Vec<u8>, BendlFormatError> {
    // Enforce the same ceiling the reader applies, so the library never writes a bundle it would
    // refuse to read back.
    if entries.len() > MAX_DIRECTORY_ENTRIES as usize {
        return Err(BendlFormatError::TooManyDirectoryEntries {
            count: entries.len() as u64,
            max: MAX_DIRECTORY_ENTRIES,
        });
    }
    let entry_count = entries.len() as u32;

    let body_len: usize = entries.iter().map(|e| e.encoded_len()).sum();
    let mut out = Vec::with_capacity(4 + body_len);
    out.extend_from_slice(&entry_count.to_le_bytes());
    for entry in entries {
        out.extend_from_slice(&entry.to_bytes()?);
    }
    Ok(out)
}

// =====================================================================
// Errors
// =====================================================================

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

    /// A directory entry name was not valid UTF-8.
    #[error("directory entry name is not valid UTF-8")]
    NameNotUtf8,

    /// A directory table contained bytes after the declared entries.
    #[error("directory table has {remaining} trailing byte(s) after declared entries")]
    TrailingDirectoryBytes {
        /// Number of unread bytes left in the bounded directory region.
        remaining: u64,
    },

    /// A directory declared more entries than [`MAX_DIRECTORY_ENTRIES`] allows. Rejected before
    /// any allocation so an inflated on-disk count cannot trigger a huge reservation.
    #[error("directory declares {count} entries, which exceeds the maximum of {max}")]
    TooManyDirectoryEntries {
        /// The entry count declared in the directory header (read path) or requested by the
        /// writer.
        count: u64,
        /// The maximum permitted entry count ([`MAX_DIRECTORY_ENTRIES`]).
        max: u32,
    },

    /// A directory table violated bundle-level validation rules.
    #[error("malformed directory: {0}")]
    MalformedDirectory(String),

    /// The header's `assignment_format` byte did not map to any known assignment format.
    #[error("unknown assignment_format byte in bundle header: {0}")]
    UnknownAssignmentFormat(u8),

    /// A directory entry's `ASSET_FLAG_CHECKSUM` bit and `checksum_len` disagree. The wire format
    /// requires `flag set iff checksum_len == 4` and `flag clear iff checksum_len == 0`.
    #[error(
        "inconsistent checksum metadata: ASSET_FLAG_CHECKSUM={flag_set}, checksum_len={checksum_len}"
    )]
    InconsistentChecksumMetadata {
        /// Whether the entry had the `ASSET_FLAG_CHECKSUM` bit set.
        flag_set: bool,
        /// The trailing-checksum length the entry actually declared.
        checksum_len: u32,
    },

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
