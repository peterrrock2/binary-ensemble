use super::petxgraph::{apply_permutation, PetxGraph};
use petgraph::graph::{Graph, NodeIndex};
use petgraph::visit::NodeIndexable;
use rustworkx_core::connectivity::connected_components;
use std::collections::{HashSet, VecDeque};

/// Compute a Reverse Cuthill-McKee ordering and apply it to the graph in place.
///
/// Each connected component is ordered independently via RCM, and components are sorted by their
/// minimum node index. The graph is then permuted in place.
///
/// Arguments:
///
/// - `petx_graph`: The graph to reorder in place.
///
/// Returns:
///
/// - The permutation that was applied: `order[new_index]` is the `NodeIndex` the node occupied
///   before reordering.
pub(super) fn apply_reverse_cuthill_mckee<Ty>(petx_graph: &mut PetxGraph<Ty>) -> Vec<NodeIndex>
where
    Ty: petgraph::EdgeType,
{
    let labels: Vec<usize> = (0..petx_graph.graph.node_bound()).collect();
    let graph = &petx_graph.graph;

    let mut components: Vec<Vec<NodeIndex>> = connected_components(graph)
        .into_iter()
        .map(|set| set.into_iter().collect())
        .collect();
    components.sort_by_key(|c| {
        c.iter()
            .map(|n| labels[n.index()])
            .min()
            .unwrap_or(usize::MAX)
    });

    let mut order = Vec::with_capacity(graph.node_count());
    for component in components {
        order.extend(rcm_component(graph, &labels, &component));
    }

    *petx_graph = apply_permutation(petx_graph, &order);
    order
}

/// Reverse Cuthill-McKee ordering for a single connected component.
///
/// Starts BFS from the minimum-degree node (ties broken by label), then reverses the result to
/// produce the RCM permutation.
///
/// # Arguments
///
/// * `graph` - The full graph (only edges within `component` are relevant).
/// * `labels` - Per-node labels for tie-breaking, indexed by `NodeIndex::index()`.
/// * `component` - The subset of `NodeIndex` values to order.
///
/// # Returns
///
/// A permutation of the nodes in `component` representing their RCM order.
pub(super) fn rcm_component<N, E, Ty>(
    graph: &Graph<N, E, Ty>,
    labels: &[usize],
    component: &[NodeIndex],
) -> Vec<NodeIndex>
where
    Ty: petgraph::EdgeType,
{
    let component_set: HashSet<NodeIndex> = component.iter().copied().collect();
    let local_deg = local_degree_in_component(graph, &component_set, component);

    let start = component
        .iter()
        .copied()
        .min_by_key(|&node| (local_deg[node.index()], labels[node.index()]))
        .unwrap();

    let mut visited = vec![false; graph.node_bound()];
    visited[start.index()] = true;
    let mut queue = VecDeque::from([start]);
    let mut order = Vec::with_capacity(component.len());

    while let Some(node) = queue.pop_front() {
        order.push(node);
        let mut neighbors: Vec<NodeIndex> = graph
            .neighbors(node)
            .filter(|&n| component_set.contains(&n) && !visited[n.index()])
            .collect();
        neighbors.sort_by_key(|&n| (local_deg[n.index()], labels[n.index()]));
        for n in neighbors {
            visited[n.index()] = true;
            queue.push_back(n);
        }
    }

    order.reverse();
    order
}

/// Compute the degree of each component node restricted to the component.
///
/// For each node in `component`, counts how many of its neighbors are also in the component. The
/// result is indexed by `NodeIndex::index()`, so entries for nodes outside the component are zero.
///
/// # Arguments
///
/// * `graph` - The full graph.
/// * `component_set` - A `HashSet` of the nodes in the component, used for fast membership checks.
/// * `component` - The slice of `NodeIndex` values in the component.
///
/// # Returns
///
/// A vector of length `graph.node_bound()` where `result[node.index()]` is the number of neighbors
/// of `node` that are in the component, or `0` for nodes not in the component.
pub(super) fn local_degree_in_component<N, E, Ty>(
    graph: &Graph<N, E, Ty>,
    component_set: &HashSet<NodeIndex>,
    component: &[NodeIndex],
) -> Vec<usize>
where
    Ty: petgraph::EdgeType,
{
    let mut local_deg = vec![0usize; graph.node_bound()];
    for &node in component {
        local_deg[node.index()] = graph
            .neighbors(node)
            .filter(|n| component_set.contains(n))
            .count();
    }
    local_deg
}
