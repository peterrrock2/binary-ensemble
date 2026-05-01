use binary_ensemble::io::bundle::format::{BendlDirectoryEntry, BendlHeader};
use std::cell::RefCell;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::rc::Rc;

/// Handle to the underlying output file shared between the live
/// `AssignmentWriter` and the `PyBenEncoder` that owns it. Needed so the
/// encoder can reach the buffered file after the inner assignment writer
/// has finished, in order to patch the bundle header and write the
/// trailing directory.
pub(super) type SharedFileSlot = Rc<RefCell<BufWriter<File>>>;

/// Wrapper around a shared buffered file that implements `Write`. The
/// `AssignmentWriter` holds one of these and delegates every write into
/// the shared slot.
pub(super) struct SharedFileWriter(pub SharedFileSlot);

impl Write for SharedFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.borrow_mut().flush()
    }
}

/// Output container produced by `PyBenEncoder`.
pub(super) enum OutputMode {
    /// Plain `.ben` file: just the assignment stream, no header or directory.
    BenOnly,
    /// `.bendl` bundle: provisional header up front, optional graph asset,
    /// then the assignment stream, then a directory written at close time.
    Bundle {
        header: BendlHeader,
        entries: Vec<BendlDirectoryEntry>,
        stream_start: u64,
        sample_count: i64,
    },
}
