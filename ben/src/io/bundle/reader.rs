//! Read-only inspection of `.bendl` files.
//!
//! A [`BendlReader`] parses a bundle's fixed header and (if present) its
//! trailing directory table. It does not read any asset payload bytes
//! until the caller explicitly requests them via [`BendlReader::asset_bytes`]
//! or [`BendlReader::asset_reader`]. The assignment stream region is
//! likewise exposed as a byte range the caller can plumb into a
//! [`BenStreamReader`] without this module reinterpreting any BEN/XBEN
//! internals.

use std::io::{self, Read, Seek, SeekFrom, Take};

use xz2::read::XzDecoder;

use super::format::{
    standardized_name_for, read_directory, AssignmentFormat, BendlDirectoryEntry, BendlFormatError,
    BendlHeader, ASSET_FLAG_XZ,
};
use crate::io::reader::{BenStreamReader, BenWireFormat};

impl From<AssignmentFormat> for BenWireFormat {
    fn from(format: AssignmentFormat) -> Self {
        match format {
            AssignmentFormat::Ben => BenWireFormat::Ben,
            AssignmentFormat::Xben => BenWireFormat::XBen,
        }
    }
}

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
    pub fn is_finalized(&self) -> bool {
        self.header.is_finalized()
    }

    /// The sample count recorded in the header, or `None` if not
    /// authoritative (i.e. the bundle is still incomplete).
    pub fn sample_count(&self) -> Option<i64> {
        if self.header.is_finalized() {
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
    /// `node_permutation_map.json`) use this to grab their payload without caring
    /// about the standardized name.
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
        if self.header.is_finalized() {
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
    /// to wrap the returned reader in a [`BenStreamReader`] (via
    /// [`BendlReader::open_assignment_reader`] or directly) as
    /// appropriate for [`BendlReader::assignment_format`].
    pub fn assignment_stream_reader(&mut self) -> io::Result<Take<&mut R>> {
        let (offset, len) = self.assignment_stream_range()?;
        self.inner.seek(SeekFrom::Start(offset))?;
        Ok((&mut self.inner).take(len))
    }

    /// Construct the appropriate assignment decoder for the bundle's
    /// declared `assignment_format` and return it as a [`BenStreamReader`]
    /// over the bundle's bounded stream region.
    ///
    /// Returns an error if the header's `assignment_format` field is
    /// unrecognized or the embedded banner is malformed.
    pub fn open_assignment_reader(
        &mut self,
    ) -> Result<BenStreamReader<Take<&mut R>>, BundleAssignmentReaderError> {
        let format = self.assignment_format().ok_or(
            BundleAssignmentReaderError::UnknownAssignmentFormat(self.header.assignment_format),
        )?;
        let stream = self.assignment_stream_reader()?;
        match format {
            AssignmentFormat::Ben => {
                BenStreamReader::from_ben(stream).map_err(BundleAssignmentReaderError::Decoder)
            }
            AssignmentFormat::Xben => {
                BenStreamReader::from_xben(stream).map_err(BundleAssignmentReaderError::Decoder)
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
        if let Some(canonical) = standardized_name_for(entry.asset_type) {
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

    /// An entry with a known singleton type is not using its standardized name.
    #[error("asset type {asset_type} must use standardized name {expected:?}, found {found:?}")]
    WrongCanonicalName {
        /// The asset type whose standardized name was violated.
        asset_type: u16,
        /// The standardized name the writer should have used.
        expected: String,
        /// The name that was actually written.
        found: String,
    },
}

