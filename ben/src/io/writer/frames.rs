use crate::codec::{BenEncodeFrame, TwoDeltaFrame};

/// A buffered delta frame awaiting chunk serialization.
pub(super) struct BufferedDeltaFrame {
    pub pair: (u16, u16),
    pub run_lengths: Vec<u16>,
    pub count: u16,
}

pub(super) enum BufferedBenFrame {
    Ben(BenEncodeFrame),
    TwoDelta(TwoDeltaFrame),
}

impl BufferedBenFrame {
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Ben(frame) => frame.as_slice(),
            Self::TwoDelta(frame) => frame.as_slice(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct AssignmentHints {
    pub is_repeated: bool,
    pub delta_pair: Option<(u16, u16)>,
}
