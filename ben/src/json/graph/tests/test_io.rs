use super::super::nx_formats::{NxAdjEntry, NxGraphAdjFormat, NxNode};
use super::super::petxgraph::*;
use petgraph::graph::{DiGraph, NodeIndex, UnGraph};
use petgraph::visit::EdgeRef;
use serde_json::json;
use std::collections::BTreeMap;

fn parse_nx(s: &str) -> NxGraphAdjFormat {
    serde_json::from_str(s).unwrap()
}

/// Collect edges as a sorted set of (source, target) pairs, canonicalized for undirected.
fn edge_set_undirected(graph: &PetxUnInnerGraph) -> Vec<(usize, usize)> {
    let mut edges: Vec<(usize, usize)> = graph
        .edge_references()
        .map(|e| {
            let (a, b) = (e.source().index(), e.target().index());
            if a <= b {
                (a, b)
            } else {
                (b, a)
            }
        })
        .collect();
    edges.sort();
    edges
}

/// Collect edges as a sorted set of (source, target) pairs for directed graphs.
fn edge_set_directed(graph: &PetxDiInnerGraph) -> Vec<(usize, usize)> {
    let mut edges: Vec<(usize, usize)> = graph
        .edge_references()
        .map(|e| (e.source().index(), e.target().index()))
        .collect();
    edges.sort();
    edges
}

/// Normalize an NxGraphAdjFormat by sorting each adjacency list by target id, so structural
/// equality can be checked after roundtrip.
fn normalize(format: &mut NxGraphAdjFormat) {
    for adj_list in &mut format.adjacency {
        adj_list.sort_by(|a, b| {
            let ak = serde_json::to_string(&a.id).unwrap();
            let bk = serde_json::to_string(&b.id).unwrap();
            ak.cmp(&bk)
        });
    }
}

// ================================================================
// == Fixtures (generated with `uv run --with networkx python3`) ==
// ================================================================

