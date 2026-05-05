use binary_ensemble::codec::decode::{
    decode_ben_to_jsonl, decode_twodelta_frame, decode_xben_to_ben, decode_xben_to_jsonl,
    xz_decompress,
};
use binary_ensemble::codec::encode::{encode_jsonl_to_xben, xz_compress};
use binary_ensemble::codec::{BenConstruct, MkvBenEncodeFrame, TwoDeltaEncodeFrame};
use binary_ensemble::format::banners::{
    MKVCHAIN_BEN_BANNER, STANDARD_BEN_BANNER, TWODELTA_BEN_BANNER,
};
use binary_ensemble::io::bundle::format::{
    encode_directory, AssignmentFormat, BendlDirectoryEntry, BendlHeader, ASSET_TYPE_CUSTOM,
    ASSET_TYPE_GRAPH, BENDL_MAGIC, BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION, FINALIZED_YES,
    HEADER_SIZE,
};
use binary_ensemble::io::bundle::writer::{
    AddAssetOptions, BendlAppender, BendlTruncate, BendlWriter,
};
use binary_ensemble::io::bundle::BendlReader;
use binary_ensemble::io::reader::{AssignmentReader, XZAssignmentReader};
use binary_ensemble::io::writer::AssignmentWriter;
use binary_ensemble::ops::relabel::relabel_ben_file_with_map;
use binary_ensemble::BenVariant;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

fn expand_ben(bytes: &[u8]) -> Vec<Vec<u16>> {
    AssignmentReader::new(bytes)
        .unwrap()
        .silent(true)
        .flat_map(|record| {
            let (assignment, count) = record.unwrap();
            std::iter::repeat(assignment).take(count as usize)
        })
        .collect()
}

fn minimal_bendl_with_entries(
    entries: Vec<BendlDirectoryEntry>,
    directory_len_adjustment: i64,
) -> Vec<u8> {
    let mut bytes = vec![0u8; HEADER_SIZE];
    let directory_offset = bytes.len() as u64;
    let mut directory = encode_directory(&entries).unwrap();
    if directory_len_adjustment > 0 {
        directory.extend(std::iter::repeat(0u8).take(directory_len_adjustment as usize));
    }
    bytes.extend_from_slice(&directory);

    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset,
        directory_len: (directory.len() as i64 + directory_len_adjustment.min(0)) as u64,
        stream_offset: directory_offset,
        stream_len: 0,
        sample_count: 0,
    };
    bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
    bytes
}

fn expect_bendl_open_err(bytes: Vec<u8>) -> binary_ensemble::io::bundle::format::BendlFormatError {
    match BendlReader::open(Cursor::new(bytes)) {
        Ok(_) => panic!("expected BendlReader::open to fail"),
        Err(err) => err,
    }
}

#[derive(Debug)]
struct CrashState {
    bytes: Vec<u8>,
    pos: u64,
    truncated: bool,
}

#[derive(Debug, Clone)]
struct HeaderPatchCrashCursor {
    state: Rc<RefCell<CrashState>>,
}

impl HeaderPatchCrashCursor {
    fn new(bytes: Vec<u8>) -> (Self, Rc<RefCell<CrashState>>) {
        let state = Rc::new(RefCell::new(CrashState {
            bytes,
            pos: 0,
            truncated: false,
        }));
        (
            Self {
                state: Rc::clone(&state),
            },
            state,
        )
    }
}

impl Read for HeaderPatchCrashCursor {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut state = self.state.borrow_mut();
        let pos = state.pos as usize;
        if pos >= state.bytes.len() {
            return Ok(0);
        }
        let count = buf.len().min(state.bytes.len() - pos);
        buf[..count].copy_from_slice(&state.bytes[pos..pos + count]);
        state.pos += count as u64;
        Ok(count)
    }
}

