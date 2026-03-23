/// A buffered delta frame awaiting chunk serialization.
pub(super) struct BufferedDeltaFrame {
    pub pair: (u16, u16),
    pub run_lengths: Vec<u16>,
    pub count: u16,
}
