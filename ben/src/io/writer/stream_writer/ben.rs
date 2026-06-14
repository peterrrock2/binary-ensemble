//! Plain-BEN encode logic for the unified stream writer.

use std::collections::HashMap;
use std::io::{self, Write};

use crate::codec::encode::encode_twodelta_frame_with_hint;
use crate::codec::encode::errors::is_twodelta_run_too_long;
use crate::codec::BenEncodeFrame;
use crate::BenVariant;

use super::super::twodelta::{
    classify_transition, pair_has_masks, twodelta_repeat_runs, TransitionKind,
    BEN_TWODELTA_DELTA_TAG, BEN_TWODELTA_SNAPSHOT_TAG, DEFAULT_TWODELTA_SNAPSHOT_INTERVAL,
};

/// State for the BEN arm. Variant lives here as the single source of truth.
pub(super) struct BenState<W: Write> {
    pub(super) writer: W,
    pub(super) variant: BenVariant,
    pub(super) previous_assignment: Vec<u16>,
    pub(super) previous_masks: HashMap<u16, Vec<usize>>,
    pub(super) twodelta_deltas_since_snapshot: usize,
    pub(super) pending_assignment: Option<Vec<u16>>,
    pub(super) pending_count: u16,
}

impl<W: Write> BenState<W> {
    pub(super) fn new(writer: W, variant: BenVariant) -> Self {
        Self {
            writer,
            variant,
            previous_assignment: Vec::new(),
            previous_masks: HashMap::new(),
            twodelta_deltas_since_snapshot: 0,
            pending_assignment: None,
            pending_count: 0,
        }
    }

    /// Encode and write the buffered assignment with the accumulated repetition count. No-op when
    /// nothing is pending.
    pub(super) fn flush_pending_frame(&mut self) -> io::Result<()> {
        let pending = match self.pending_assignment.take() {
            Some(p) => p,
            None => return Ok(()),
        };
        let count = self.pending_count;
        self.pending_count = 0;
        self.encode_and_write_frame(&pending, count)?;
        self.previous_assignment = pending;
        Ok(())
    }