impl Write for HeaderPatchCrashCursor {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut state = self.state.borrow_mut();
        if state.truncated && state.pos < HEADER_SIZE as u64 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "simulated crash while patching bundle header",
            ));
        }
        let pos = state.pos as usize;
        let end = pos + buf.len();
        if end > state.bytes.len() {
            state.bytes.resize(end, 0);
        }
        state.bytes[pos..end].copy_from_slice(buf);
        state.pos = end as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for HeaderPatchCrashCursor {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let mut state = self.state.borrow_mut();
        let next = match pos {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::End(offset) => state.bytes.len() as i128 + offset as i128,
            SeekFrom::Current(offset) => state.pos as i128 + offset as i128,
        };
        if next < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before start",
            ));
        }
        state.pos = next as u64;
        Ok(state.pos)
    }
}

impl BendlTruncate for HeaderPatchCrashCursor {
    fn truncate_at(&mut self, len: u64) -> std::io::Result<()> {
        let mut state = self.state.borrow_mut();
        state.truncated = true;
        state.bytes.truncate(len as usize);
        if state.pos > len {
            state.pos = len;
        }
        Ok(())
    }
}

fn tiny_bendl_bundle() -> Vec<u8> {
    let mut writer = BendlWriter::new(Cursor::new(Vec::new()), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "base.bin",
            b"base",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    writer
        .write_stream_bytes(b"STANDARD BEN FILE\x00\x01\x02", 1)
        .unwrap();
    writer.finish().unwrap().into_inner()
}

fn assert_ben_bytes_do_not_panic(bytes: Vec<u8>) {
    let outcome = std::panic::catch_unwind(|| {
        if let Ok(reader) = AssignmentReader::new(bytes.as_slice()) {
            for record in reader.silent(true).take(16) {
                let _ = record;
            }
        }
    });
    assert!(outcome.is_ok(), "BEN parser panicked for bytes: {bytes:?}");
}

fn assert_xben_bytes_do_not_panic(bytes: Vec<u8>) {
    let outcome = std::panic::catch_unwind(|| {
        if let Ok(reader) = XZAssignmentReader::new(bytes.as_slice()) {
            for record in reader.silent(true).take(16) {
                let _ = record;
            }
        }
    });
    assert!(outcome.is_ok(), "XBEN parser panicked for bytes: {bytes:?}");
}

#[test]
fn standard_rle_splits_assignment_run_longer_than_u16_max() {
    let assignment = vec![7u16; u16::MAX as usize + 1];
    let mut ben = Vec::new();
    {
        let mut writer = AssignmentWriter::new(&mut ben, BenVariant::Standard).unwrap();
        writer.write_assignment(assignment.clone()).unwrap();
        writer.finish().unwrap();
    }

    let decoded = expand_ben(&ben);
    assert_eq!(decoded, vec![assignment]);
}

#[test]
fn mkvchain_writer_splits_repetition_count_longer_than_u16_max() {
    let sample = vec![1u16, 2, 2, 1];
    let mut ben = Vec::new();
    {
        let mut writer = AssignmentWriter::new(&mut ben, BenVariant::MkvChain).unwrap();
        for _ in 0..(u16::MAX as usize + 1) {
            writer.write_assignment(sample.clone()).unwrap();
        }
        writer.finish().unwrap();
    }

    let mut reader = AssignmentReader::new(ben.as_slice()).unwrap().silent(true);
    let first = reader.next().unwrap().unwrap();
    let second = reader.next().unwrap().unwrap();
    assert!(reader.next().is_none());
    assert_eq!(first, (sample.clone(), u16::MAX));
    assert_eq!(second, (sample, 1));
}

#[test]
fn twodelta_writer_splits_repetition_count_longer_than_u16_max() {
    let sample = vec![1u16, 1, 2, 2];
    let mut ben = Vec::new();
    {
        let mut writer = AssignmentWriter::new(&mut ben, BenVariant::TwoDelta).unwrap();
        for _ in 0..(u16::MAX as usize + 1) {
            writer.write_assignment(sample.clone()).unwrap();
        }
        writer.finish().unwrap();
    }

    let mut total = 0usize;
    let mut unique_frames = 0usize;
    AssignmentReader::new(ben.as_slice())
        .unwrap()
        .silent(true)
        .for_each_assignment(|assignment, count| {
            assert_eq!(assignment, sample.as_slice());
            total += count as usize;
            unique_frames += 1;
            Ok(true)
        })
        .unwrap();
    assert_eq!(total, u16::MAX as usize + 1);
    assert_eq!(unique_frames, 2);
}

#[test]
fn xben_mkvchain_splits_repetition_count_longer_than_u16_max() {
    let mut jsonl = String::new();
    for sample in 1..=(u16::MAX as usize + 1) {
        jsonl.push_str(&format!(r#"{{"assignment":[4,4,5],"sample":{sample}}}"#));
        jsonl.push('\n');
    }

    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        BufReader::new(jsonl.as_bytes()),
        &mut xben,
        BenVariant::MkvChain,
        Some(1),
        Some(0),
        None,
    )
    .unwrap();

    let mut reader = XZAssignmentReader::new(xben.as_slice())
        .unwrap()
        .silent(true);
    assert_eq!(reader.next().unwrap().unwrap(), (vec![4, 4, 5], u16::MAX));
    assert_eq!(reader.next().unwrap().unwrap(), (vec![4, 4, 5], 1));
    assert!(reader.next().is_none());
}

#[test]
fn malformed_ben_bit_widths_return_invalid_data() {
    let mut ben = STANDARD_BEN_BANNER.to_vec();
    ben.extend_from_slice(&[0, 1, 0, 0, 0, 0]);
    let err = AssignmentReader::new(ben.as_slice())
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

    let mut ben = STANDARD_BEN_BANNER.to_vec();
    ben.extend_from_slice(&[17, 1, 0, 0, 0, 0]);
    let err = decode_ben_to_jsonl(ben.as_slice(), Vec::new()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn malformed_twodelta_bit_width_and_extra_runs_return_errors() {
    let anchor = MkvBenEncodeFrame::from_assignment(vec![1u16, 2], Some(1));
    let mut ben = TWODELTA_BEN_BANNER.to_vec();
    ben.extend_from_slice(anchor.as_slice());
    ben.extend_from_slice(&[0, 1, 0, 2, 0, 0, 0, 0, 0, 1]);

    let mut reader = AssignmentReader::new(ben.as_slice()).unwrap().silent(true);
    assert_eq!(reader.next().unwrap().unwrap(), (vec![1, 2], 1));
    let err = reader.next().unwrap().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

    let frame = TwoDeltaEncodeFrame::from_run_lengths((1, 2), vec![1, 1], Some(1));
    let err = decode_twodelta_frame(vec![1u16], &frame).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn direct_xben_helpers_propagate_corrupt_xz_errors() {
    let jsonl = b"{\"assignment\":[1,2,1],\"sample\":1}\n";
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        BufReader::new(jsonl.as_slice()),
        &mut xben,
        BenVariant::Standard,
        Some(1),
        Some(0),
        None,
    )
    .unwrap();
    xben.truncate(xben.len() - 1);

    assert!(decode_xben_to_jsonl(BufReader::new(xben.as_slice()), Vec::new()).is_err());
    assert!(decode_xben_to_ben(BufReader::new(xben.as_slice()), Vec::new()).is_err());
    assert!(xz_decompress(BufReader::new(xben.as_slice()), Vec::new()).is_err());
}

#[test]
fn xz_compress_propagates_input_reader_errors() {
    struct FailingReader;
    impl std::io::Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::Other, "boom"))
        }
    }
    impl std::io::BufRead for FailingReader {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            Err(std::io::Error::new(std::io::ErrorKind::Other, "boom"))
        }
        fn consume(&mut self, _amt: usize) {}
    }

    let err = xz_compress(FailingReader, Vec::new(), Some(1), Some(0)).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
}

