use super::petxgraph::{apply_permutation, PetxGraph};
use super::rcm::{local_degree_in_component, rcm_component};
use petgraph::graph::{Graph, NodeIndex};
use petgraph::visit::NodeIndexable;
use rustworkx_core::connectivity::connected_components;
use std::cmp::Reverse;
use std::collections::HashSet;

/// Tracks how many original nodes have been finalized so far and emits
/// periodic `tracing::info!` milestones when verbose logging is enabled.
///
/// Progress is measured in real-node chunks (base-case RCM calls and
/// per-cluster RCM calls at depth 0). Coarse-graph recursion does not
/// contribute, so `total` corresponds exactly to the number of nodes in
/// the original graph.
struct MlcProgress {
    total: usize,
    done: usize,
    last_logged_pct: usize,
}

impl MlcProgress {
    /// Create a new progress tracker for a graph with `total` real nodes.
    fn new(total: usize) -> Self {
        Self {
            total,
            done: 0,
            last_logged_pct: 0,
        }
    }

    /// Record that `chunk` more real nodes have been finalized. Emits an
    /// `info!` log line whenever completion crosses a 5% boundary.
    fn add(&mut self, chunk: usize) {
        self.done += chunk;
        let pct = if self.total == 0 {
            100
        } else {
            self.done * 100 / self.total
        };
        if pct >= self.last_logged_pct + 5 || self.done == self.total {
            tracing::info!(
                "MLC progress: {}/{} nodes ({}%)",
                self.done,
                self.total,
                pct
            );
            self.last_logged_pct = pct;
        }
    }
}

/// Compute a multilevel cluster ordering and apply it to the graph in place.
///
/// The graph is reordered so that nodes which are topologically close end up
/// at adjacent indices. Each connected component is ordered independently,
/// and components are sorted by their minimum node index.
///
/// Arguments:
///
/// - `petx_graph`: The graph to reorder in place. Only edge topology is
///   considered; node and edge attributes are preserved but relocated.
///
/// Returns:
///
/// - The permutation that was applied: `order[new_index]` is the `NodeIndex`
///   the node occupied before reordering.
pub(super) fn apply_multi_level_clustering<Ty>(petx_graph: &mut PetxGraph<Ty>) -> Vec<NodeIndex>
where
    Ty: petgraph::EdgeType,
{
    let total_nodes = petx_graph.graph.node_count();
    tracing::info!("MLC: starting on graph with {} nodes", total_nodes);

    let labels: Vec<usize> = (0..petx_graph.graph.node_bound()).collect();
    let mut progress = MlcProgress::new(total_nodes);
    let order = mlc_order_inner(&petx_graph.graph, &labels, Some(&mut progress), 0);
    *petx_graph = apply_permutation(petx_graph, &order);

    tracing::info!("MLC: complete");
    order
}

/// Recursively order each connected component via multilevel clustering, then
/// concatenate the results.
///
/// Components are sorted by their minimum label so that the output order is
/// deterministic. Each component is ordered independently by
/// [`mlc_component`].
///
/// # Arguments
///
/// * `graph` - The input graph to order. Generic over node/edge weights and
///   edge type so it also works with the coarse graph during recursion.
/// * `labels` - A per-node label vector used for tie-breaking when choosing
///   BFS seeds and sorting neighbors. Indexed by `NodeIndex::index()`.
/// * `progress` - Optional progress tracker. `Some(_)` at the top level and
///   `None` when recursing into the coarse graph so only real-node work
///   contributes to the counter.
/// * `depth` - Recursion depth. Zero at the top level, incremented each
///   time we recurse into a coarse graph. Used only for logging.
///
/// # Returns
///
/// A permutation vector where `order[new_index]` is the `NodeIndex` of the
/// node that should occupy position `new_index`.
fn mlc_order_inner<N, E, Ty>(
    graph: &Graph<N, E, Ty>,
    labels: &[usize],
    mut progress: Option<&mut MlcProgress>,
    depth: usize,
) -> Vec<NodeIndex>
where
    Ty: petgraph::EdgeType,
{
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

    tracing::debug!(
        "MLC depth={}: {} component(s) to order",
        depth,
        components.len()
    );

    let mut order = Vec::with_capacity(graph.node_count());
    for component in components {
        order.extend(mlc_component(
            graph,
            labels,
            &component,
            progress.as_deref_mut(),
            depth,
        ));
    }
    order
}

