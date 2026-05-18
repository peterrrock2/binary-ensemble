use super::petxgraph::{apply_permutation, PetxGraph};
use super::rcm::{local_degree_in_component, rcm_component};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use petgraph::graph::{Graph, NodeIndex};
use petgraph::visit::NodeIndexable;
use rustworkx_core::connectivity::connected_components;
use std::cmp::Reverse;
use std::collections::HashSet;
use std::time::Duration;

/// Per-phase progress tracker for MLC, with one spinner line per recursion depth.
///
/// Phase 1 (depth 0) processes the original nodes; phase 2 processes the level-1 clusters produced
/// by phase 1; and so on. Bars are added lazily the first time a given depth is reached, and each
/// bar's total grows as new work at that depth is discovered (e.g. when the next top-level
/// component recurses and introduces more coarse nodes).
///
/// Spinners auto-hide when stderr is not a terminal (e.g. under `cargo test` or when output is
/// piped), so no config is needed for CI/test environments.
struct MlcProgress {
    multi: MultiProgress,
    bars: Vec<ProgressBar>,
    totals: Vec<usize>,
    dones: Vec<usize>,
}

impl MlcProgress {
    /// Create an empty tracker. Bars are added lazily as depths are reached.
    fn new() -> Self {
        Self {
            multi: MultiProgress::new(),
            bars: Vec::new(),
            totals: Vec::new(),
            dones: Vec::new(),
        }
    }

    /// Make sure a bar exists for `depth`, creating any intermediate bars that don't exist yet.
    fn ensure_depth(&mut self, depth: usize) {
        while self.bars.len() <= depth {
            let bar = self.multi.add(ProgressBar::new_spinner());
            bar.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.cyan} {msg}")
                    .unwrap(),
            );
            bar.enable_steady_tick(Duration::from_millis(100));
            self.bars.push(bar);
            self.totals.push(0);
            self.dones.push(0);
            let d = self.bars.len() - 1;
            self.refresh(d);
        }
    }

    /// Record that `n` more items will be processed at `depth`.
    fn add_total(&mut self, depth: usize, n: usize) {
        self.ensure_depth(depth);
        self.totals[depth] += n;
        self.refresh(depth);
    }

    /// Record that `n` more items at `depth` have been finalized.
    fn add_done(&mut self, depth: usize, n: usize) {
        self.ensure_depth(depth);
        self.dones[depth] += n;
        self.refresh(depth);
    }

    fn refresh(&self, depth: usize) {
        let done = self.dones[depth];
        let total = self.totals[depth];
        let pct = if total == 0 { 0 } else { done * 100 / total };
        self.bars[depth].set_message(format!(
            "MLC phase {}: {}/{} {} ({}%)",
            depth + 1,
            done,
            total,
            Self::unit_for_depth(depth),
            pct
        ));
    }

    fn unit_for_depth(depth: usize) -> String {
        if depth == 0 {
            "nodes".to_string()
        } else {
            format!("level-{} clusters", depth)
        }
    }

    /// Stop all spinners, leaving a final "complete" message on each.
    fn finish(&self) {
        for (d, bar) in self.bars.iter().enumerate() {
            bar.finish_with_message(format!(
                "MLC phase {}: complete ({} {})",
                d + 1,
                self.totals[d],
                Self::unit_for_depth(d)
            ));
        }
    }
}

/// Compute a multilevel cluster ordering and apply it to the graph in place.
///
/// The graph is reordered so that nodes which are topologically close end up at adjacent indices.
/// Each connected component is ordered independently, and components are sorted by their minimum
/// node index.
///
/// Arguments:
///
/// - `petx_graph`: The graph to reorder in place. Only edge topology is considered; node and edge
///   attributes are preserved but relocated.
///
/// Returns:
///
/// - The permutation that was applied: `order[new_index]` is the `NodeIndex` the node occupied
///   before reordering.
pub(super) fn apply_multi_level_clustering<Ty>(petx_graph: &mut PetxGraph<Ty>) -> Vec<NodeIndex>
where
    Ty: petgraph::EdgeType,
{
    let labels: Vec<usize> = (0..petx_graph.graph.node_bound()).collect();
    let mut progress = MlcProgress::new();
    let order = mlc_order_inner(&petx_graph.graph, &labels, &mut progress, 0);
    *petx_graph = apply_permutation(petx_graph, &order);

    progress.finish();
    order
}

