use std::io::{self, Error, ErrorKind, Read};

use super::{BenStreamInner, BenStreamReader};
use crate::BenVariant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TwoDeltaFrameEvent {
    Snapshot {
        assignment: Vec<u16>,
        changes: Option<Vec<(u32, u16, u16)>>,
        count: u16,
    },
    Delta {
        changes: Vec<(u32, u16, u16)>,
        count: u16,
    },
}

pub struct TwoDeltaFrameEventReader<R: Read> {
    inner: BenStreamReader<R>,
    errored: bool,
}

impl<R: Read> BenStreamReader<R> {
    pub fn into_twodelta_events(self) -> TwoDeltaFrameEventReader<R> {
        TwoDeltaFrameEventReader::from_stream(self)
    }
}

impl<R: Read> TwoDeltaFrameEventReader<R> {
    pub(super) fn from_stream(inner: BenStreamReader<R>) -> Self {
        Self {
            inner,
            errored: false,
        }
    }
}

pub(super) fn diff_changes(old: &[u16], new: &[u16]) -> Vec<(u32, u16, u16)> {
    old.iter()
        .zip(new.iter())
        .enumerate()
        .filter(|(_, (old_val, new_val))| old_val != new_val)
        .map(|(idx, (old_val, new_val))| (idx as u32, *old_val, *new_val))
        .collect()
}

impl<R: Read> Iterator for TwoDeltaFrameEventReader<R> {
    type Item = io::Result<TwoDeltaFrameEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.inner.variant != BenVariant::TwoDelta {
            if self.errored {
                return None;
            }

            self.errored = true;
            return Some(Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "Attempted iteration on TwoDeltaFrameEventReader with non-TwoDelta stream \
                    found variant: {:?}",
                    self.inner.variant
                ),
            )));
        }

        match self.inner.inner_mut() {
            BenStreamInner::Ben {
                reader,
                previous_assignment,
                twodelta_masks,
                ..
            } => super::ben::next_event_ben(reader, previous_assignment, twodelta_masks),
            BenStreamInner::XBen(inner) => super::xben::next_event_xben(inner),
        }
    }
}