    /// Encode one `(assignment, count)` directly, used for both flush and `write_frame`. Updates
    /// `previous_masks` for TwoDelta.
    fn encode_and_write_frame(&mut self, assignment: &[u16], count: u16) -> io::Result<()> {
        match self.variant {
            BenVariant::Standard => {
                let frame =
                    BenEncodeFrame::from_assignment(assignment, BenVariant::Standard, None)?;
                for _ in 0..count {
                    self.writer.write_all(frame.as_slice())?;
                }
            }
            BenVariant::MkvChain => {
                let frame =
                    BenEncodeFrame::from_assignment(assignment, BenVariant::MkvChain, Some(count))?;
                self.writer.write_all(frame.as_slice())?;
            }
            BenVariant::TwoDelta => {
                if self.previous_assignment.is_empty() {
                    // First frame: a snapshot. Seeds the position masks for subsequent deltas.
                    self.write_twodelta_snapshot(assignment, count)?;
                } else if self.twodelta_deltas_since_snapshot >= DEFAULT_TWODELTA_SNAPSHOT_INTERVAL
                {
                    self.write_twodelta_snapshot(assignment, count)?;
                } else {
                    match classify_transition(&self.previous_assignment, assignment)? {
                        TransitionKind::Repeat => match twodelta_repeat_frame(assignment, count) {
                            Ok(frame) => {
                                self.write_twodelta_delta_frame(frame)?;
                            }
                            // A pair-projected run longer than u16::MAX cannot be expressed in a
                            // delta-shaped frame (splitting it would require zero-length runs,
                            // which readers reject as corruption); a snapshot splits long runs
                            // natively.
                            Err(e) if is_twodelta_run_too_long(&e) => {
                                self.write_twodelta_snapshot(assignment, count)?;
                            }
                            Err(e) => return Err(e),
                        },
                        // Clean 2-swap where both districts already exist: cheap delta against the
                        // maintained masks.
                        TransitionKind::Delta(a, b)
                            if pair_has_masks(&self.previous_masks, a, b) =>
                        {
                            match encode_twodelta_frame_with_hint(
                                &self.previous_assignment,
                                assignment,
                                Some((a, b)),
                                Some(&mut self.previous_masks),
                                Some(count),
                            ) {
                                Ok(frame) => {
                                    self.write_twodelta_delta_frame(frame)?;
                                }
                                // Same representability limit as the repeat arm. The failed
                                // encode leaves `previous_masks` untouched, and the snapshot
                                // reseeds them from `assignment`.
                                Err(e) if is_twodelta_run_too_long(&e) => {
                                    self.write_twodelta_snapshot(assignment, count)?;
                                }
                                Err(e) => return Err(e),
                            }
                        }
                        // A >2-district transition, or a 2-id transition that introduces a district
                        // absent from the previous assignment (no mask to delta against): full
                        // snapshot, then rebuild masks so the next delta has a correct baseline.
                        TransitionKind::Delta(..) | TransitionKind::Snapshot => {
                            self.write_twodelta_snapshot(assignment, count)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn write_twodelta_delta_frame(&mut self, frame: BenEncodeFrame) -> io::Result<()> {
        self.writer.write_all(&[BEN_TWODELTA_DELTA_TAG])?;
        self.writer.write_all(frame.as_slice())?;
        self.twodelta_deltas_since_snapshot += 1;
        Ok(())
    }

    /// Write a snapshot frame (`MkvChain` wire format under the snapshot tag) and (re)seed the
    /// position masks from `assignment` so any following delta frame has a correct baseline.
    fn write_twodelta_snapshot(&mut self, assignment: &[u16], count: u16) -> io::Result<()> {
        self.twodelta_deltas_since_snapshot = 0;
        self.previous_masks.clear();
        for (idx, &val) in assignment.iter().enumerate() {
            self.previous_masks.entry(val).or_default().push(idx);
        }
        let frame = BenEncodeFrame::from_assignment(assignment, BenVariant::MkvChain, Some(count))?;
        self.writer.write_all(&[BEN_TWODELTA_SNAPSHOT_TAG])?;
        self.writer.write_all(frame.as_slice())?;
        Ok(())
    }

    pub(super) fn write_assignment(&mut self, assign_vec: Vec<u16>) -> io::Result<()> {
        if self.pending_assignment.as_deref() == Some(assign_vec.as_slice()) {
            if self.pending_count == u16::MAX {
                self.flush_pending_frame()?;
                self.pending_assignment = Some(assign_vec);
                self.pending_count = 1;
                return Ok(());
            }
            self.pending_count += 1;
            return Ok(());
        }
        self.flush_pending_frame()?;
        self.pending_assignment = Some(assign_vec);
        self.pending_count = 1;
        Ok(())
    }

    /// Encode one frame with the supplied count, flushing any pending merge state first. Caller has
    /// already verified `count != 0` and that the writer is in a valid state.
    pub(super) fn write_frame(&mut self, assignment: Vec<u16>, count: u16) -> io::Result<()> {
        self.flush_pending_frame()?;
        self.encode_and_write_frame(&assignment, count)?;
        // For TwoDelta, the next delta is encoded against the just-emitted frame.
        // `encode_and_write_frame` already updated `previous_masks` (a snapshot reseeds them, a
        // delta maintains them in place); in all variants we update `previous_assignment` here so a
        // subsequent `write_assignment` sees the right baseline.
        self.previous_assignment = assignment;
        Ok(())
    }
}

pub(crate) fn twodelta_repeat_frame(assignment: &[u16], count: u16) -> io::Result<BenEncodeFrame> {
    let (pair, run_lengths) = twodelta_repeat_runs(assignment)?;
    BenEncodeFrame::from_run_lengths(pair, run_lengths, Some(count))
}