/// Recursively order each connected component via multilevel clustering, then concatenate the
/// results.
///
/// Components are sorted by decreasing size (ties broken by minimum label) so that larger
/// components occupy the beginning of the output. Each component is ordered independently by
/// [`mlc_component`].
///
/// # Arguments
///
/// * `graph` - The input graph to order. Generic over node/edge weights and edge type so it also
///   works with the coarse graph during recursion.
/// * `labels` - A per-node label vector used for tie-breaking when choosing seeds and sorting
///   neighbors. Indexed by `NodeIndex::index()`.
/// * `progress` - Progress tracker for the multi-phase spinner display.
/// * `depth` - Recursion depth (0 at the top level). Used to route progress updates to the correct
///   phase bar.
///
/// # Returns
///
/// A permutation vector where `order[new_index]` is the `NodeIndex` of the node that should occupy
/// position `new_index`.
fn mlc_order_inner<N, E, Ty>(
    graph: &Graph<N, E, Ty>,
    labels: &[usize],
    progress: &mut MlcProgress,
    depth: usize,
) -> Vec<NodeIndex>
where
    Ty: petgraph::EdgeType,
{
    progress.add_total(depth, graph.node_count());

    let mut components: Vec<Vec<NodeIndex>> = connected_components(graph)
        .into_iter()
        .map(|set| set.into_iter().collect())
        .collect();
    components.sort_by_key(|c| {
        let min_label = c
            .iter()
            .map(|n| labels[n.index()])
            .min()
            .unwrap_or(usize::MAX);
        (Reverse(c.len()), min_label)
    });

    let mut order = Vec::with_capacity(graph.node_count());
    for component in components {
        order.extend(mlc_component(graph, labels, &component, progress, depth));
    }
    order
}

/// Order a single connected component by seed-expansion clustering plus recursive coarsening.
///
/// Steps:
///
/// 1. Singleton components return immediately.
/// 2. [`greedy_cluster_partition`] carves the component into stars.
/// 3. Each cluster is re-ordered internally via [`rcm_component`] on its induced subgraph, so
///    peripheral leaves bracket the cluster and the high-degree seed sits in the interior.
/// 4. If the partition returns a single cluster (a star that covers the whole component), that
///    RCM-ordered cluster is the final order.
/// 5. Otherwise a coarse graph is built with one node per cluster, and [`mlc_order_inner`] recurses
///    on it to decide the order in which clusters are emitted. The recursion terminates when each
///    coarse component collapses to a single cluster.
/// 6. The final order is produced by unrolling: emit clusters in the recursive coarse order, each
///    cluster in its RCM-ordered form.
///
/// # Arguments
///
/// * `graph` - The full graph (only edges within `component` are relevant).
/// * `labels` - Per-node labels for tie-breaking, indexed by `NodeIndex::index()`.
/// * `component` - The subset of `NodeIndex` values to order.
/// * `progress` - Progress tracker for the multi-phase spinner display.
/// * `depth` - Recursion depth; routes progress updates to the correct phase bar.
///
/// # Returns
///
/// A permutation of the nodes in `component` representing their new order.
fn mlc_component<N, E, Ty>(
    graph: &Graph<N, E, Ty>,
    labels: &[usize],
    component: &[NodeIndex],
    progress: &mut MlcProgress,
    depth: usize,
) -> Vec<NodeIndex>
where
    Ty: petgraph::EdgeType,
{
    if component.len() == 1 {
        progress.add_done(depth, 1);
        return vec![component[0]];
    }

    // `greedy_cluster_partition` ticks this depth's progress per cluster, so every node in
    // `component` contributes to phase `depth+1` exactly once.
    let mut clusters = greedy_cluster_partition(graph, labels, component, progress, depth);

    // Reorder each cluster internally via RCM on the subgraph induced by its members. This puts
    // peripheral (degree-1) nodes at both ends of the cluster and the high-degree seed near the
    // middle/end, which keeps cluster boundaries "loose" and avoids stranding the most- connected
    // node next to the previous cluster.
    for cluster in clusters.iter_mut() {
        *cluster = rcm_component(graph, labels, cluster);
    }

    // Single-cluster case: the whole component is one star.
    if clusters.len() == 1 {
        return clusters.into_iter().next().unwrap();
    }

    // Multi-cluster case: recurse on the coarse graph to decide the order in which the clusters
    // appear.
    let (coarse_graph, coarse_labels) = build_coarse_graph(graph, labels, &clusters);
    let coarse_order = mlc_order_inner(&coarse_graph, &coarse_labels, progress, depth + 1);

    let mut order = Vec::with_capacity(component.len());
    for coarse_node in coarse_order {
        order.extend(clusters[coarse_node.index()].iter().copied());
    }
    order
}

