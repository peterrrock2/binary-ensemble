//! JSON metadata structs for the optional `metadata.json` asset.
//!
//! The authoritative values for `major_version`, `minor_version`, `assignment_format`, `complete`,
//! and the BEN variant all live in the fixed bundle header (or in the embedded stream banner for
//! the variant). The `metadata.json` asset is a best-effort human-readable mirror intended for
//! debugging and tooling; writers should prefer reading the header directly rather than trusting
//! fields in this struct.

use serde::{Deserialize, Serialize};

/// Serde representation of the optional `metadata.json` asset.
///
/// Field names mirror the header where possible so that the JSON is easy to cross-reference against
/// the binary layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BendlManifest {
    /// Incompatible-change version of the bundle format.
    pub major_version: u16,
    /// Additive version of the bundle format.
    pub minor_version: u16,
    /// Container format of the embedded assignment stream (`"ben"` or `"xben"`).
    pub assignment_format: String,
    /// BEN variant (`"standard"`, `"mkv_chain"`, or `"two_delta"`) as carried by the embedded
    /// stream's 17-byte banner.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub variant: Option<String>,
    /// Whether the bundle was finalized successfully.
    pub complete: bool,
}
