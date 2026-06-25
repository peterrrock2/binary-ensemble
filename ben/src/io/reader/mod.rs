pub mod errors;
mod stream_reader;
pub mod subsample;
#[cfg(test)]
mod tests;
pub(crate) mod twodelta;

pub use errors::DecoderInitError;
pub use stream_reader::{
    BenStreamFrameReader, BenStreamReader, BenWireFormat, TwoDeltaFrameEvent,
    TwoDeltaFrameEventReader,
};
pub use subsample::{
    build_frame_iter, build_frame_iter_from_reader, count_samples_from_file,
    count_samples_from_frame_iter, Ben32Frame, DecodeFrame, FrameIter, MkvRecord, Selection,
    SubsampleFrameDecoder,
};