/// Recursively order a single connected component via multilevel clustering.
///
/// Single-node components are returned as-is. Otherwise the component is
/// greedily partitioned into clusters; each cluster is then ordered by
/// recursively applying `mlc_component` to it, a coarse graph of
/// inter-cluster edges is built, and the coarse graph is ordered via
/// [`mlc_order_inner`] to determine the final cluster sequence.
///
/// If the greedy partition produces a single cluster (or the unreachable
/// all-singletons case), the algorithm cannot make progress and falls back
/// to RCM on the whole component.
///
/// # Arguments
///
/// * `graph` - The full graph (only edges within `component` are relevant).
/// * `labels` - Per-node labels for tie-breaking, indexed by
///   `NodeIndex::index()`.
/// * `component` - The subset of `NodeIndex` values to order.
/// * `progress` - Optional progress tracker, `Some(_)` only when ordering
///   real nodes. Advanced when the recursion bottoms out at a singleton
///   component or hits the degenerate RCM fallback.
/// * `depth` - Recursion depth, zero at the top level. Used for logging.
///
/// # Returns
///
/// A permutation of the nodes in `component` representing their new order.
fn mlc_component<N, E, Ty>(
    graph: &Graph<N, E, Ty>,
    labels: &[usize],
    component: &[NodeIndex],
    mut progress: Option<&mut MlcProgress>,
    depth: usize,
) -> Vec<NodeIndex>
where
    Ty: petgraph::EdgeType,
{
    if component.len() == 1 {
        tracing::debug!("MLC depth={}: singleton component", depth);
        if let Some(p) = progress.as_deref_mut() {
            p.add(1);
        }
        return vec![component[0]];
    }

    let clusters = greedy_cluster_partition(graph, labels, component);
    if clusters.len() <= 1 || clusters.len() == component.len() {
        tracing::debug!(
            "MLC depth={}: degenerate partition ({} clusters from {} nodes), falling back to RCM",
            depth,
            clusters.len(),
            component.len()
        );
        let order = rcm_component(graph, labels, component);
        if let Some(p) = progress.as_deref_mut() {
            p.add(component.len());
        }
        return order;
    }

    tracing::debug!(
        "MLC depth={}: partitioned {} nodes into {} clusters",
        depth,
        component.len(),
        clusters.len()
    );

    let mut cluster_orders: Vec<Vec<NodeIndex>> = Vec::with_capacity(clusters.len());
    for cluster in &clusters {
        let order = mlc_component(graph, labels, cluster, progress.as_deref_mut(), depth + 1);
        cluster_orders.push(order);
    }

    let (coarse_graph, coarse_labels) = build_coarse_graph(graph, labels, &clusters);
    let coarse_order = mlc_order_inner(&coarse_graph, &coarse_labels, None, depth + 1);

    let mut order = Vec::with_capacity(component.len());
    for coarse_node in coarse_order {
        order.extend(cluster_orders[coarse_node.index()].iter().copied());
    }
    order
}

