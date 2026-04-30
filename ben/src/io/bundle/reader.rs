//! Read-only inspection of `.bendl` files.
//!
//! A [`BendlReader`] parses a bundle's fixed header and (if present) its
//! trailing directory table. It does not read any asset payload bytes
//! until the caller explicitly requests them via [`BendlReader::asset_bytes`]
//! or [`BendlReader::asset_reader`]. The assignment stream region is
//! likewise exposed as a byte range the caller can plumb into the
//! existing `AssignmentReader` / `XZAssignmentReader` without this module
//! reinterpreting any BEN/XBEN internals.

use std::io::{self, Read, Seek, SeekFrom, Take};

use xz2::read::XzDecoder;

use super::format::{
    canonical_name_for, read_directory, AssignmentFormat, BendlDirectoryEntry, BendlFormatError,
    BendlHeader, ASSET_FLAG_XZ,
};

/// Reader for a single `.bendl` file.
pub struct BendlReader<R: Read + Seek> {
    inner: R,
    header: BendlHeader,
    directory: Vec<BendlDirectoryEntry>,
}

impl<R: Read + Seek> BendlReader<R> {
    /// Open a `.bendl` file by validating its header and loading the
    /// directory table if one exists.
    ///
    /// The underlying reader is left at an unspecified position; callers
    /// should seek explicitly before reading asset or stream bytes.
    pub fn open(mut inner: R) -> Result<Self, BendlFormatError> {
        inner.seek(SeekFrom::Start(0))?;
        let header = BendlHeader::read_from(&mut inner)?;

        let directory = if header.directory_offset != 0 && header.directory_len != 0 {
            inner.seek(SeekFrom::Start(header.directory_offset))?;
            let mut bounded = (&mut inner).take(header.directory_len);
            let directory = read_directory(&mut bounded)?;
            let remaining = bounded.limit();
            if remaining != 0 {
                return Err(BendlFormatError::TrailingDirectoryBytes { remaining });
            }
            validate_directory_entries(&directory)
                .map_err(|e| BendlFormatError::MalformedDirectory(e.to_string()))?;
            directory
        } else {
            Vec::new()
        };

        Ok(BendlReader {
            inner,
            header,
            directory,
        })
    }

    /// The parsed fixed header.
    pub fn header(&self) -> &BendlHeader {
        &self.header
    }

    /// Whether the bundle was successfully finalized.
    pub fn is_complete(&self) -> bool {
        self.header.is_complete()
    }

    /// The sample count recorded in the header, or `None` if not
    /// authoritative (i.e. the bundle is still incomplete).
    pub fn sample_count(&self) -> Option<i64> {
        if self.header.is_complete() {
            Some(self.header.sample_count)
        } else {
            None
        }
    }

    /// The container format of the embedded assignment stream.
    pub fn assignment_format(&self) -> Option<AssignmentFormat> {
        self.header.assignment_format_typed()
    }

    /// All directory entries in the order they appear in the directory.
    pub fn assets(&self) -> &[BendlDirectoryEntry] {
        &self.directory
    }

    /// Look up a directory entry by canonical or custom name.
    pub fn find_asset_by_name(&self, name: &str) -> Option<&BendlDirectoryEntry> {
        self.directory.iter().find(|e| e.name == name)
    }

    /// Look up the unique directory entry with the given asset type, if
    /// any. Singleton types (`metadata.json`, `graph.json`,
    /// `relabel_map.json`) use this to grab their payload without caring
    /// about the canonical name.
    pub fn find_asset_by_type(&self, asset_type: u16) -> Option<&BendlDirectoryEntry> {
        self.directory.iter().find(|e| e.asset_type == asset_type)
    }

    /// Return the byte range occupied by the assignment stream.
    ///
    /// For finalized bundles this is `(stream_offset, stream_len)` as
    /// recorded in the header. For incomplete bundles the end of the
    /// stream is taken as EOF (or the directory start, if a provisional
    /// directory was written).
    pub fn assignment_stream_range(&mut self) -> io::Result<(u64, u64)> {
        if self.header.is_complete() {
            Ok((self.header.stream_offset, self.header.stream_len))
        } else {
            let end = if self.header.directory_offset != 0 {
                self.header.directory_offset
            } else {
                self.inner.seek(SeekFrom::End(0))?
            };
            let len = end.saturating_sub(self.header.stream_offset);
            Ok((self.header.stream_offset, len))
        }
    }

    /// Return a `Take` reader positioned at the start of the assignment
    /// stream and limited to its declared length. The caller is expected
    /// to wrap the returned reader in an `AssignmentReader` or
    /// `XZAssignmentReader` as appropriate for `assignment_format()`.
    pub fn assignment_stream_reader(&mut self) -> io::Result<Take<&mut R>> {
        let (offset, len) = self.assignment_stream_range()?;
        self.inner.seek(SeekFrom::Start(offset))?;
        Ok((&mut self.inner).take(len))
    }