const KARATE_JSON: &str = r#"{"directed": false, "multigraph": false, "graph": [["name", "Zachary's Karate Club"]], "nodes": [{"club": "Mr. Hi", "id": 0}, {"club": "Mr. Hi", "id": 1}, {"club": "Mr. Hi", "id": 2}, {"club": "Mr. Hi", "id": 3}, {"club": "Mr. Hi", "id": 4}, {"club": "Mr. Hi", "id": 5}, {"club": "Mr. Hi", "id": 6}, {"club": "Mr. Hi", "id": 7}, {"club": "Mr. Hi", "id": 8}, {"club": "Officer", "id": 9}, {"club": "Mr. Hi", "id": 10}, {"club": "Mr. Hi", "id": 11}, {"club": "Mr. Hi", "id": 12}, {"club": "Mr. Hi", "id": 13}, {"club": "Officer", "id": 14}, {"club": "Officer", "id": 15}, {"club": "Mr. Hi", "id": 16}, {"club": "Mr. Hi", "id": 17}, {"club": "Officer", "id": 18}, {"club": "Mr. Hi", "id": 19}, {"club": "Officer", "id": 20}, {"club": "Mr. Hi", "id": 21}, {"club": "Officer", "id": 22}, {"club": "Officer", "id": 23}, {"club": "Officer", "id": 24}, {"club": "Officer", "id": 25}, {"club": "Officer", "id": 26}, {"club": "Officer", "id": 27}, {"club": "Officer", "id": 28}, {"club": "Officer", "id": 29}, {"club": "Officer", "id": 30}, {"club": "Officer", "id": 31}, {"club": "Officer", "id": 32}, {"club": "Officer", "id": 33}], "adjacency": [[{"weight": 4, "id": 1}, {"weight": 5, "id": 2}, {"weight": 3, "id": 3}, {"weight": 3, "id": 4}, {"weight": 3, "id": 5}, {"weight": 3, "id": 6}, {"weight": 2, "id": 7}, {"weight": 2, "id": 8}, {"weight": 2, "id": 10}, {"weight": 3, "id": 11}, {"weight": 1, "id": 12}, {"weight": 3, "id": 13}, {"weight": 2, "id": 17}, {"weight": 2, "id": 19}, {"weight": 2, "id": 21}, {"weight": 2, "id": 31}], [{"weight": 4, "id": 0}, {"weight": 6, "id": 2}, {"weight": 3, "id": 3}, {"weight": 4, "id": 7}, {"weight": 5, "id": 13}, {"weight": 1, "id": 17}, {"weight": 2, "id": 19}, {"weight": 2, "id": 21}, {"weight": 2, "id": 30}], [{"weight": 5, "id": 0}, {"weight": 6, "id": 1}, {"weight": 3, "id": 3}, {"weight": 4, "id": 7}, {"weight": 5, "id": 8}, {"weight": 1, "id": 9}, {"weight": 3, "id": 13}, {"weight": 2, "id": 27}, {"weight": 2, "id": 28}, {"weight": 2, "id": 32}], [{"weight": 3, "id": 0}, {"weight": 3, "id": 1}, {"weight": 3, "id": 2}, {"weight": 3, "id": 7}, {"weight": 3, "id": 12}, {"weight": 3, "id": 13}], [{"weight": 3, "id": 0}, {"weight": 2, "id": 6}, {"weight": 3, "id": 10}], [{"weight": 3, "id": 0}, {"weight": 5, "id": 6}, {"weight": 3, "id": 10}, {"weight": 3, "id": 16}], [{"weight": 3, "id": 0}, {"weight": 2, "id": 4}, {"weight": 5, "id": 5}, {"weight": 3, "id": 16}], [{"weight": 2, "id": 0}, {"weight": 4, "id": 1}, {"weight": 4, "id": 2}, {"weight": 3, "id": 3}], [{"weight": 2, "id": 0}, {"weight": 5, "id": 2}, {"weight": 3, "id": 30}, {"weight": 3, "id": 32}, {"weight": 4, "id": 33}], [{"weight": 1, "id": 2}, {"weight": 2, "id": 33}], [{"weight": 2, "id": 0}, {"weight": 3, "id": 4}, {"weight": 3, "id": 5}], [{"weight": 3, "id": 0}], [{"weight": 1, "id": 0}, {"weight": 3, "id": 3}], [{"weight": 3, "id": 0}, {"weight": 5, "id": 1}, {"weight": 3, "id": 2}, {"weight": 3, "id": 3}, {"weight": 3, "id": 33}], [{"weight": 3, "id": 32}, {"weight": 2, "id": 33}], [{"weight": 3, "id": 32}, {"weight": 4, "id": 33}], [{"weight": 3, "id": 5}, {"weight": 3, "id": 6}], [{"weight": 2, "id": 0}, {"weight": 1, "id": 1}], [{"weight": 1, "id": 32}, {"weight": 2, "id": 33}], [{"weight": 2, "id": 0}, {"weight": 2, "id": 1}, {"weight": 1, "id": 33}], [{"weight": 3, "id": 32}, {"weight": 1, "id": 33}], [{"weight": 2, "id": 0}, {"weight": 2, "id": 1}], [{"weight": 2, "id": 32}, {"weight": 3, "id": 33}], [{"weight": 5, "id": 25}, {"weight": 4, "id": 27}, {"weight": 3, "id": 29}, {"weight": 5, "id": 32}, {"weight": 4, "id": 33}], [{"weight": 2, "id": 25}, {"weight": 3, "id": 27}, {"weight": 2, "id": 31}], [{"weight": 5, "id": 23}, {"weight": 2, "id": 24}, {"weight": 7, "id": 31}], [{"weight": 4, "id": 29}, {"weight": 2, "id": 33}], [{"weight": 2, "id": 2}, {"weight": 4, "id": 23}, {"weight": 3, "id": 24}, {"weight": 4, "id": 33}], [{"weight": 2, "id": 2}, {"weight": 2, "id": 31}, {"weight": 2, "id": 33}], [{"weight": 3, "id": 23}, {"weight": 4, "id": 26}, {"weight": 4, "id": 32}, {"weight": 2, "id": 33}], [{"weight": 2, "id": 1}, {"weight": 3, "id": 8}, {"weight": 3, "id": 32}, {"weight": 3, "id": 33}], [{"weight": 2, "id": 0}, {"weight": 2, "id": 24}, {"weight": 7, "id": 25}, {"weight": 2, "id": 28}, {"weight": 4, "id": 32}, {"weight": 4, "id": 33}], [{"weight": 2, "id": 2}, {"weight": 3, "id": 8}, {"weight": 3, "id": 14}, {"weight": 3, "id": 15}, {"weight": 1, "id": 18}, {"weight": 3, "id": 20}, {"weight": 2, "id": 22}, {"weight": 5, "id": 23}, {"weight": 4, "id": 29}, {"weight": 3, "id": 30}, {"weight": 4, "id": 31}, {"weight": 5, "id": 33}], [{"weight": 4, "id": 8}, {"weight": 2, "id": 9}, {"weight": 3, "id": 13}, {"weight": 2, "id": 14}, {"weight": 4, "id": 15}, {"weight": 2, "id": 18}, {"weight": 1, "id": 19}, {"weight": 1, "id": 20}, {"weight": 4, "id": 23}, {"weight": 2, "id": 26}, {"weight": 4, "id": 27}, {"weight": 2, "id": 28}, {"weight": 2, "id": 29}, {"weight": 3, "id": 30}, {"weight": 4, "id": 31}, {"weight": 5, "id": 32}, {"weight": 3, "id": 22}]]}"#;

const SMALL_DIRECTED_JSON: &str = r#"{"directed": true, "multigraph": false, "graph": [], "nodes": [{"id": 0}, {"id": 1}, {"id": 2}, {"id": 3}], "adjacency": [[{"id": 1}, {"id": 2}], [{"id": 2}], [{"id": 3}], [{"id": 0}]]}"#;

const K5_JSON: &str = r#"{"directed": false, "multigraph": false, "graph": [], "nodes": [{"id": 0}, {"id": 1}, {"id": 2}, {"id": 3}, {"id": 4}], "adjacency": [[{"id": 1}, {"id": 2}, {"id": 3}, {"id": 4}], [{"id": 0}, {"id": 2}, {"id": 3}, {"id": 4}], [{"id": 0}, {"id": 1}, {"id": 3}, {"id": 4}], [{"id": 0}, {"id": 1}, {"id": 2}, {"id": 4}], [{"id": 0}, {"id": 1}, {"id": 2}, {"id": 3}]]}"#;

