use clap::{Parser, ValueEnum};

#[derive(Parser, Debug, Clone, ValueEnum, PartialEq)]
/// Defines the mode of operation.
pub(super) enum Mode {
    /// Sort a JSON dual graph by a key and emit a relabeling map.
    Json,
    /// Relabel or canonicalize a BEN file.
    Ben,
}

#[derive(Parser, Debug, Clone, ValueEnum, PartialEq)]
/// Topology-based ordering methods for JSON graph relabeling.
pub(super) enum OrderingMethod {
    /// Recursive multilevel clustering based on local neighborhoods.
    #[clap(alias = "mlc")]
    MultiLevelCluster,
    /// Reverse Cuthill-McKee ordering.
    #[clap(alias = "rcm")]
    ReverseCuthillMckee,
}

#[derive(Parser, Debug, Clone, ValueEnum, PartialEq)]
/// BEN variants supported for BEN-mode output.
pub(super) enum BenCliVariant {
    Standard,
    MkvChain,
    #[clap(alias = "twodelta")]
    TwoDelta,
}

#[derive(Parser, Debug)]
#[command(
    name = "Relabeling Binary Ensemble CLI Tool",
    about = concat!(
        "This is a command line tool for relabeling binary ensembles ",
        "to help improve compression ratios for BEN and XBEN files."
    ),
    version
)]
/// Defines the command line arguments accepted by the program.
// TODO: Change the name of shape_file to dual_graph_file.
pub(super) struct Args {
    /// Input file to read from.
    #[arg()]
    pub input_file: String,
    /// Output file to write to.
    #[arg(short, long)]
    pub output_file: Option<String>,
    /// Key to sort the JSON or BEN file by.
    #[arg(short, long)]
    pub key: Option<String>,
    /// Topology-based ordering method to use instead of a key sort.
    #[arg(long, value_enum)]
    pub ordering: Option<OrderingMethod>,
    /// Shape file to use for sorting the BEN file. Only needed
    /// in BEN mode when a map is not provided.
    #[arg(short, long)]
    pub shape_file: Option<String>,
    /// Map file to use for relabeling the BEN file.
    #[arg(short = 'p', long)]
    pub map_file: Option<String>,
    /// Mode to run the program in (either JSON or BEN).
    /// The JSON mode will sort a JSON file by a given key or graph-ordering
    /// method. The BEN mode will relabel a BEN file according to a map file
    /// or a graph-ordering request (which also requires a dual-graph file). If no
    /// map file or key is provided, the BEN mode will canonicalize
    /// the assignment vectors in the BEN file.
    #[arg(short, long)]
    pub mode: Mode,
    /// Only relabel the first `n` expanded samples in BEN mode.
    #[arg(long)]
    pub n_items: Option<usize>,
    /// BEN variant to use for the BEN-mode output file.
    #[arg(long, value_enum)]
    pub output_variant: Option<BenCliVariant>,
    /// Rewrite the BEN stream without canonicalizing or map relabeling.
    #[arg(long)]
    pub convert_only: bool,
    /// Verbosity level for the program.
    #[arg(short, long)]
    pub verbose: bool,
    /// Suppress in-place progress spinners. Trace logging is unaffected.
    #[arg(short = 'q', long)]
    pub quiet: bool,
}