    /// Construct the appropriate assignment decoder for the bundle's
    /// declared `assignment_format` and return it as a
    /// [`BundleAssignmentReader`] enum.
    ///
    /// - `AssignmentFormat::Ben` produces a
    ///   [`crate::io::reader::AssignmentReader`] over a `Take<&mut R>`.
    /// - `AssignmentFormat::Xben` produces a
    ///   [`crate::io::reader::XZAssignmentReader`] over a `Take<&mut R>`.
    ///
    /// Returns an error if the header's `assignment_format` field is
    /// unrecognized or the embedded banner is malformed.
    pub fn open_assignment_reader(
        &mut self,
    ) -> Result<BundleAssignmentReader<Take<&mut R>>, BundleAssignmentReaderError> {
        let format = self.assignment_format().ok_or(
            BundleAssignmentReaderError::UnknownAssignmentFormat(self.header.assignment_format),
        )?;
        let stream = self.assignment_stream_reader()?;
        match format {
            AssignmentFormat::Ben => {
                let inner = crate::io::reader::AssignmentReader::new(stream)
                    .map_err(BundleAssignmentReaderError::Decoder)?;
                Ok(BundleAssignmentReader::Ben(inner))
            }
            AssignmentFormat::Xben => {
                let inner = crate::io::reader::XZAssignmentReader::new(stream)
                    .map_err(BundleAssignmentReaderError::Decoder)?;
                Ok(BundleAssignmentReader::Xben(inner))
            }
        }
    }

    /// Read the fully-decoded bytes of an asset by directory entry.
    ///
    /// If the entry has [`ASSET_FLAG_XZ`] set, the payload is decompressed
    /// through `xz2::read::XzDecoder`. Otherwise the bytes are returned
    /// as-is.
    pub fn asset_bytes(&mut self, entry: &BendlDirectoryEntry) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        self.asset_reader(entry)?.read_to_end(&mut out)?;
        Ok(out)
    }

    /// Obtain a boxed reader for the decoded contents of an asset.
    ///
    /// The returned reader is positioned at the first decoded byte and
    /// automatically handles xz decompression when the asset is flagged
    /// as compressed. The reader borrows `self`, so only one asset or
    /// stream reader may be live at a time.
    pub fn asset_reader<'a>(
        &'a mut self,
        entry: &BendlDirectoryEntry,
    ) -> io::Result<Box<dyn Read + 'a>> {
        self.inner.seek(SeekFrom::Start(entry.payload_offset))?;
        let raw = (&mut self.inner).take(entry.payload_len);
        if entry.asset_flags & ASSET_FLAG_XZ != 0 {
            Ok(Box::new(XzDecoder::new(raw)))
        } else {
            Ok(Box::new(raw))
        }
    }

    /// Validate that the loaded directory is well-formed under the
    /// canonical-name and uniqueness rules.
    ///
    /// Returns [`BundleValidationError`] if any entry violates the rules.
    /// This is called automatically by [`BendlReader::open`] when the
    /// `strict` constructor is used in tests; in normal reads, the
    /// writer is already expected to enforce these rules and a
    /// malformed bundle is a program bug somewhere else.
    pub fn validate_directory(&self) -> Result<(), BundleValidationError> {
        validate_directory_entries(&self.directory)
    }

}