/// Partition a component into star-shaped clusters using a greedy seed-expansion strategy.
///
/// At each step, the lowest-degree unassigned node (ties broken by label) is chosen as a seed, and
/// the seed together with all of its unassigned neighbors becomes the next cluster. Local degrees
/// are then decremented for every unassigned node adjacent to a newly-assigned one, so subsequent
/// seed selections reflect the residual graph.
///
/// Only cluster *membership* is meaningful here; the internal order of each returned cluster is not
/// final and is expected to be overwritten by the caller (e.g. via [`rcm_component`]).
///
/// # Arguments
///
/// * `graph` - The full graph (only edges within `component` are relevant).
/// * `labels` - Per-node labels for tie-breaking, indexed by `NodeIndex::index()`.
/// * `component` - The subset of `NodeIndex` values to partition.
/// * `progress` - Progress tracker; `depth`'s done counter is advanced by each cluster's size as
///   the cluster is formed, so the caller's phase bar fills up gradually during large partitions.
/// * `depth` - Recursion depth of the caller, used to select the phase bar to update.
///
/// # Returns
///
/// A vector of clusters, where each cluster is a vector of `NodeIndex` values. Every node in
/// `component` appears in exactly one cluster.
fn greedy_cluster_partition<N, E, Ty>(
    graph: &Graph<N, E, Ty>,
    labels: &[usize],
    component: &[NodeIndex],
    progress: &mut MlcProgress,
    depth: usize,
) -> Vec<Vec<NodeIndex>>
where
    Ty: petgraph::EdgeType,
{
    let component_set: HashSet<NodeIndex> = component.iter().copied().collect();
    let mut local_deg = local_degree_in_component(graph, &component_set, component);

    let mut assigned = vec![false; graph.node_bound()];
    let mut remaining: Vec<NodeIndex> = component.to_vec();
    let mut clusters = Vec::new();

    while !remaining.is_empty() {
        remaining.sort_by_key(|&node| (local_deg[node.index()], labels[node.index()]));
        let seed = remaining[0];

        let mut cluster = vec![seed];
        assigned[seed.index()] = true;

        // Cluster membership is seed + every unassigned in-component neighbor. Internal order here
        // is irrelevant: the caller (`mlc_component`) overwrites it with an RCM ordering on the
        // cluster's induced subgraph.
        for neighbor in graph.neighbors(seed) {
            if component_set.contains(&neighbor) && !assigned[neighbor.index()] {
                assigned[neighbor.index()] = true;
                cluster.push(neighbor);
            }
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
        progress.add_done(depth, cluster.len());
        clusters.push(cluster);
    }

    clusters
}

/// Build a coarse graph where each cluster is contracted into a single node.
///
/// The coarse graph is always undirected: an edge exists between two coarse nodes whenever any
/// original-graph edge connects their clusters. Each coarse node's label is the minimum original
/// label among its cluster members.
///
/// # Arguments
///
/// * `graph` - The full graph containing the original edges.
/// * `labels` - Per-node labels for the original graph, indexed by `NodeIndex::index()`.
/// * `clusters` - The partition produced by [`greedy_cluster_partition`]. Cluster `i` maps to
///   coarse node `i`.
///
/// # Returns
///
/// A tuple of:
/// * The coarse `Graph<(), (), Undirected>` with one node per cluster and one edge per
///   inter-cluster connection.
/// * A label vector for the coarse graph (one entry per cluster), where each label is the minimum
///   original label in that cluster.
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
