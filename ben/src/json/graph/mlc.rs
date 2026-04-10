use super::petxgraph::{apply_permutation, PetxGraph};
use super::rcm::rcm_component;
use petgraph::graph::{Graph, NodeIndex};
use petgraph::visit::{EdgeRef, NodeIndexable};
use rustworkx_core::connectivity::connected_components;
use std::collections::{HashMap, HashSet};

/// Cap on Louvain local-move passes within a single level. Phase 1
/// usually converges in far fewer passes; the cap is purely defensive.
const LOUVAIN_MAX_PASSES: usize = 32;

/// Cap on Louvain coarsening levels (phase 1 + contract iterations).
/// Each level either strictly reduces node count or hits a modularity
/// fixed point, so this bound is purely defensive.
const LOUVAIN_MAX_LEVELS: usize = 32;

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

    let clusters = louvain_cluster_partition(graph, labels, component);
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

/// Partition a component into communities via full multilevel Louvain.
///
/// Runs the standard two-phase Louvain algorithm end-to-end:
///
/// 1. **Phase 1 (local moves):** each node is repeatedly considered for
///    moving into one of its neighbors' communities, picking the move
///    that maximizes the modularity gain. Passes over the node set
///    continue until no move improves modularity or
///    [`LOUVAIN_MAX_PASSES`] is reached.
/// 2. **Phase 2 (contract):** each community is collapsed into a single
///    super-node, with intra-community edges becoming self-loops and
///    inter-community edges becoming weighted edges between super-nodes.
///    The coarser graph is then fed back into phase 1.
///
/// The loop terminates when a phase-1 sweep makes no moves (a fixed
/// point of the modularity objective) or after [`LOUVAIN_MAX_LEVELS`]
/// levels. Unlike the single-level variant, each contract step is
/// consumed internally — the caller still uses MLC's existing
/// coarse-graph machinery ([`build_coarse_graph`] + [`mlc_order_inner`])
/// to *order* clusters, but the clustering itself is already coarsened.
///
/// Internally this operates on [`LouvainGraph`], a compact adjacency
/// representation that tracks weighted edges, self-loop weights, node
/// degrees, and total weight m. The modularity move-gain is the standard
/// integer-safe form
///
/// ```text
/// Δ ∝ 2m·k_{i,in}(C) − k_i·Σ_tot(C),
/// ```
///
/// computed after temporarily removing node i from its current community
/// so the "stay put" baseline uses the same formula and ties prefer
/// staying. Node-processing order is deterministic (by level label).
///
/// # Arguments
///
/// * `graph` - The full graph (only edges within `component` are relevant).
/// * `labels` - Per-node labels used to fix a deterministic
///   node-processing order and tiebreak across runs.
/// * `component` - The subset of `NodeIndex` values to partition.
///
/// # Returns
///
/// A vector of clusters, one per community found at the coarsest level.
/// Clusters are sorted by their minimum-label member; nodes within each
/// cluster are sorted by label. Every node in `component` appears in
/// exactly one cluster.
fn louvain_cluster_partition<N, E, Ty>(
    graph: &Graph<N, E, Ty>,
    labels: &[usize],
    component: &[NodeIndex],
) -> Vec<Vec<NodeIndex>>
where
    Ty: petgraph::EdgeType,
{
    let mut g = louvain_init_graph(graph, component);

    // No edges means no modularity to optimize; return singletons so the
    // caller's degenerate-partition guard kicks in.
    if g.total_weight == 0 {
        return component.iter().map(|&n| vec![n]).collect();
    }

    // Each super-node currently groups one or more original NodeIndex
    // values from `component`. Initially every node is its own super-node.
    let mut super_nodes: Vec<Vec<NodeIndex>> =
        component.iter().map(|&n| vec![n]).collect();

    // Level-labels track the minimum original label within each super-node
    // so phase 1 has a deterministic processing order at every level.
    let mut level_labels: Vec<usize> =
        component.iter().map(|&n| labels[n.index()]).collect();

    for _ in 0..LOUVAIN_MAX_LEVELS {
        let (community_of, any_move) = louvain_phase1(&g, &level_labels);
        if !any_move {
            break;
        }

        let (new_g, new_id_of) = louvain_contract(&g, &community_of);
        let new_n = new_g.n();

        // Roll super-node membership and level labels forward one level.
        let mut new_super_nodes: Vec<Vec<NodeIndex>> = vec![Vec::new(); new_n];
        let mut new_level_labels: Vec<usize> = vec![usize::MAX; new_n];
        for old in 0..g.n() {
            let new_c = new_id_of[old];
            let chunk = std::mem::take(&mut super_nodes[old]);
            new_super_nodes[new_c].extend(chunk);
            if level_labels[old] < new_level_labels[new_c] {
                new_level_labels[new_c] = level_labels[old];
            }
        }

        super_nodes = new_super_nodes;
        level_labels = new_level_labels;
        g = new_g;

        if g.n() <= 1 {
            break;
        }
    }

    // Deterministic output: nodes sorted by label within each cluster, and
    // clusters sorted by their minimum-label member.
    let mut clusters = super_nodes;
    for cluster in &mut clusters {
        cluster.sort_by_key(|&n| labels[n.index()]);
    }
    clusters.sort_by_key(|cluster| labels[cluster[0].index()]);
    clusters
}

