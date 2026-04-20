pub(super) const XBEN_TWODELTA_FULL_TAG: u8 = 0;
pub(super) const XBEN_TWODELTA_CHUNK_TAG: u8 = 2;

pub(super) enum XBenTwoDeltaFrame {
    Full {
        runs: Vec<(u16, u16)>,
    },
    Delta {
        pair: (u16, u16),
        run_lengths: Vec<u16>,
    },
}