const P4_JSON: &str = r#"{"directed": false, "multigraph": false, "graph": [], "nodes": [{"id": 0}, {"id": 1}, {"id": 2}, {"id": 3}], "adjacency": [[{"id": 1}], [{"id": 0}, {"id": 2}], [{"id": 1}, {"id": 3}], [{"id": 2}]]}"#;

const SINGLE_NODE_JSON: &str = r#"{"directed": false, "multigraph": false, "graph": [], "nodes": [{"label": "solo", "id": 0}], "adjacency": [[]]}"#;

const TWO_TRIANGLES_JSON: &str = r#"{"directed": false, "multigraph": false, "graph": [], "nodes": [{"id": 0}, {"id": 1}, {"id": 2}, {"id": 3}, {"id": 4}, {"id": 5}], "adjacency": [[{"id": 1}, {"id": 2}], [{"id": 0}, {"id": 2}], [{"id": 1}, {"id": 0}], [{"id": 4}, {"id": 5}], [{"id": 3}, {"id": 5}], [{"id": 4}, {"id": 3}]]}"#;

const STRING_IDS_JSON: &str = r#"{"directed": false, "multigraph": false, "graph": [], "nodes": [{"weight": 1.0, "id": "alpha"}, {"weight": 2.0, "id": "beta"}, {"weight": 3.0, "id": "gamma"}], "adjacency": [[{"color": "red", "id": "beta"}], [{"color": "red", "id": "alpha"}, {"color": "blue", "id": "gamma"}], [{"color": "blue", "id": "beta"}]]}"#;

const DIRECTED_CYCLE_JSON: &str = r#"{"directed": true, "multigraph": false, "graph": [], "nodes": [{"id": 0}, {"id": 1}, {"id": 2}, {"id": 3}, {"id": 4}], "adjacency": [[{"id": 1}], [{"id": 2}], [{"id": 3}], [{"id": 4}], [{"id": 0}]]}"#;

const SELF_LOOP_JSON: &str = r#"{"directed": false, "multigraph": false, "graph": [], "nodes": [{"id": 0}, {"id": 1}], "adjacency": [[{"id": 1}, {"id": 0}], [{"id": 0}]]}"#;

const EMPTY_EDGES_JSON: &str = r#"{"directed": false, "multigraph": false, "graph": [], "nodes": [{"id": 0}, {"id": 1}, {"id": 2}], "adjacency": [[], [], []]}"#;

// =============================
// == Karate club graph tests ==
// =============================