/// Compact weighted-undirected graph used by the multilevel Louvain
/// implementation.
///
/// `adj[u]` lists `(v, weight)` pairs for non-loop edges; each non-loop
/// edge appears in both endpoints' lists (symmetric storage). Self-loops
/// are stored separately in `self_loop[u]` so `adj` never contains
/// self-references.
///
/// Weighted modularity conventions apply:
/// `deg[u] = Σ adj[u].1 + 2·self_loop[u]`, and
/// `total_weight = Σ deg[u] / 2`.
struct LouvainGraph {
    adj: Vec<Vec<(usize, i64)>>,
    self_loop: Vec<i64>,
    deg: Vec<i64>,
    total_weight: i64,
}

impl LouvainGraph {
    fn n(&self) -> usize {
        self.adj.len()
    }
}

/// Build the initial [`LouvainGraph`] for a connected component of the
/// original unweighted graph.
///
/// Every edge starts with weight 1. Self-loops (should any exist in the
/// input) are routed into `self_loop`. Nodes are compacted into
/// `0..component.len()` following the order of `component`.
fn louvain_init_graph<N, E, Ty>(
    graph: &Graph<N, E, Ty>,
    component: &[NodeIndex],
) -> LouvainGraph
where
    Ty: petgraph::EdgeType,
{
    let n = component.len();
    let mut idx_of = vec![usize::MAX; graph.node_bound()];
    for (i, &node) in component.iter().enumerate() {
        idx_of[node.index()] = i;
    }

    let mut adj_maps: Vec<HashMap<usize, i64>> = (0..n).map(|_| HashMap::new()).collect();
    let mut self_loop = vec![0i64; n];

    // `edge_references()` yields each edge exactly once, which avoids
    // any ambiguity around how petgraph reports self-loops via
    // `neighbors()`.
    for edge_ref in graph.edge_references() {
        let u = idx_of[edge_ref.source().index()];
        let v = idx_of[edge_ref.target().index()];
        if u == usize::MAX || v == usize::MAX {
            continue;
        }
        if u == v {
            self_loop[u] += 1;
        } else {
            *adj_maps[u].entry(v).or_insert(0) += 1;
            *adj_maps[v].entry(u).or_insert(0) += 1;
        }
    }

    let adj: Vec<Vec<(usize, i64)>> = adj_maps
        .into_iter()
        .map(|m| {
            let mut v: Vec<_> = m.into_iter().collect();
            v.sort_unstable_by_key(|&(nb, _)| nb);
            v
        })
        .collect();

    let mut deg = vec![0i64; n];
    for u in 0..n {
        let s: i64 = adj[u].iter().map(|&(_, w)| w).sum();
        deg[u] = s + 2 * self_loop[u];
    }
    let total_weight = deg.iter().sum::<i64>() / 2;

    LouvainGraph {
        adj,
        self_loop,
        deg,
        total_weight,
    }
}

