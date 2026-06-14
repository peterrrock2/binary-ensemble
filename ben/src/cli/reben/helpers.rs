use super::args::{BenCliVariant, OrderingMethod};
use crate::json::graph::GraphOrderingMethod;
use crate::ops::relabel::{relabel_ben_file, RelabelOptions};
use crate::BenVariant;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read};

pub(super) fn read_node_permutation_map_file(
    map_file_name: &str,
) -> Result<(HashMap<usize, usize>, String), String> {
    let map_file = File::open(map_file_name)
        .map_err(|e| format!("Could not open map file {map_file_name:?}: {e}"))?;
    let map_reader = BufReader::new(map_file);

    let data: Value = serde_json::from_reader(map_reader)
        .map_err(|e| format!("Could not parse map file {map_file_name:?} as JSON: {e}"))?;

    let map_obj = data
        .get("node_permutation_old_to_new")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!(
                "Map file {map_file_name:?} must contain object field \
                 node_permutation_old_to_new"
            )
        })?;

    let mut new_to_old_node_map = HashMap::with_capacity(map_obj.len());
    for (old_idx_text, new_idx_value) in map_obj {
        let old_idx = old_idx_text.parse::<usize>().map_err(|e| {
            format!(
                "Map file {map_file_name:?} contains invalid old node index {old_idx_text:?}: {e}"
            )
        })?;
        let new_idx = new_idx_value.as_u64().ok_or_else(|| {
            format!(
                "Map file {map_file_name:?} maps old node {old_idx} to non-integer value \
                 {new_idx_value}"
            )
        })? as usize;
        new_to_old_node_map.insert(new_idx, old_idx);
    }

    let label = data["key"]
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| data["ordering_method"].as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "map".to_string());

    Ok((new_to_old_node_map, label))
}

/// Convert a CLI ordering method variant to the library's graph ordering type.
pub(super) fn to_graph_ordering(ordering: &OrderingMethod) -> GraphOrderingMethod {
    match ordering {
        OrderingMethod::MultiLevelCluster => GraphOrderingMethod::MultiLevelCluster,
        OrderingMethod::ReverseCuthillMckee => GraphOrderingMethod::ReverseCuthillMckee,
    }
}

/// Return the kebab-case display name for an ordering method.
pub(super) fn ordering_method_name(ordering: &OrderingMethod) -> &'static str {
    match ordering {
        OrderingMethod::MultiLevelCluster => "multi-level-cluster",
        OrderingMethod::ReverseCuthillMckee => "reverse-cuthill-mckee",
    }
}

/// Return the lowercase display name for a BEN variant.
pub(super) fn ben_variant_name(variant: BenVariant) -> &'static str {
    match variant {
        BenVariant::Standard => "standard",
        BenVariant::MkvChain => "mkvchain",
        BenVariant::TwoDelta => "twodelta",
    }
}

/// Convert a CLI BEN variant to the library's `BenVariant` type.
pub(super) fn to_ben_variant(variant: &BenCliVariant) -> BenVariant {
    match variant {
        BenCliVariant::Standard => BenVariant::Standard,
        BenCliVariant::MkvChain => BenVariant::MkvChain,
        BenCliVariant::TwoDelta => BenVariant::TwoDelta,
    }
}

/// Strip a trailing `.jsonl.ben` or `.ben` extension, leaving the bare stem for output naming.
pub(super) fn ben_stem(name: &str) -> &str {
    name.strip_suffix(".jsonl.ben")
        .or_else(|| name.strip_suffix(".ben"))
        .unwrap_or(name)
}

/// Build a unique temp sibling path next to `target` so an atomic rename stays on one filesystem.
/// Uniqueness comes from pid + a wall-clock nonce + a process-local counter, avoiding collisions
/// across concurrent and repeated runs without pulling in a dependency.
fn temp_sibling_path(target: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{target}.reben-tmp-{}-{nonce:016x}-{seq:x}",
        std::process::id()
    )
}

/// Relabel `target_path` in place: stream into a temp sibling, then atomically rename over the
/// original. The original is left untouched on any error before the rename.
pub(super) fn relabel_in_place<R: Read>(
    reader: R,
    target_path: &str,
    options: RelabelOptions,
) -> Result<(), String> {
    let tmp_path = temp_sibling_path(target_path);
    let tmp_file = File::create(&tmp_path)
        .map_err(|e| format!("Could not create temp file {tmp_path:?}: {e}"))?;
    let mut writer = BufWriter::new(tmp_file);

    // `relabel_ben_file` owns and drops `reader` before returning, so the input handle is closed by
    // the time we rename over it (matters on Windows). `&mut writer` keeps the file ours to sync.
    let result = relabel_ben_file(reader, &mut writer, options).and_then(|()| {
        let file = writer.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()
    });

    match result {
        Ok(()) => fs::rename(&tmp_path, target_path).map_err(|e| {
            let _ = fs::remove_file(&tmp_path);
            format!("Could not replace {target_path:?} with relabeled output: {e}")
        }),
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            Err(format!("BEN relabeling failed: {e}"))
        }
    }
}

/// Derive a human-readable label from the key or ordering method for file naming.
pub(super) fn relabeling_label(
    key: Option<&str>,
    ordering: Option<&OrderingMethod>,
) -> Result<String, String> {
    match (key, ordering) {
        (Some(_), Some(_)) => Err("Provide either --key or --ordering, not both.".to_string()),
        (Some(key), None) => Ok(key.to_string()),
        (None, Some(ordering)) => Ok(ordering_method_name(ordering).to_string()),
        (None, None) => Err("Provide either --key or --ordering.".to_string()),
    }
}
