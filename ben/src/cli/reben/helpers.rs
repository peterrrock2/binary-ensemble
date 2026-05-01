use super::args::{BenCliVariant, OrderingMethod};
use crate::json::graph::GraphOrderingMethod;
use crate::BenVariant;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;

pub(super) fn read_relabel_map_file(
    map_file_name: &str,
) -> Result<(HashMap<usize, usize>, String), String> {
    let map_file = File::open(map_file_name)
        .map_err(|e| format!("Could not open map file {map_file_name:?}: {e}"))?;
    let map_reader = BufReader::new(map_file);

    let data: Value = serde_json::from_reader(map_reader)
        .map_err(|e| format!("Could not parse map file {map_file_name:?} as JSON: {e}"))?;

    let map_obj = data
        .get("relabeling_old_to_new_nodes_map")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!(
                "Map file {map_file_name:?} must contain object field \
                 relabeling_old_to_new_nodes_map"
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
