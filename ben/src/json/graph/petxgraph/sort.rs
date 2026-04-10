use super::permutation::apply_permutation;
use super::{PetxGraph, PetxNode};
use petgraph::graph::NodeIndex;
use serde_json::Value;
use std::cmp::Ordering;

/// Sort a `PetxGraph` by a node attribute and apply the permutation in place.
///
/// Nodes are ordered by the value of `key` in their attribute map, using
/// numeric comparison when possible and falling back to string comparison.
///
/// Returns the permutation that was applied.
pub(in crate::json::graph) fn sort_by_key<Ty>(
    petx_graph: &mut PetxGraph<Ty>,
    key: &str,
) -> Vec<NodeIndex>
where
    Ty: petgraph::EdgeType,
{
    let graph = &petx_graph.graph;
    let mut order: Vec<NodeIndex> = graph.node_indices().collect();

    order.sort_by(|&a, &b| {
        let a_val = get_sort_attr(&graph[a], key);
        let b_val = get_sort_attr(&graph[b], key);
        compare_attr_values(a_val, b_val)
    });

    *petx_graph = apply_permutation(petx_graph, &order);
    order
}

/// Look up the sort attribute for a node.
///
/// The special key `"id"` is mapped to the internal `"__networkx_id__"`
/// attribute so callers can sort by the NetworkX node id.
///
/// # Arguments
///
/// * `node` - The node whose attribute is being looked up.
/// * `key` - The attribute name. `"id"` is treated as an alias for
///   `"__networkx_id__"`.
///
/// # Returns
///
/// A reference to the attribute [`Value`], or `None` if the attribute is
/// absent.
fn get_sort_attr<'a>(node: &'a PetxNode, key: &str) -> Option<&'a Value> {
    if key == "id" {
        node.attrs.get("__networkx_id__")
    } else {
        node.attrs.get(key)
    }
}

/// Compare two optional attribute values for sorting.
///
/// Values are compared numerically when both can be interpreted as `u64`.
/// Otherwise they are compared as strings. `None` is treated as the string
/// `"null"`.
///
/// # Arguments
///
/// * `a` - The left-hand attribute value (or `None` if absent).
/// * `b` - The right-hand attribute value (or `None` if absent).
///
/// # Returns
///
/// An [`Ordering`] suitable for use in a sort comparator.
fn compare_attr_values(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    let extract = |val: Option<&Value>| -> Result<u64, String> {
        match val {
            Some(Value::String(s)) => s.parse::<u64>().map_err(|_| s.clone()),
            Some(Value::Number(n)) => n.as_u64().ok_or_else(|| n.to_string()),
            Some(v) => Err(v.to_string()),
            None => Err("null".to_string()),
        }
    };

    match (extract(a), extract(b)) {
        (Ok(a_num), Ok(b_num)) => a_num.cmp(&b_num),
        (Err(a_str), Err(b_str)) => a_str.cmp(&b_str),
        (Err(a_str), Ok(b_num)) => a_str.cmp(&b_num.to_string()),
        (Ok(a_num), Err(b_str)) => a_num.to_string().cmp(&b_str),
    }
}