#[test]
fn karate_club_node_and_edge_counts() {
    let nx = parse_nx(KARATE_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();
    assert_eq!(petx.graph.node_count(), 34);
    assert_eq!(petx.graph.edge_count(), 78);
}

#[test]
fn karate_club_graph_attrs_preserved() {
    let nx = parse_nx(KARATE_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();
    assert_eq!(petx.graph_attrs.len(), 1);
    assert_eq!(petx.graph_attrs[0].0, "name");
    assert_eq!(petx.graph_attrs[0].1, json!("Zachary's Karate Club"));
}

#[test]
fn karate_club_node_attrs_preserved() {
    let nx = parse_nx(KARATE_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();

    // Node 0 should have club="Mr. Hi"
    let node0 = petx.graph.node_weight(NodeIndex::new(0)).unwrap();
    assert_eq!(node0.attrs.get("club"), Some(&json!("Mr. Hi")));
    assert_eq!(node0.attrs.get("__networkx_id__"), Some(&json!(0)));

    // Node 33 should have club="Officer"
    let node33 = petx.graph.node_weight(NodeIndex::new(33)).unwrap();
    assert_eq!(node33.attrs.get("club"), Some(&json!("Officer")));
}

#[test]
fn karate_club_roundtrip() {
    let nx_original = parse_nx(KARATE_JSON);
    let petx: PetxUnGraph = nx_original.clone().try_into().unwrap();
    let mut nx_roundtrip = NxGraphAdjFormat::try_from(&petx).unwrap();
    let mut nx_expected = nx_original;

    normalize(&mut nx_expected);
    normalize(&mut nx_roundtrip);

    assert_eq!(nx_roundtrip.directed, nx_expected.directed);
    assert_eq!(nx_roundtrip.multigraph, nx_expected.multigraph);
    assert_eq!(nx_roundtrip.graph, nx_expected.graph);
    assert_eq!(nx_roundtrip.nodes.len(), nx_expected.nodes.len());

    for (orig, rt) in nx_expected.nodes.iter().zip(nx_roundtrip.nodes.iter()) {
        assert_eq!(orig.id, rt.id);
        assert_eq!(orig.attrs, rt.attrs);
    }

    for (i, (orig_adj, rt_adj)) in nx_expected
        .adjacency
        .iter()
        .zip(nx_roundtrip.adjacency.iter())
        .enumerate()
    {
        assert_eq!(
            orig_adj.len(),
            rt_adj.len(),
            "adjacency list length mismatch at node {}",
            i
        );
    }
}

// =======================
// == Complete graph K5 ==
// =======================

#[test]
fn k5_node_and_edge_counts() {
    let nx = parse_nx(K5_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();
    assert_eq!(petx.graph.node_count(), 5);
    // K5 has C(5,2) = 10 edges
    assert_eq!(petx.graph.edge_count(), 10);
}

#[test]
fn k5_all_pairs_connected() {
    let nx = parse_nx(K5_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();
    let edges = edge_set_undirected(&petx.graph);

    for i in 0..5 {
        for j in (i + 1)..5 {
            assert!(edges.contains(&(i, j)), "K5 missing edge ({}, {})", i, j);
        }
    }
}

#[test]
fn k5_roundtrip() {
    let nx_original = parse_nx(K5_JSON);
    let petx: PetxUnGraph = nx_original.clone().try_into().unwrap();
    let mut nx_roundtrip = NxGraphAdjFormat::try_from(&petx).unwrap();
    let mut nx_expected = nx_original;
    normalize(&mut nx_expected);
    normalize(&mut nx_roundtrip);
    assert_eq!(nx_roundtrip, nx_expected);
}

// ===================
// == Path graph P4 ==
// ===================

#[test]
fn p4_structure() {
    let nx = parse_nx(P4_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();
    assert_eq!(petx.graph.node_count(), 4);
    assert_eq!(petx.graph.edge_count(), 3);

    let edges = edge_set_undirected(&petx.graph);
    assert_eq!(edges, vec![(0, 1), (1, 2), (2, 3)]);
}

#[test]
fn p4_roundtrip() {
    let nx_original = parse_nx(P4_JSON);
    let petx: PetxUnGraph = nx_original.clone().try_into().unwrap();
    let mut nx_roundtrip = NxGraphAdjFormat::try_from(&petx).unwrap();
    let mut nx_expected = nx_original;
    normalize(&mut nx_expected);
    normalize(&mut nx_roundtrip);
    assert_eq!(nx_roundtrip, nx_expected);
}

// =====================
// == Directed graphs ==
// =====================

#[test]
fn small_directed_structure() {
    let nx = parse_nx(SMALL_DIRECTED_JSON);
    let petx: PetxDiGraph = nx.try_into().unwrap();
    assert_eq!(petx.graph.node_count(), 4);
    assert_eq!(petx.graph.edge_count(), 5);

    let edges = edge_set_directed(&petx.graph);
    assert_eq!(edges, vec![(0, 1), (0, 2), (1, 2), (2, 3), (3, 0)]);
}

#[test]
fn small_directed_roundtrip() {
    let nx_original = parse_nx(SMALL_DIRECTED_JSON);
    let petx: PetxDiGraph = nx_original.clone().try_into().unwrap();
    let mut nx_roundtrip = NxGraphAdjFormat::try_from(&petx).unwrap();
    let mut nx_expected = nx_original;
    normalize(&mut nx_expected);
    normalize(&mut nx_roundtrip);
    assert_eq!(nx_roundtrip, nx_expected);
}

#[test]
fn directed_cycle_structure() {
    let nx = parse_nx(DIRECTED_CYCLE_JSON);
    let petx: PetxDiGraph = nx.try_into().unwrap();
    assert_eq!(petx.graph.node_count(), 5);
    assert_eq!(petx.graph.edge_count(), 5);

    let edges = edge_set_directed(&petx.graph);
    assert_eq!(edges, vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]);
}

#[test]
fn directed_cycle_roundtrip() {
    let nx_original = parse_nx(DIRECTED_CYCLE_JSON);
    let petx: PetxDiGraph = nx_original.clone().try_into().unwrap();
    let mut nx_roundtrip = NxGraphAdjFormat::try_from(&petx).unwrap();
    let mut nx_expected = nx_original;
    normalize(&mut nx_expected);
    normalize(&mut nx_roundtrip);
    assert_eq!(nx_roundtrip, nx_expected);
}

// ================
// == Edge cases ==
// ================

#[test]
fn single_node_no_edges() {
    let nx = parse_nx(SINGLE_NODE_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();
    assert_eq!(petx.graph.node_count(), 1);
    assert_eq!(petx.graph.edge_count(), 0);

    let node = petx.graph.node_weight(NodeIndex::new(0)).unwrap();
    assert_eq!(node.attrs.get("label"), Some(&json!("solo")));
}

#[test]
fn single_node_roundtrip() {
    let nx_original = parse_nx(SINGLE_NODE_JSON);
    let petx: PetxUnGraph = nx_original.clone().try_into().unwrap();
    let nx_roundtrip = NxGraphAdjFormat::try_from(&petx).unwrap();
    assert_eq!(nx_roundtrip, nx_original);
}

#[test]
fn empty_edges_graph() {
    let nx = parse_nx(EMPTY_EDGES_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();
    assert_eq!(petx.graph.node_count(), 3);
    assert_eq!(petx.graph.edge_count(), 0);
}

#[test]
fn empty_edges_roundtrip() {
    let nx_original = parse_nx(EMPTY_EDGES_JSON);
    let petx: PetxUnGraph = nx_original.clone().try_into().unwrap();
    let nx_roundtrip = NxGraphAdjFormat::try_from(&petx).unwrap();
    assert_eq!(nx_roundtrip, nx_original);
}

#[test]
fn self_loop_preserved() {
    let nx = parse_nx(SELF_LOOP_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();
    assert_eq!(petx.graph.node_count(), 2);
    // self-loop (0,0) + edge (0,1)
    assert_eq!(petx.graph.edge_count(), 2);

    let edges = edge_set_undirected(&petx.graph);
    assert!(edges.contains(&(0, 0)), "self-loop should be preserved");
    assert!(edges.contains(&(0, 1)));
}

#[test]
fn disconnected_graph_two_triangles() {
    let nx = parse_nx(TWO_TRIANGLES_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();
    assert_eq!(petx.graph.node_count(), 6);
    assert_eq!(petx.graph.edge_count(), 6);

    let edges = edge_set_undirected(&petx.graph);
    // Triangle 1: {0,1,2}
    assert!(edges.contains(&(0, 1)));
    assert!(edges.contains(&(0, 2)));
    assert!(edges.contains(&(1, 2)));
    // Triangle 2: {3,4,5}
    assert!(edges.contains(&(3, 4)));
    assert!(edges.contains(&(3, 5)));
    assert!(edges.contains(&(4, 5)));
}

#[test]
fn two_triangles_roundtrip() {
    let nx_original = parse_nx(TWO_TRIANGLES_JSON);
    let petx: PetxUnGraph = nx_original.clone().try_into().unwrap();
    let mut nx_roundtrip = NxGraphAdjFormat::try_from(&petx).unwrap();
    let mut nx_expected = nx_original;
    normalize(&mut nx_expected);
    normalize(&mut nx_roundtrip);
    assert_eq!(nx_roundtrip, nx_expected);
}

// =========================================
// == String node IDs and edge attributes ==
// =========================================

#[test]
fn string_ids_structure() {
    let nx = parse_nx(STRING_IDS_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();
    assert_eq!(petx.graph.node_count(), 3);
    assert_eq!(petx.graph.edge_count(), 2);

    // Verify node IDs are stored as strings
    let node0 = petx.graph.node_weight(NodeIndex::new(0)).unwrap();
    assert_eq!(node0.attrs.get("__networkx_id__"), Some(&json!("alpha")));
    assert_eq!(node0.attrs.get("weight"), Some(&json!(1.0)));
}

#[test]
fn string_ids_edge_attrs_preserved() {
    let nx = parse_nx(STRING_IDS_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();

    // Find the edge between alpha (0) and beta (1)
    let edge = petx.graph.edges(NodeIndex::new(0)).next().unwrap();
    let weight = edge.weight();
    assert_eq!(weight.attrs.get("color"), Some(&json!("red")));
}

#[test]
fn string_ids_roundtrip() {
    let nx_original = parse_nx(STRING_IDS_JSON);
    let petx: PetxUnGraph = nx_original.clone().try_into().unwrap();
    let mut nx_roundtrip = NxGraphAdjFormat::try_from(&petx).unwrap();
    let mut nx_expected = nx_original;
    normalize(&mut nx_expected);
    normalize(&mut nx_roundtrip);
    assert_eq!(nx_roundtrip.directed, nx_expected.directed);
    assert_eq!(nx_roundtrip.nodes.len(), nx_expected.nodes.len());
    for (orig, rt) in nx_expected.nodes.iter().zip(nx_roundtrip.nodes.iter()) {
        assert_eq!(orig.id, rt.id);
        assert_eq!(orig.attrs, rt.attrs);
    }
}

// ==============================
// == graph_has_parallel_edges ==
// ==============================

#[test]
fn no_parallel_edges_simple_graph() {
    let nx = parse_nx(K5_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();
    assert!(!graph_has_parallel_edges(&petx.graph));
}

#[test]
fn no_parallel_edges_directed() {
    let nx = parse_nx(SMALL_DIRECTED_JSON);
    let petx: PetxDiGraph = nx.try_into().unwrap();
    assert!(!graph_has_parallel_edges(&petx.graph));
}

#[test]
fn parallel_edges_detected_undirected() {
    // Manually build a graph with parallel edges
    let mut graph = UnGraph::<PetxNode, NxAdjEntry>::new_undirected();
    let a = graph.add_node(PetxNode {
        attrs: BTreeMap::from([("__networkx_id__".into(), json!(0))]),
    });
    let b = graph.add_node(PetxNode {
        attrs: BTreeMap::from([("__networkx_id__".into(), json!(1))]),
    });
    let edge = NxAdjEntry {
        id: json!(0),
        key: None,
        attrs: BTreeMap::new(),
    };
    graph.add_edge(a, b, edge.clone());
    graph.add_edge(a, b, edge);
    assert!(graph_has_parallel_edges(&graph));
}

#[test]
fn parallel_edges_detected_directed() {
    let mut graph = DiGraph::<PetxNode, NxAdjEntry>::new();
    let a = graph.add_node(PetxNode {
        attrs: BTreeMap::from([("__networkx_id__".into(), json!(0))]),
    });
    let b = graph.add_node(PetxNode {
        attrs: BTreeMap::from([("__networkx_id__".into(), json!(1))]),
    });
    let edge = NxAdjEntry {
        id: json!(0),
        key: None,
        attrs: BTreeMap::new(),
    };
    graph.add_edge(a, b, edge.clone());
    graph.add_edge(a, b, edge);
    assert!(graph_has_parallel_edges(&graph));
}

#[test]
fn antiparallel_not_parallel_in_directed() {
    // In directed graphs, (a->b) and (b->a) are NOT parallel
    let mut graph = DiGraph::<PetxNode, NxAdjEntry>::new();
    let a = graph.add_node(PetxNode {
        attrs: BTreeMap::from([("__networkx_id__".into(), json!(0))]),
    });
    let b = graph.add_node(PetxNode {
        attrs: BTreeMap::from([("__networkx_id__".into(), json!(1))]),
    });
    let edge = NxAdjEntry {
        id: json!(0),
        key: None,
        attrs: BTreeMap::new(),
    };
    graph.add_edge(a, b, edge.clone());
    graph.add_edge(b, a, edge);
    assert!(!graph_has_parallel_edges(&graph));
}

// ======================================
// == nx_node <-> petx_node conversion ==
// ======================================

#[test]
fn nx_to_petx_node_stores_id_in_attrs() {
    let nx_node = NxNode {
        id: json!(42),
        attrs: BTreeMap::from([("color".into(), json!("blue"))]),
    };
    let petx = nx_node_to_petx_node(nx_node);
    assert_eq!(petx.attrs.get("__networkx_id__"), Some(&json!(42)));
    assert_eq!(petx.attrs.get("color"), Some(&json!("blue")));
}

#[test]
fn petx_to_nx_node_restores_id() {
    let petx = PetxNode {
        attrs: BTreeMap::from([
            ("__networkx_id__".into(), json!("node_a")),
            ("weight".into(), json!(3.14)),
        ]),
    };
    let nx = petx_node_to_nx_node(&petx).unwrap();
    assert_eq!(nx.id, json!("node_a"));
    assert_eq!(nx.attrs.get("weight"), Some(&json!(3.14)));
    assert!(!nx.attrs.contains_key("__networkx_id__"));
}

#[test]
fn petx_to_nx_node_missing_id_errors() {
    let petx = PetxNode {
        attrs: BTreeMap::from([("color".into(), json!("red"))]),
    };
    let result = petx_node_to_nx_node(&petx);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("__networkx_id__"),
        "error should mention missing id field: {}",
        err
    );
}

// =================
// == Error cases ==
// =================

#[test]
fn directedness_mismatch_undirected_to_directed() {
    let nx = parse_nx(K5_JSON); // undirected
    let result = PetxDiGraph::try_from(nx);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("directedness mismatch"),
        "unexpected error: {}",
        err
    );
}

#[test]
fn directedness_mismatch_directed_to_undirected() {
    let nx = parse_nx(SMALL_DIRECTED_JSON); // directed
    let result = PetxUnGraph::try_from(nx);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("directedness mismatch"),
        "unexpected error: {}",
        err
    );
}

#[test]
fn duplicate_node_id_error() {
    let nx = NxGraphAdjFormat {
        directed: false,
        multigraph: false,
        graph: vec![],
        nodes: vec![
            NxNode {
                id: json!(0),
                attrs: BTreeMap::new(),
            },
            NxNode {
                id: json!(0),
                attrs: BTreeMap::new(),
            },
        ],
        adjacency: vec![vec![], vec![]],
    };
    let result = PetxUnGraph::try_from(nx);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("duplicate node id"),
        "unexpected error: {}",
        err
    );
}

#[test]
fn node_adjacency_length_mismatch_error() {
    let nx = NxGraphAdjFormat {
        directed: false,
        multigraph: false,
        graph: vec![],
        nodes: vec![NxNode {
            id: json!(0),
            attrs: BTreeMap::new(),
        }],
        adjacency: vec![vec![], vec![]], // 1 node but 2 adjacency lists
    };
    let result = PetxUnGraph::try_from(nx);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("length mismatch"),
        "unexpected error: {}",
        err
    );
}

#[test]
fn missing_neighbor_node_error() {
    let nx = NxGraphAdjFormat {
        directed: false,
        multigraph: false,
        graph: vec![],
        nodes: vec![NxNode {
            id: json!(0),
            attrs: BTreeMap::new(),
        }],
        adjacency: vec![vec![NxAdjEntry {
            id: json!(999), // doesn't exist
            key: None,
            attrs: BTreeMap::new(),
        }]],
    };
    let result = PetxUnGraph::try_from(nx);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("unknown node id"),
        "unexpected error: {}",
        err
    );
}

// ============================================================
// == Type alias smoke tests (ensures they compile and work) ==
// ============================================================

#[test]
fn type_aliases_work() {
    let nx_un = parse_nx(P4_JSON);
    let _petx_un: PetxUnGraph = nx_un.try_into().unwrap();

    let nx_di = parse_nx(SMALL_DIRECTED_JSON);
    let _petx_di: PetxDiGraph = nx_di.try_into().unwrap();

    // Verify inner graph types match
    let _inner_un: &PetxUnInnerGraph = &_petx_un.graph;
    let _inner_di: &PetxDiInnerGraph = &_petx_di.graph;
}

// ===================================
// == Undirected edge deduplication ==
// ===================================

#[test]
fn undirected_dedup_produces_correct_edge_count() {
    // NetworkX adjacency format lists each undirected edge twice: once from each endpoint. The
    // converter should deduplicate to a single petgraph edge.
    let nx = parse_nx(P4_JSON);
    // P4 adjacency has 6 total entries (1+2+2+1) but only 3 unique edges
    let total_adj_entries: usize = nx.adjacency.iter().map(|a| a.len()).sum();
    assert_eq!(total_adj_entries, 6);

    let petx: PetxUnGraph = nx.try_into().unwrap();
    assert_eq!(petx.graph.edge_count(), 3);
}

#[test]
fn construct_nx_from_petx_restores_both_directions() {
    // When converting back, each undirected edge should appear in both endpoints' adjacency lists.
    let nx_original = parse_nx(P4_JSON);
    let petx: PetxUnGraph = nx_original.try_into().unwrap();
    let nx_roundtrip = NxGraphAdjFormat::try_from(&petx).unwrap();

    let total_adj_entries: usize = nx_roundtrip.adjacency.iter().map(|a| a.len()).sum();
    assert_eq!(total_adj_entries, 6);

    // Node 0 should have neighbor 1 (id field is the target node's id)
    assert_eq!(nx_roundtrip.adjacency[0].len(), 1);
    assert_eq!(nx_roundtrip.adjacency[0][0].id, json!(1));
    // Node 1 should have neighbors 0 and 2
    assert_eq!(nx_roundtrip.adjacency[1].len(), 2);
}

// ============================================
// == multigraph flag detection on roundtrip ==
// ============================================

#[test]
fn simple_graph_roundtrip_multigraph_false() {
    let nx = parse_nx(K5_JSON);
    let petx: PetxUnGraph = nx.try_into().unwrap();
    let nx_rt = NxGraphAdjFormat::try_from(&petx).unwrap();
    assert!(!nx_rt.multigraph);
}

#[test]
fn graph_with_parallel_edges_sets_multigraph_true() {
    // Build a PetxGraph with parallel edges, convert to NxGraphAdjFormat
    let mut inner = UnGraph::<PetxNode, NxAdjEntry>::new_undirected();
    let a = inner.add_node(PetxNode {
        attrs: BTreeMap::from([("__networkx_id__".into(), json!(0))]),
    });
    let b = inner.add_node(PetxNode {
        attrs: BTreeMap::from([("__networkx_id__".into(), json!(1))]),
    });
    let edge = NxAdjEntry {
        id: json!(0),
        key: None,
        attrs: BTreeMap::new(),
    };
    inner.add_edge(a, b, edge.clone());
    inner.add_edge(a, b, edge);

    let petx = PetxUnGraph {
        graph_attrs: vec![],
        graph: inner,
    };
    let nx = NxGraphAdjFormat::try_from(&petx).unwrap();
    assert!(nx.multigraph);
}

// =============================
// == JSON roundtrip fidelity ==
// =============================
//
// These tests verify that the full pipeline
//   JSON string → NxGraphAdjFormat → PetxGraph → NxGraphAdjFormat → JSON string
// produces output whose serde_json::Value representation matches the input. This catches
// serialization artifacts (e.g. `"key": null` for absent fields) that struct-level roundtrip tests
// miss.

/// Normalize a `serde_json::Value` representing an NxGraphAdjFormat by sorting each adjacency list
/// by the stringified `"id"` field, so edge-order differences don't cause spurious failures.
fn normalize_json_value(v: &mut serde_json::Value) {
    if let Some(adj) = v.get_mut("adjacency").and_then(|a| a.as_array_mut()) {
        for list in adj.iter_mut() {
            if let Some(entries) = list.as_array_mut() {
                entries.sort_by(|a, b| {
                    let ak = a.get("id").map(|v| v.to_string()).unwrap_or_default();
                    let bk = b.get("id").map(|v| v.to_string()).unwrap_or_default();
                    ak.cmp(&bk)
                });
            }
        }
    }
}

/// Full JSON-level roundtrip for an undirected graph fixture.
fn assert_json_roundtrip_undirected(json_str: &str) {
    let nx: NxGraphAdjFormat = serde_json::from_str(json_str).unwrap();
    let petx: PetxUnGraph = nx.try_into().unwrap();
    let nx_back = NxGraphAdjFormat::try_from(&petx).unwrap();
    let serialized = serde_json::to_string(&nx_back).unwrap();

    let mut expected: serde_json::Value = serde_json::from_str(json_str).unwrap();
    let mut actual: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    normalize_json_value(&mut expected);
    normalize_json_value(&mut actual);
    assert_eq!(actual, expected);
}

/// Full JSON-level roundtrip for a directed graph fixture.
fn assert_json_roundtrip_directed(json_str: &str) {
    let nx: NxGraphAdjFormat = serde_json::from_str(json_str).unwrap();
    let petx: PetxDiGraph = nx.try_into().unwrap();
    let nx_back = NxGraphAdjFormat::try_from(&petx).unwrap();
    let serialized = serde_json::to_string(&nx_back).unwrap();

    let mut expected: serde_json::Value = serde_json::from_str(json_str).unwrap();
    let mut actual: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    normalize_json_value(&mut expected);
    normalize_json_value(&mut actual);
    assert_eq!(actual, expected);
}

#[test]
fn json_fidelity_karate_club() {
    assert_json_roundtrip_undirected(KARATE_JSON);
}

#[test]
fn json_fidelity_k5() {
    assert_json_roundtrip_undirected(K5_JSON);
}

#[test]
fn json_fidelity_p4() {
    assert_json_roundtrip_undirected(P4_JSON);
}

#[test]
fn json_fidelity_single_node() {
    assert_json_roundtrip_undirected(SINGLE_NODE_JSON);
}

#[test]
fn json_fidelity_two_triangles() {
    assert_json_roundtrip_undirected(TWO_TRIANGLES_JSON);
}

#[test]
fn json_fidelity_string_ids() {
    assert_json_roundtrip_undirected(STRING_IDS_JSON);
}

#[test]
fn json_fidelity_empty_edges() {
    assert_json_roundtrip_undirected(EMPTY_EDGES_JSON);
}

#[test]
fn json_fidelity_self_loop() {
    assert_json_roundtrip_undirected(SELF_LOOP_JSON);
}

#[test]
fn json_fidelity_small_directed() {
    assert_json_roundtrip_directed(SMALL_DIRECTED_JSON);
}

#[test]
fn json_fidelity_directed_cycle() {
    assert_json_roundtrip_directed(DIRECTED_CYCLE_JSON);
}

// ── nx_convert error paths ───────────────────────────────────────────

#[test]
fn directedness_mismatch_undirected_as_directed() {
    let nx = parse_nx(P4_JSON);
    assert!(!nx.directed);
    let err = PetxDiGraph::try_from(nx).unwrap_err();
    assert!(err.to_string().contains("directedness mismatch"));
}

#[test]
fn directedness_mismatch_directed_as_undirected() {
    let nx = parse_nx(SMALL_DIRECTED_JSON);
    assert!(nx.directed);
    let err = PetxUnGraph::try_from(nx).unwrap_err();
    assert!(err.to_string().contains("directedness mismatch"));
}

#[test]
fn node_adjacency_length_mismatch() {
    let mut nx = parse_nx(P4_JSON);
    nx.adjacency.pop();
    let err = PetxUnGraph::try_from(nx).unwrap_err();
    assert!(err.to_string().contains("length mismatch"));
}

#[test]
fn duplicate_node_id() {
    let json = r#"{
        "directed": false, "multigraph": false, "graph": [],
        "nodes": [{"id": 0}, {"id": 0}],
        "adjacency": [[], []]
    }"#;
    let nx = parse_nx(json);
    let err = PetxUnGraph::try_from(nx).unwrap_err();
    assert!(err.to_string().contains("duplicate node id"));
}

#[test]
fn missing_neighbor_node() {
    let json = r#"{
        "directed": false, "multigraph": false, "graph": [],
        "nodes": [{"id": 0}, {"id": 1}],
        "adjacency": [[{"id": 99}], []]
    }"#;
    let nx = parse_nx(json);
    let err = PetxUnGraph::try_from(nx).unwrap_err();
    assert!(err.to_string().contains("unknown node id"));
}

#[test]
fn petx_node_to_nx_node_missing_networkx_id() {
    let node = PetxNode {
        attrs: BTreeMap::new(),
    };
    let err = petx_node_to_nx_node(&node).unwrap_err();
    assert!(err.to_string().contains("__networkx_id__"));
}

#[test]
fn graph_has_parallel_edges_detects_multigraph() {
    let mut graph = UnGraph::<PetxNode, NxAdjEntry>::new_undirected();
    let a = graph.add_node(PetxNode {
        attrs: BTreeMap::from([("__networkx_id__".to_string(), json!(0))]),
    });
    let b = graph.add_node(PetxNode {
        attrs: BTreeMap::from([("__networkx_id__".to_string(), json!(1))]),
    });
    let edge = NxAdjEntry {
        id: json!(1),
        key: None,
        attrs: BTreeMap::new(),
    };
    graph.add_edge(a, b, edge.clone());
    graph.add_edge(a, b, edge);
    assert!(graph_has_parallel_edges(&graph));
}

#[test]
fn graph_has_no_parallel_edges_for_simple_graph() {
    let mut graph = DiGraph::<PetxNode, NxAdjEntry>::new();
    let a = graph.add_node(PetxNode {
        attrs: BTreeMap::from([("__networkx_id__".to_string(), json!(0))]),
    });
    let b = graph.add_node(PetxNode {
        attrs: BTreeMap::from([("__networkx_id__".to_string(), json!(1))]),
    });
    let edge = NxAdjEntry {
        id: json!(1),
        key: None,
        attrs: BTreeMap::new(),
    };
    graph.add_edge(a, b, edge);
    assert!(!graph_has_parallel_edges(&graph));
}

#[test]
fn nx_node_to_petx_node_preserves_attrs() {
    let nx_node = NxNode {
        id: json!(42),
        attrs: BTreeMap::from([("color".to_string(), json!("red"))]),
    };
    let petx = nx_node_to_petx_node(nx_node);
    assert_eq!(petx.attrs["__networkx_id__"], json!(42));
    assert_eq!(petx.attrs["color"], json!("red"));
}