/// Partition a component into small clusters using a greedy seed-expansion
/// strategy.
///
/// Seeds are chosen in order of increasing local degree (ties broken by label).
/// Each seed expands to include all of its unassigned neighbors. After each
/// cluster is formed, local degrees are incrementally updated: for every
/// unassigned neighbor of a newly-assigned node, the neighbor's degree is
/// decremented. Nodes are then re-sorted before picking the next seed.
///
/// # Arguments
///
/// * `graph` - The full graph (only edges within `component` are relevant).
/// * `labels` - Per-node labels for tie-breaking, indexed by
///   `NodeIndex::index()`.
/// * `component` - The subset of `NodeIndex` values to partition.
///
/// # Returns
///
/// A vector of clusters, where each cluster is a vector of `NodeIndex`
/// values. Every node in `component` appears in exactly one cluster.
fn greedy_cluster_partition<N, E, Ty>(
    graph: &Graph<N, E, Ty>,
    labels: &[usize],
    component: &[NodeIndex],
) -> Vec<Vec<NodeIndex>>
where
    Ty: petgraph::EdgeType,
{
    let component_set: HashSet<NodeIndex> = component.iter().copied().collect();
    let mut local_deg = local_degree_in_component(graph, &component_set, component);

    let mut assigned = vec![false; graph.node_bound()];
    let mut remaining: Vec<NodeIndex> = component.to_vec();
    let mut clusters = Vec::new();

    // Epoch-based marking for seed neighbors avoids rebuilding a set each
    // iteration.
    let mut seed_marks = vec![0usize; graph.node_bound()];
    let mut mark_epoch = 1usize;

    while !remaining.is_empty() {
        remaining.sort_by_key(|&node| (local_deg[node.index()], labels[node.index()]));
        let seed = remaining[0];

        let mut cluster = vec![seed];
        assigned[seed.index()] = true;

        for neighbor in graph.neighbors(seed) {
            if component_set.contains(&neighbor) {
                seed_marks[neighbor.index()] = mark_epoch;
            }
        }

        let mut candidates: Vec<NodeIndex> = graph
            .neighbors(seed)
            .filter(|&n| component_set.contains(&n) && !assigned[n.index()])
            .collect();
        candidates.sort_by_key(|&neighbor| {
            let shared = graph
                .neighbors(neighbor)
                .filter(|&next| {
                    component_set.contains(&next) && seed_marks[next.index()] == mark_epoch
                })
                .count();
            (
                Reverse(shared),
                local_deg[neighbor.index()],
                labels[neighbor.index()],
            )
        });

        for neighbor in candidates {
            assigned[neighbor.index()] = true;
            cluster.push(neighbor);
        }

        mark_epoch = mark_epoch.wrapping_add(1);
        if mark_epoch == 0 {
            seed_marks.fill(0);
            mark_epoch = 1;
        }

        // Decrement degrees of unassigned nodes adjacent to the new cluster.
        for &node in &cluster {
            for neighbor in graph.neighbors(node) {
                if component_set.contains(&neighbor) && !assigned[neighbor.index()] {
                    local_deg[neighbor.index()] -= 1;
                }
            }
        }

        remaining.retain(|&n| !assigned[n.index()]);
        clusters.push(cluster);
    }

    clusters
}

/// Build a coarse graph where each cluster is contracted into a single node.
///
/// The coarse graph is always undirected: an edge exists between two coarse
/// nodes whenever any original-graph edge connects their clusters. Each coarse
/// node's label is the minimum original label among its cluster members.
///
/// # Arguments
///
/// * `graph` - The full graph containing the original edges.
/// * `labels` - Per-node labels for the original graph, indexed by
///   `NodeIndex::index()`.
/// * `clusters` - The partition produced by [`greedy_cluster_partition`].
///   Cluster `i` maps to coarse node `i`.
///
/// # Returns
///
/// A tuple of:
/// * The coarse `Graph<(), (), Undirected>` with one node per cluster and
///   one edge per inter-cluster connection.
/// * A label vector for the coarse graph (one entry per cluster), where
///   each label is the minimum original label in that cluster.
fn build_coarse_graph<N, E, Ty>(
    graph: &Graph<N, E, Ty>,
    labels: &[usize],
    clusters: &[Vec<NodeIndex>],
) -> (Graph<(), (), petgraph::Undirected>, Vec<usize>)
where
    Ty: petgraph::EdgeType,
{
    let mut cluster_of = vec![usize::MAX; graph.node_bound()];
    for (ci, cluster) in clusters.iter().enumerate() {
        for &node in cluster {
            cluster_of[node.index()] = ci;
        }
    }

    let mut coarse_graph = Graph::<(), (), petgraph::Undirected>::with_capacity(clusters.len(), 0);
    for _ in 0..clusters.len() {
        coarse_graph.add_node(());
    }

    let mut seen_edges: HashSet<(usize, usize)> = HashSet::new();
    for (ci, cluster) in clusters.iter().enumerate() {
        for &node in cluster {
            for neighbor in graph.neighbors(node) {
                let nc = cluster_of[neighbor.index()];
                if nc != ci && nc != usize::MAX {
                    let canonical = if ci < nc { (ci, nc) } else { (nc, ci) };
                    if seen_edges.insert(canonical) {
                        coarse_graph.add_edge(NodeIndex::new(ci), NodeIndex::new(nc), ());
                    }
                }
            }
        }
    }

    let coarse_labels: Vec<usize> = clusters
        .iter()
        .map(|cluster| {
            cluster
                .iter()
                .map(|n| labels[n.index()])
                .min()
                .unwrap_or(usize::MAX)
        })
        .collect();

    (coarse_graph, coarse_labels)
}