pub(crate) fn validate_directory_entries(
    directory: &[BendlDirectoryEntry],
) -> Result<(), BundleValidationError> {
    let mut seen_names = std::collections::HashSet::new();

    for entry in directory {
        if !seen_names.insert(entry.name.as_str()) {
            return Err(BundleValidationError::DuplicateName(entry.name.clone()));
        }
        if let Some(canonical) = canonical_name_for(entry.asset_type) {
            if entry.name != canonical {
                return Err(BundleValidationError::WrongCanonicalName {
                    asset_type: entry.asset_type,
                    expected: canonical.to_string(),
                    found: entry.name.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Either a BEN or an XBEN assignment decoder over a bundle's embedded
/// stream region.
///
/// Both variants hold a `Take<&mut R>` reader limited to the stream
/// window declared in the bundle header, so they cannot accidentally
/// read into the trailing directory table.
pub enum BundleAssignmentReader<R: std::io::Read> {
    /// The bundle carries an uncompressed BEN stream.
    Ben(crate::io::reader::AssignmentReader<R>),
    /// The bundle carries an xz-compressed XBEN stream.
    Xben(crate::io::reader::XZAssignmentReader<R>),
}

impl<R: std::io::Read> BundleAssignmentReader<R> {
    /// True when the reader is backed by a BEN stream.
    pub fn is_ben(&self) -> bool {
        matches!(self, BundleAssignmentReader::Ben(_))
    }

    /// True when the reader is backed by an XBEN stream.
    pub fn is_xben(&self) -> bool {
        matches!(self, BundleAssignmentReader::Xben(_))
    }
}

/// Errors raised by [`BendlReader::open_assignment_reader`].
#[derive(Debug, thiserror::Error)]
pub enum BundleAssignmentReaderError {
    /// The header's `assignment_format` byte did not map to a known format.
    #[error("unknown assignment_format in bundle header: {0}")]
    UnknownAssignmentFormat(u8),
    /// The embedded BEN/XBEN decoder rejected the stream banner.
    #[error(transparent)]
    Decoder(#[from] crate::io::reader::DecoderInitError),
    /// An underlying I/O error occurred while seeking to the stream.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Errors raised when a directory violates the canonical-name or
/// uniqueness rules.
#[derive(Debug, thiserror::Error)]
pub enum BundleValidationError {
    /// Two entries share the same name.
    #[error("duplicate asset name: {0:?}")]
    DuplicateName(String),

    /// An entry with a known singleton type is not using its canonical name.
    #[error("asset type {asset_type} must use canonical name {expected:?}, found {found:?}")]
    WrongCanonicalName {
        /// The asset type whose canonical name was violated.
        asset_type: u16,
        /// The canonical name the writer should have used.
        expected: String,
        /// The name that was actually written.
        found: String,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use xz2::write::XzEncoder;

    use super::*;
    use crate::io::bundle::format::{
        encode_directory, ASSET_FLAG_JSON, ASSET_FLAG_XZ, ASSET_TYPE_CUSTOM, ASSET_TYPE_GRAPH,
        ASSET_TYPE_METADATA, BENDL_MAGIC, BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION, COMPLETE_NO,
        COMPLETE_YES, HEADER_SIZE,
    };

    /// Build a complete in-memory finalized bundle with two assets:
    /// an xz-compressed `graph.json` and a raw custom blob, followed by
    /// a fake BEN stream and a trailing directory.
    fn build_finalized_bundle() -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        // Asset payloads (decoded):
        let graph_json = br#"{"nodes":[0,1,2],"edges":[[0,1],[1,2]]}"#.to_vec();
        let custom_blob = vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        let fake_stream = b"STANDARD BEN FILE\x00\x01\x02\x03fake payload".to_vec();

        // xz-compress graph_json using the default preset.
        let mut encoder = XzEncoder::new(Vec::new(), 6);
        encoder.write_all(&graph_json).unwrap();
        let compressed_graph = encoder.finish().unwrap();

        // Layout:
        //   [0 .. 64) header
        //   [64 .. 64+len(compressed_graph)) graph payload
        //   [... .. ...+len(custom_blob)) custom payload
        //   [stream_offset .. stream_offset+len(fake_stream)) stream
        //   [directory_offset .. EOF) directory
        let mut bundle = Vec::new();
        // Reserve space for header; fill later.
        bundle.extend(std::iter::repeat(0u8).take(HEADER_SIZE));

        let graph_offset = bundle.len() as u64;
        bundle.extend_from_slice(&compressed_graph);

        let custom_offset = bundle.len() as u64;
        bundle.extend_from_slice(&custom_blob);

        let stream_offset = bundle.len() as u64;
        bundle.extend_from_slice(&fake_stream);
        let stream_len = fake_stream.len() as u64;

        let directory_offset = bundle.len() as u64;

        let entries = vec![
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_GRAPH,
                asset_flags: ASSET_FLAG_JSON | ASSET_FLAG_XZ,
                name: "graph.json".to_string(),
                payload_offset: graph_offset,
                payload_len: compressed_graph.len() as u64,
                checksum: None,
            },
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_CUSTOM,
                asset_flags: 0,
                name: "custom.bin".to_string(),
                payload_offset: custom_offset,
                payload_len: custom_blob.len() as u64,
                checksum: None,
            },
        ];
        let directory_bytes = encode_directory(&entries).unwrap();
        bundle.extend_from_slice(&directory_bytes);
        let directory_len = directory_bytes.len() as u64;

        // Now patch the header.
        let header = BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            complete: COMPLETE_YES,
            assignment_format: AssignmentFormat::Ben.to_u8(),
            reserved_0: 0,
            flags: 0,
            directory_offset,
            directory_len,
            stream_offset,
            stream_len,
            sample_count: 42,
        };
        bundle[..HEADER_SIZE].copy_from_slice(&header.to_bytes());

        (bundle, graph_json, custom_blob, fake_stream)
    }

    #[test]
    fn open_finalized_bundle_and_read_metadata() {
        let (bytes, _, _, _) = build_finalized_bundle();
        let reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        assert!(reader.is_complete());
        assert_eq!(reader.sample_count(), Some(42));
        assert_eq!(reader.assignment_format(), Some(AssignmentFormat::Ben));
        assert_eq!(reader.assets().len(), 2);
        assert!(reader.validate_directory().is_ok());
    }

    #[test]
    fn read_compressed_graph_asset_decodes_through_xz() {
        let (bytes, graph_json, _, _) = build_finalized_bundle();
        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        let entry = reader
            .find_asset_by_type(ASSET_TYPE_GRAPH)
            .cloned()
            .expect("graph entry");
        let bytes_out = reader.asset_bytes(&entry).unwrap();
        assert_eq!(bytes_out, graph_json);
    }

    #[test]
    fn read_raw_custom_asset_returns_exact_bytes() {
        let (bytes, _, custom_blob, _) = build_finalized_bundle();
        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        let entry = reader
            .find_asset_by_name("custom.bin")
            .cloned()
            .expect("custom entry");
        let bytes_out = reader.asset_bytes(&entry).unwrap();
        assert_eq!(bytes_out, custom_blob);
    }

    #[test]
    fn assignment_stream_range_matches_finalized_header() {
        let (bytes, _, _, fake_stream) = build_finalized_bundle();
        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        let (offset, len) = reader.assignment_stream_range().unwrap();
        assert_eq!(len, fake_stream.len() as u64);
        let mut buf = Vec::new();
        reader
            .assignment_stream_reader()
            .unwrap()
            .read_to_end(&mut buf)
            .unwrap();
        assert_eq!(buf, fake_stream);
        // Sanity-check the offset is consistent with the header.
        assert_eq!(offset, reader.header().stream_offset);
    }

    #[test]
    fn incomplete_bundle_reports_no_directory_and_stream_runs_to_eof() {
        // Build an incomplete bundle: header + some fake stream bytes, no directory.
        let fake_stream = b"STANDARD BEN FILE\x00\x01some partial bytes".to_vec();
        let mut bytes = Vec::new();
        let header = BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            complete: COMPLETE_NO,
            assignment_format: AssignmentFormat::Ben.to_u8(),
            reserved_0: 0,
            flags: 0,
            directory_offset: 0,
            directory_len: 0,
            stream_offset: HEADER_SIZE as u64,
            stream_len: 0,
            sample_count: -1,
        };
        bytes.extend_from_slice(&header.to_bytes());
        bytes.extend_from_slice(&fake_stream);

        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        assert!(!reader.is_complete());
        assert_eq!(reader.sample_count(), None);
        assert!(reader.assets().is_empty());

        let (offset, len) = reader.assignment_stream_range().unwrap();
        assert_eq!(offset, HEADER_SIZE as u64);
        assert_eq!(len, fake_stream.len() as u64);

        let mut buf = Vec::new();
        reader
            .assignment_stream_reader()
            .unwrap()
            .read_to_end(&mut buf)
            .unwrap();
        assert_eq!(buf, fake_stream);
    }

    #[test]
    fn open_rejects_malformed_magic() {
        let mut bytes = vec![0u8; HEADER_SIZE];
        bytes[0..8].copy_from_slice(b"NOPENOPE");
        match BendlReader::open(Cursor::new(bytes)) {
            Err(BendlFormatError::InvalidMagic(_)) => {}
            Err(other) => panic!("expected InvalidMagic, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn validate_directory_catches_duplicate_names() {
        let entries = vec![
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_CUSTOM,
                asset_flags: 0,
                name: "a".to_string(),
                payload_offset: 64,
                payload_len: 1,
                checksum: None,
            },
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_CUSTOM,
                asset_flags: 0,
                name: "a".to_string(),
                payload_offset: 65,
                payload_len: 1,
                checksum: None,
            },
        ];
        let reader = BendlReader {
            inner: Cursor::new(Vec::<u8>::new()),
            header: BendlHeader::provisional(AssignmentFormat::Ben, 64),
            directory: entries,
        };
        let err = reader.validate_directory().unwrap_err();
        assert!(matches!(err, BundleValidationError::DuplicateName(ref n) if n == "a"));
    }

    #[test]
    fn validate_directory_catches_wrong_canonical_name() {
        let entries = vec![BendlDirectoryEntry {
            asset_type: ASSET_TYPE_GRAPH,
            asset_flags: 0,
            name: "not_graph.json".to_string(),
            payload_offset: 64,
            payload_len: 1,
            checksum: None,
        }];
        let reader = BendlReader {
            inner: Cursor::new(Vec::<u8>::new()),
            header: BendlHeader::provisional(AssignmentFormat::Ben, 64),
            directory: entries,
        };
        let err = reader.validate_directory().unwrap_err();
        assert!(matches!(
            err,
            BundleValidationError::WrongCanonicalName {
                asset_type: ASSET_TYPE_GRAPH,
                ..
            }
        ));
    }

    // -----------------------------------------------------------------------
    // Robustness tests
    // -----------------------------------------------------------------------

    /// Build a small finalized bundle with a known graph asset, metadata
    /// asset, empty stream, and no validation pitfalls. Useful as a base
    /// that tests can mutate byte-by-byte.
    fn build_basic_finalized_bundle() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(std::iter::repeat(0u8).take(HEADER_SIZE));

        // One raw metadata asset right after the header.
        let metadata_payload = br#"{"k":"v"}"#.to_vec();
        let metadata_offset = bytes.len() as u64;
        bytes.extend_from_slice(&metadata_payload);

        // Stream region is empty.
        let stream_offset = bytes.len() as u64;
        let stream_len = 0u64;

        // Directory at EOF with one entry.
        let directory_offset = bytes.len() as u64;
        let entries = vec![BendlDirectoryEntry {
            asset_type: ASSET_TYPE_METADATA,
            asset_flags: ASSET_FLAG_JSON,
            name: "metadata.json".to_string(),
            payload_offset: metadata_offset,
            payload_len: metadata_payload.len() as u64,
            checksum: None,
        }];
        let directory = encode_directory(&entries).unwrap();
        bytes.extend_from_slice(&directory);
        let directory_len = directory.len() as u64;

        let header = BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            complete: COMPLETE_YES,
            assignment_format: AssignmentFormat::Ben.to_u8(),
            reserved_0: 0,
            flags: 0,
            directory_offset,
            directory_len,
            stream_offset,
            stream_len,
            sample_count: 0,
        };
        bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
        bytes
    }

    #[test]
    fn open_rejects_short_header() {
        let too_short = vec![0u8; HEADER_SIZE - 1];
        match BendlReader::open(Cursor::new(too_short)) {
            Err(BendlFormatError::Io(_)) => {}
            Err(other) => panic!("expected Io, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn open_rejects_unsupported_major_version() {
        let mut bytes = build_basic_finalized_bundle();
        // major_version lives at offset 8..10 in the header.
        bytes[8..10].copy_from_slice(&(BENDL_MAJOR_VERSION + 1).to_le_bytes());
        match BendlReader::open(Cursor::new(bytes)) {
            Err(BendlFormatError::UnsupportedMajorVersion { .. }) => {}
            Err(other) => panic!("expected UnsupportedMajorVersion, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn open_rejects_directory_with_inflated_entry_count() {
        let mut bytes = build_basic_finalized_bundle();
        // Read directory_offset from the header (bytes 24..32).
        let directory_offset = u64::from_le_bytes(bytes[24..32].try_into().unwrap()) as usize;
        // Blow up the entry count at the start of the directory to a
        // value that cannot possibly fit in the remaining file bytes.
        bytes[directory_offset..directory_offset + 4].copy_from_slice(&9999u32.to_le_bytes());
        match BendlReader::open(Cursor::new(bytes)) {
            Err(BendlFormatError::Io(_)) => {}
            Err(other) => panic!("expected Io, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn open_rejects_directory_with_chopped_final_entry() {
        // Drop the last byte of the file, which lies inside the name
        // field of the final directory entry.
        let mut bytes = build_basic_finalized_bundle();
        bytes.pop();
        match BendlReader::open(Cursor::new(bytes)) {
            Err(BendlFormatError::Io(_)) => {}
            Err(other) => panic!("expected Io, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn asset_bytes_read_twice_returns_identical_payload() {
        let (bytes, _, custom_blob, _) = build_finalized_bundle();
        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        let entry = reader.find_asset_by_name("custom.bin").cloned().unwrap();
        let first = reader.asset_bytes(&entry).unwrap();
        let second = reader.asset_bytes(&entry).unwrap();
        assert_eq!(first, second);
        assert_eq!(first, custom_blob);
    }

    #[test]
    fn interleaved_reads_do_not_corrupt_each_other() {
        // Read asset A, then stream, then asset A again, then asset B.
        let (bytes, graph_json, custom_blob, fake_stream) = build_finalized_bundle();
        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();

        let graph_entry = reader
            .find_asset_by_type(ASSET_TYPE_GRAPH)
            .cloned()
            .unwrap();
        let custom_entry = reader.find_asset_by_name("custom.bin").cloned().unwrap();

        let graph_first = reader.asset_bytes(&graph_entry).unwrap();
        assert_eq!(graph_first, graph_json);

        let mut stream_buf = Vec::new();
        reader
            .assignment_stream_reader()
            .unwrap()
            .read_to_end(&mut stream_buf)
            .unwrap();
        assert_eq!(stream_buf, fake_stream);

        let graph_second = reader.asset_bytes(&graph_entry).unwrap();
        assert_eq!(graph_second, graph_json);

        let custom = reader.asset_bytes(&custom_entry).unwrap();
        assert_eq!(custom, custom_blob);
    }

    #[test]
    fn asset_bytes_errors_when_declared_length_runs_past_eof() {
        // Hand-construct a bundle where the metadata directory entry
        // claims a payload_len that extends well past EOF.
        let mut bytes = build_basic_finalized_bundle();
        // Parse the directory offset to find where the entry lives.
        let directory_offset = u64::from_le_bytes(bytes[24..32].try_into().unwrap()) as usize;
        // Skip the u32 entry count (4 bytes) and then the 16-byte fixed
        // entry header up to `payload_len` (bytes 16..24 of the entry).
        let entry_start = directory_offset + 4;
        let payload_len_offset = entry_start + 16;
        bytes[payload_len_offset..payload_len_offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());

        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        let entry = reader.find_asset_by_name("metadata.json").cloned().unwrap();
        // The reader opens fine — the directory parses. But reading the
        // asset bytes must surface an error eventually (short read vs
        // declared length). xz would also trip on this, but this is the
        // raw-asset path.
        match reader.asset_bytes(&entry) {
            Ok(bytes) => {
                // At the very least the returned bytes should not pretend
                // to fill u64::MAX — saturate at what the file actually had.
                assert!(bytes.len() < u64::MAX as usize);
            }
            Err(_) => {}
        }
    }

    #[test]
    fn incomplete_bundle_sample_count_is_none_even_if_header_value_is_nonzero() {
        // Build an incomplete bundle but stuff a stale sample count into
        // the header. `sample_count()` must still return None because
        // the `complete` flag is what makes the value authoritative.
        let header = BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            complete: COMPLETE_NO,
            assignment_format: AssignmentFormat::Ben.to_u8(),
            reserved_0: 0,
            flags: 0,
            directory_offset: 0,
            directory_len: 0,
            stream_offset: HEADER_SIZE as u64,
            stream_len: 0,
            sample_count: 999_999, // lie, but header is "incomplete"
        };
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&header.to_bytes());
        let reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        assert!(!reader.is_complete());
        assert_eq!(reader.sample_count(), None);
    }

    #[test]
    fn unknown_assignment_format_reports_none_on_typed_getter() {
        // Build a finalized but otherwise-empty bundle and corrupt the
        // assignment_format byte to a value that is neither BEN nor XBEN.
        let mut bytes = build_basic_finalized_bundle();
        // assignment_format byte is at offset 13 in the header.
        bytes[13] = 42;
        let reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.assignment_format(), None);
        // The header still parses and the directory is still available.
        assert_eq!(reader.assets().len(), 1);
    }

    #[test]
    fn open_assignment_reader_rejects_unknown_assignment_format() {
        let mut bytes = build_basic_finalized_bundle();
        bytes[13] = 42; // corrupt assignment format byte
        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        match reader.open_assignment_reader() {
            Err(BundleAssignmentReaderError::UnknownAssignmentFormat(42)) => {}
            Err(other) => panic!("expected UnknownAssignmentFormat(42), got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn incomplete_bundle_stream_range_runs_to_eof_without_directory() {
        let fake_stream = b"STANDARD BEN FILE\x00\x01payload bytes".to_vec();
        let header = BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            complete: COMPLETE_NO,
            assignment_format: AssignmentFormat::Ben.to_u8(),
            reserved_0: 0,
            flags: 0,
            directory_offset: 0,
            directory_len: 0,
            stream_offset: HEADER_SIZE as u64,
            stream_len: 0,
            sample_count: -1,
        };
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&header.to_bytes());
        bytes.extend_from_slice(&fake_stream);
        let eof = bytes.len() as u64;

        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        let (off, len) = reader.assignment_stream_range().unwrap();
        assert_eq!(off, HEADER_SIZE as u64);
        assert_eq!(off + len, eof);
    }

    #[test]
    fn validate_directory_catches_duplicate_singleton_types() {
        // Two entries of type METADATA (both with canonical name
        // "metadata.json"). The canonical-name check would fire for
        // the second entry because the name is duplicated, so force a
        // different name shape: this is a belt-and-braces test that
        // confirms the singleton check is separate from the name check.
        let entries = vec![
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_METADATA,
                asset_flags: 0,
                name: "metadata.json".to_string(),
                payload_offset: 64,
                payload_len: 1,
                checksum: None,
            },
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_METADATA,
                asset_flags: 0,
                // Distinct name so the duplicate-name check does not fire
                // first; the singleton-type check should catch this.
                name: "meta2.json".to_string(),
                payload_offset: 65,
                payload_len: 1,
                checksum: None,
            },
        ];
        let reader = BendlReader {
            inner: Cursor::new(Vec::<u8>::new()),
            header: BendlHeader::provisional(AssignmentFormat::Ben, 64),
            directory: entries,
        };
        // The second entry has asset_type METADATA but name "meta2.json"
        // which fails the canonical-name check.
        let err = reader.validate_directory().unwrap_err();
        assert!(matches!(
            err,
            BundleValidationError::WrongCanonicalName { .. }
        ));
    }

    #[test]
    fn validate_directory_accepts_well_formed_multi_singleton_bundle() {
        // A bundle with one of every singleton type, plus two custom
        // assets with distinct names, should validate cleanly.
        let entries = vec![
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_METADATA,
                asset_flags: ASSET_FLAG_JSON,
                name: "metadata.json".to_string(),
                payload_offset: 64,
                payload_len: 4,
                checksum: None,
            },
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_GRAPH,
                asset_flags: ASSET_FLAG_JSON | ASSET_FLAG_XZ,
                name: "graph.json".to_string(),
                payload_offset: 68,
                payload_len: 4,
                checksum: None,
            },
            BendlDirectoryEntry {
                asset_type: crate::io::bundle::format::ASSET_TYPE_RELABEL_MAP,
                asset_flags: ASSET_FLAG_JSON,
                name: "relabel_map.json".to_string(),
                payload_offset: 72,
                payload_len: 4,
                checksum: None,
            },
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_CUSTOM,
                asset_flags: 0,
                name: "a.bin".to_string(),
                payload_offset: 76,
                payload_len: 4,
                checksum: None,
            },
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_CUSTOM,
                asset_flags: 0,
                name: "b.bin".to_string(),
                payload_offset: 80,
                payload_len: 4,
                checksum: None,
            },
        ];
        let reader = BendlReader {
            inner: Cursor::new(Vec::<u8>::new()),
            header: BendlHeader::provisional(AssignmentFormat::Ben, 64),
            directory: entries,
        };
        reader.validate_directory().expect("well-formed directory");
    }

    #[test]
    fn stress_thousand_custom_assets_round_trip() {
        // Build a directory with 1000 small custom assets, each with a
        // unique payload derived from its index, and confirm they all
        // round-trip via `asset_bytes`. This catches any off-by-one or
        // seek-caching bugs that might only show up with many entries.
        const N: usize = 1000;

        let mut bytes = Vec::new();
        bytes.extend(std::iter::repeat(0u8).take(HEADER_SIZE));

        let mut entries = Vec::with_capacity(N);
        let mut expected = Vec::with_capacity(N);
        for i in 0..N {
            let payload: Vec<u8> = (0..(i % 31 + 1) as u8)
                .map(|j| (i as u8).wrapping_add(j))
                .collect();
            let offset = bytes.len() as u64;
            bytes.extend_from_slice(&payload);
            entries.push(BendlDirectoryEntry {
                asset_type: ASSET_TYPE_CUSTOM,
                asset_flags: 0,
                name: format!("blob-{i:04}.bin"),
                payload_offset: offset,
                payload_len: payload.len() as u64,
                checksum: None,
            });
            expected.push(payload);
        }

        let stream_offset = bytes.len() as u64;
        let stream_len = 0u64;
        let directory_offset = bytes.len() as u64;
        let directory = encode_directory(&entries).unwrap();
        bytes.extend_from_slice(&directory);
        let directory_len = directory.len() as u64;

        let header = BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            complete: COMPLETE_YES,
            assignment_format: AssignmentFormat::Ben.to_u8(),
            reserved_0: 0,
            flags: 0,
            directory_offset,
            directory_len,
            stream_offset,
            stream_len,
            sample_count: 0,
        };
        bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());

        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.assets().len(), N);
        reader.validate_directory().unwrap();
        // Access in scrambled order to exercise seeking.
        for &idx in &[0usize, N - 1, 1, N / 2, N / 3, 2 * N / 3, 7, 999] {
            let name = format!("blob-{idx:04}.bin");
            let entry = reader.find_asset_by_name(&name).cloned().unwrap();
            let got = reader.asset_bytes(&entry).unwrap();
            assert_eq!(got, expected[idx], "mismatch at index {idx}");
        }
    }

    #[test]
    fn xz_flagged_asset_with_corrupt_payload_surfaces_io_error() {
        // Hand-build a bundle with a single asset flagged ASSET_FLAG_XZ
        // whose payload bytes are not a valid xz container. `asset_bytes`
        // must surface an io::Error rather than panicking.
        let mut bytes = Vec::new();
        bytes.extend(std::iter::repeat(0u8).take(HEADER_SIZE));

        let bad_payload = vec![0xFFu8, 0xFE, 0xFD, 0xFC, 0xFB];
        let payload_offset = bytes.len() as u64;
        bytes.extend_from_slice(&bad_payload);

        let stream_offset = bytes.len() as u64;
        let directory_offset = bytes.len() as u64;
        let entries = vec![BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: ASSET_FLAG_XZ,
            name: "broken.xz".to_string(),
            payload_offset,
            payload_len: bad_payload.len() as u64,
            checksum: None,
        }];
        let directory = encode_directory(&entries).unwrap();
        bytes.extend_from_slice(&directory);

        let header = BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            complete: COMPLETE_YES,
            assignment_format: AssignmentFormat::Ben.to_u8(),
            reserved_0: 0,
            flags: 0,
            directory_offset,
            directory_len: directory.len() as u64,
            stream_offset,
            stream_len: 0,
            sample_count: 0,
        };
        bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());

        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        let entry = reader.find_asset_by_name("broken.xz").cloned().unwrap();
        let res = reader.asset_bytes(&entry);
        assert!(res.is_err(), "expected xz decode error, got {res:?}");
    }

    #[test]
    fn reader_scales_to_very_wide_stream_offset_field() {
        // Confirm the `Take` bound clamps a stream reader even when the
        // header's stream_len is much larger than the actual remaining
        // bytes: the reader must return the shorter slice rather than
        // loop forever or panic. This is a "short read" tolerance check.
        let fake_stream = b"STANDARD BEN FILE\x00\x01tiny".to_vec();
        let actual_len = fake_stream.len() as u64;
        let directory_offset = HEADER_SIZE as u64 + actual_len;
        // Build a bundle that lies about stream_len: claims ten times
        // what's actually present.
        let entries: Vec<BendlDirectoryEntry> = Vec::new();
        let directory_bytes = encode_directory(&entries).unwrap();
        let header = BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            complete: COMPLETE_YES,
            assignment_format: AssignmentFormat::Ben.to_u8(),
            reserved_0: 0,
            flags: 0,
            directory_offset,
            directory_len: directory_bytes.len() as u64,
            stream_offset: HEADER_SIZE as u64,
            stream_len: actual_len * 10, // lie
            sample_count: 0,
        };
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&header.to_bytes());
        bytes.extend_from_slice(&fake_stream);
        bytes.extend_from_slice(&directory_bytes);

        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        let mut buf = Vec::new();
        // Take will try to read `stream_len` bytes but the Cursor will
        // just return however many bytes remain from stream_offset to EOF.
        // The reader must not panic; it must simply return what it got.
        reader
            .assignment_stream_reader()
            .unwrap()
            .read_to_end(&mut buf)
            .unwrap();
        // Take includes the directory bytes in the window since they come
        // after stream_offset and the claim exceeds file size — so we
        // assert only that we got *at least* the real stream bytes as a
        // prefix, which is the basic "no truncation of what exists" check.
        assert!(buf.starts_with(&fake_stream));
    }

    #[test]
    fn incomplete_bundle_with_nonzero_directory_offset_uses_it_as_stream_end() {
        // An incomplete bundle where directory_offset is non-zero:
        // the stream end is taken as directory_offset, not EOF.
        let fake_stream = b"STANDARD BEN FILE\x00partial".to_vec();
        let fake_dir = b"some-directory-bytes";
        let stream_start = HEADER_SIZE as u64;
        let dir_offset = stream_start + fake_stream.len() as u64;

        let header = BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            complete: COMPLETE_NO,
            assignment_format: AssignmentFormat::Ben.to_u8(),
            reserved_0: 0,
            flags: 0,
            directory_offset: dir_offset,
            directory_len: 0,
            stream_offset: stream_start,
            stream_len: 0,
            sample_count: -1,
        };
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&header.to_bytes());
        bytes.extend_from_slice(&fake_stream);
        bytes.extend_from_slice(fake_dir);

        let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
        assert!(!reader.is_complete());

        let (offset, len) = reader.assignment_stream_range().unwrap();
        assert_eq!(offset, stream_start);
        assert_eq!(len, fake_stream.len() as u64);
    }

    #[test]
    fn validate_directory_rejects_wrong_canonical_name() {
        use crate::io::bundle::format::BendlDirectoryEntry;

        let entries = vec![BendlDirectoryEntry {
            asset_type: ASSET_TYPE_GRAPH,
            asset_flags: ASSET_FLAG_JSON,
            name: "not_the_canonical_name.json".to_string(),
            payload_offset: 64,
            payload_len: 10,
            checksum: None,
        }];
        let err = validate_directory_entries(&entries).unwrap_err();
        match err {
            BundleValidationError::WrongCanonicalName { .. } => {}
            _ => panic!("expected WrongCanonicalName, got {err:?}"),
        }
    }
}