#[test]
fn relabel_map_out_of_range_old_indices_error_cleanly() {
    let mut ben = Vec::new();
    {
        let mut writer = AssignmentWriter::new(&mut ben, BenVariant::Standard).unwrap();
        writer.write_assignment(vec![10, 20]).unwrap();
        writer.finish().unwrap();
    }

    let out_of_range_old = HashMap::from([(0usize, 0usize), (1, 2)]);
    let err = relabel_ben_file_with_map(ben.as_slice(), Vec::new(), out_of_range_old).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn bendl_open_rejects_malformed_directory_invariants() {
    let dup_entries = vec![
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: 0,
            name: "dup.bin".to_string(),
            payload_offset: HEADER_SIZE as u64,
            payload_len: 0,
            checksum: None,
        },
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: 0,
            name: "dup.bin".to_string(),
            payload_offset: HEADER_SIZE as u64,
            payload_len: 0,
            checksum: None,
        },
    ];
    let duplicate_bundle = minimal_bendl_with_entries(dup_entries, 0);
    let err = expect_bendl_open_err(duplicate_bundle.clone());
    assert!(err.to_string().contains("malformed directory"));
    assert!(BendlAppender::open(Cursor::new(duplicate_bundle)).is_err());

    let wrong_canonical = vec![BendlDirectoryEntry {
        asset_type: ASSET_TYPE_GRAPH,
        asset_flags: 0,
        name: "not_graph.json".to_string(),
        payload_offset: HEADER_SIZE as u64,
        payload_len: 0,
        checksum: None,
    }];
    let err = expect_bendl_open_err(minimal_bendl_with_entries(wrong_canonical, 0));
    assert!(err.to_string().contains("malformed directory"));
}