/// Run one Louvain phase-1 (local-move) sweep on a [`LouvainGraph`].
///
/// Each node starts in its own community. At most [`LOUVAIN_MAX_PASSES`]
/// passes over the node set are attempted; passes stop early once a full
/// pass completes with no improving move. Nodes are processed in
/// ascending `level_labels` order for determinism.
///
/// # Returns
///
/// * A dense assignment `community_of[u]` giving the community id for
///   each node. Ids are integers in `0..n` but not necessarily contiguous
///   — [`louvain_contract`] remaps them.
/// * A flag that is `true` if at least one node moved during the sweep.
fn louvain_phase1(g: &LouvainGraph, level_labels: &[usize]) -> (Vec<usize>, bool) {
    let n = g.n();
    let m2 = 2 * g.total_weight;

    let mut community_of: Vec<usize> = (0..n).collect();
    let mut community_sum_deg: Vec<i64> = g.deg.clone();

    let mut node_order: Vec<usize> = (0..n).collect();
    node_order.sort_by_key(|&u| level_labels[u]);

    // Scratch buffers reused across nodes.
    let mut contrib: Vec<i64> = vec![0; n];
    let mut contrib_keys: Vec<usize> = Vec::new();

    let mut any_move = false;

    for _ in 0..LOUVAIN_MAX_PASSES {
        let mut improved = false;

        for &u in &node_order {
            let ci = community_of[u];
            let k_u = g.deg[u];

            // Tally weighted edges to each neighbor community.
            for &(v, w) in &g.adj[u] {
                let cj = community_of[v];
                if contrib[cj] == 0 {
                    contrib_keys.push(cj);
                }
                contrib[cj] += w;
            }

            // Temporarily remove u from its current community so that
            // `community_sum_deg[ci]` and `contrib[ci]` reflect the
            // post-removal state uniformly for every candidate.
            community_sum_deg[ci] -= k_u;

            // Baseline candidate: stay in `ci`. `contrib[ci]` is 0 if no
            // neighbor is currently in `ci`, which is the correct value.
            let mut best_community = ci;
            let mut best_gain = m2 * contrib[ci] - k_u * community_sum_deg[ci];

            // Sort touched keys so ties deterministically prefer the
            // lower community id.
            contrib_keys.sort_unstable();
            for &cj in &contrib_keys {
                if cj == ci {
                    continue;
                }
                let gain = m2 * contrib[cj] - k_u * community_sum_deg[cj];
                if gain > best_gain {
                    best_gain = gain;
                    best_community = cj;
                }
            }

            // Reset scratch for the next node.
            for &k in &contrib_keys {
                contrib[k] = 0;
            }
            contrib_keys.clear();

            // Commit the (possibly no-op) move.
            community_sum_deg[best_community] += k_u;
            if best_community != ci {
                community_of[u] = best_community;
                improved = true;
                any_move = true;
            }
        }

        if !improved {
            break;
        }
    }

    (community_of, any_move)
}

/// Contract a [`LouvainGraph`] using a community assignment, producing a
/// new (coarser) graph whose nodes are the communities.
///
/// Intra-community edges become contributions to the new node's
/// self-loop; inter-community edges become weighted edges between
/// super-nodes. Both old self-loops and internal edges at this level are
/// preserved in the new self-loop so the total weight is invariant.
///
/// # Arguments
///
/// * `g` - The current-level graph.
/// * `community_of` - Community assignment produced by
///   [`louvain_phase1`]. Values may be any integers in `0..g.n()`.
///
/// # Returns
///
/// * The new coarser [`LouvainGraph`].
/// * A remap `new_id_of[u]` giving the new super-node index of each old
///   node. Used by the caller to roll super-node membership forward.
fn louvain_contract(g: &LouvainGraph, community_of: &[usize]) -> (LouvainGraph, Vec<usize>) {
    let n = g.n();

    // Dense-remap community ids to 0..new_n in first-seen order.
    let mut dense = vec![usize::MAX; n];
    let mut new_id_of = vec![usize::MAX; n];
    let mut new_n = 0usize;
    for u in 0..n {
        let c = community_of[u];
        if dense[c] == usize::MAX {
            dense[c] = new_n;
            new_n += 1;
        }
        new_id_of[u] = dense[c];
    }

    // Carry forward existing self-loops verbatim.
    let mut new_self_loop = vec![0i64; new_n];
    for u in 0..n {
        new_self_loop[new_id_of[u]] += g.self_loop[u];
    }

    // Aggregate edges. Internal (intra-community) edges are double-counted
    // by the symmetric adjacency, so we accumulate and halve at the end.
    // External edges are also double-counted, but symmetrically across
    // the two endpoints — which is exactly the shape the new symmetric
    // adj needs, so no halving is required there.
    let mut internal_accum = vec![0i64; new_n];
    let mut new_adj_maps: Vec<HashMap<usize, i64>> = (0..new_n).map(|_| HashMap::new()).collect();

    for u in 0..n {
        let cu = new_id_of[u];
        for &(v, w) in &g.adj[u] {
            let cv = new_id_of[v];
            if cu == cv {
                internal_accum[cu] += w;
            } else {
                *new_adj_maps[cu].entry(cv).or_insert(0) += w;
            }
        }
    }

    for c in 0..new_n {
        new_self_loop[c] += internal_accum[c] / 2;
    }

    let new_adj: Vec<Vec<(usize, i64)>> = new_adj_maps
        .into_iter()
        .map(|m| {
            let mut v: Vec<_> = m.into_iter().collect();
            v.sort_unstable_by_key(|&(nb, _)| nb);
            v
        })
        .collect();

    let mut new_deg = vec![0i64; new_n];
    for c in 0..new_n {
        let s: i64 = new_adj[c].iter().map(|&(_, w)| w).sum();
        new_deg[c] = s + 2 * new_self_loop[c];
    }
    let new_total = new_deg.iter().sum::<i64>() / 2;

    debug_assert_eq!(new_total, g.total_weight);

    (
        LouvainGraph {
            adj: new_adj,
            self_loop: new_self_loop,
            deg: new_deg,
            total_weight: new_total,
        },
        new_id_of,
    )
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
/// * `clusters` - The partition produced by [`louvain_cluster_partition`].
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