#[test]
fn bendl_open_rejects_directory_len_mismatches() {
    let entries = vec![BendlDirectoryEntry {
        asset_type: ASSET_TYPE_CUSTOM,
        asset_flags: 0,
        name: "ok.bin".to_string(),
        payload_offset: HEADER_SIZE as u64,
        payload_len: 0,
        checksum: None,
    }];

    let trailing = minimal_bendl_with_entries(entries.clone(), 1);
    let err = expect_bendl_open_err(trailing);
    assert!(err.to_string().contains("trailing"));

    let too_short = minimal_bendl_with_entries(entries, -1);
    let err = expect_bendl_open_err(too_short);
    assert!(matches!(
        err,
        binary_ensemble::io::bundle::format::BendlFormatError::Io(_)
    ));
}

#[test]
fn xben_twodelta_huge_incomplete_chunk_errors_without_panicking() {
    let mut inner = TWODELTA_BEN_BANNER.to_vec();
    inner.push(2); // XBEN_TWODELTA_CHUNK_TAG
    inner.extend_from_slice(&u32::MAX.to_be_bytes());

    let mut xben = Vec::new();
    xz_compress(
        BufReader::new(inner.as_slice()),
        &mut xben,
        Some(1),
        Some(0),
    )
    .unwrap();

    let mut reader = XZAssignmentReader::new(xben.as_slice()).unwrap();
    let err = reader.next().unwrap().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn zero_count_frames_are_rejected() {
    let frame = MkvBenEncodeFrame::from_assignment(vec![1u16], Some(0));
    let mut ben = MKVCHAIN_BEN_BANNER.to_vec();
    ben.extend_from_slice(frame.as_slice());
    let err = AssignmentReader::new(ben.as_slice())
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

    let mut inner = MKVCHAIN_BEN_BANNER.to_vec();
    inner.extend_from_slice(&(1u32 << 16 | 1).to_be_bytes());
    inner.extend_from_slice(&[0, 0, 0, 0]);
    inner.extend_from_slice(&0u16.to_be_bytes());
    let mut xben = Vec::new();
    xz_compress(
        BufReader::new(inner.as_slice()),
        &mut xben,
        Some(1),
        Some(0),
    )
    .unwrap();
    let err = XZAssignmentReader::new(xben.as_slice())
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn seeded_malformed_ben_bytes_do_not_panic() {
    let mut valid_standard = Vec::new();
    {
        let mut writer = AssignmentWriter::new(&mut valid_standard, BenVariant::Standard).unwrap();
        writer.write_assignment(vec![1, 1, 2, 3]).unwrap();
        writer.write_assignment(vec![3, 3, 2, 1]).unwrap();
        writer.finish().unwrap();
    }

    let mut valid_mkv = Vec::new();
    {
        let mut writer = AssignmentWriter::new(&mut valid_mkv, BenVariant::MkvChain).unwrap();
        writer.write_assignment(vec![4, 4, 5]).unwrap();
        writer.write_assignment(vec![4, 4, 5]).unwrap();
        writer.write_assignment(vec![5, 4, 4]).unwrap();
        writer.finish().unwrap();
    }

    let mut valid_twodelta = Vec::new();
    {
        let mut writer = AssignmentWriter::new(&mut valid_twodelta, BenVariant::TwoDelta).unwrap();
        writer.write_assignment(vec![1, 1, 2, 2]).unwrap();
        writer.write_assignment(vec![1, 2, 1, 2]).unwrap();
        writer.write_assignment(vec![2, 2, 1, 1]).unwrap();
        writer.finish().unwrap();
    }

    for seed in [valid_standard, valid_mkv, valid_twodelta] {
        for len in 0..=seed.len() {
            assert_ben_bytes_do_not_panic(seed[..len].to_vec());
        }

        for idx in 0..seed.len() {
            let mut mutated = seed.clone();
            mutated[idx] ^= 0xA5;
            assert_ben_bytes_do_not_panic(mutated);
        }

        if seed.len() >= STANDARD_BEN_BANNER.len() + 6 {
            let mut inflated_frame_len = seed.clone();
            let start = STANDARD_BEN_BANNER.len() + 2;
            inflated_frame_len[start..start + 4].copy_from_slice(&1024u32.to_be_bytes());
            assert_ben_bytes_do_not_panic(inflated_frame_len);
        }
    }
}

#[test]
fn seeded_malformed_xben_bytes_do_not_panic() {
    let jsonl =
        b"{\"assignment\":[1,1,2,2],\"sample\":1}\n{\"assignment\":[1,2,1,2],\"sample\":2}\n";
    let mut seeds = Vec::new();
    for variant in [
        BenVariant::Standard,
        BenVariant::MkvChain,
        BenVariant::TwoDelta,
    ] {
        let mut xben = Vec::new();
        encode_jsonl_to_xben(
            BufReader::new(jsonl.as_slice()),
            &mut xben,
            variant,
            Some(1),
            Some(0),
            Some(32),
        )
        .unwrap();
        seeds.push(xben);
    }

    for seed in seeds {
        for len in 0..=seed.len() {
            assert_xben_bytes_do_not_panic(seed[..len].to_vec());
        }

        for idx in (0..seed.len()).step_by(3) {
            let mut mutated = seed.clone();
            mutated[idx] ^= 0x5A;
            assert_xben_bytes_do_not_panic(mutated);
        }
    }

    let mut unknown_tag_inner = STANDARD_BEN_BANNER.to_vec();
    unknown_tag_inner.push(0xFF);
    let mut unknown_tag_xben = Vec::new();
    xz_compress(
        BufReader::new(unknown_tag_inner.as_slice()),
        &mut unknown_tag_xben,
        Some(1),
        Some(0),
    )
    .unwrap();
    assert_xben_bytes_do_not_panic(unknown_tag_xben);
}

#[test]
fn bendl_append_header_patch_crash_is_rejected_on_reopen() {
    let base = tiny_bendl_bundle();
    assert!(BendlReader::open(Cursor::new(base.clone())).is_ok());

    let (cursor, state) = HeaderPatchCrashCursor::new(base);
    let mut appender = BendlAppender::open(cursor).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "after-crash.bin",
            b"payload written before header patch",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();

    let err = match appender.commit() {
        Ok(_) => panic!("expected simulated header patch crash"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("simulated crash"));

    let damaged = state.borrow().bytes.clone();
    assert!(BendlReader::open(Cursor::new(damaged.clone())).is_err());
    assert!(BendlAppender::open(Cursor::new(damaged)).is_err());
}

#[test]
fn bendl_append_truncated_new_directory_is_rejected_on_reopen() {
    let base = tiny_bendl_bundle();
    let mut appender = BendlAppender::open(Cursor::new(base)).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "complete-append.bin",
            b"payload",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let mut appended = appender.commit().unwrap().into_inner();
    assert!(BendlReader::open(Cursor::new(appended.clone())).is_ok());

    appended.pop();
    let err = expect_bendl_open_err(appended);
    assert!(err.to_string().contains("IO error"));
}
