use binary_ensemble::codec::decode::{
    decode_ben_to_jsonl, decode_twodelta_frame, decode_xben_to_ben, decode_xben_to_jsonl,
    xz_decompress,
};
use binary_ensemble::codec::encode::{encode_jsonl_to_xben, xz_compress};
use binary_ensemble::codec::BenEncodeFrame;
use binary_ensemble::format::banners::{
    MKVCHAIN_BEN_BANNER, STANDARD_BEN_BANNER, TWODELTA_BEN_BANNER,
};
use binary_ensemble::io::bundle::format::{
    encode_directory, AssignmentFormat, BendlDirectoryEntry, BendlFormatError, BendlHeader,
    ASSET_TYPE_CUSTOM, ASSET_TYPE_GRAPH, BENDL_MAGIC, BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION,
    FINALIZED_YES, HEADER_FLAG_STREAM_CHECKSUM, HEADER_SIZE,
};
use binary_ensemble::io::bundle::writer::{AddAssetOptions, BendlAppender, BendlWriter};
use binary_ensemble::io::bundle::{BendlReadError, BendlReader, ChecksumError, ChecksumTarget};
use binary_ensemble::io::reader::BenStreamReader;
use binary_ensemble::io::writer::BenStreamWriter;
use binary_ensemble::ops::relabel::{relabel_ben_file, RelabelOptions};
use binary_ensemble::test_utils::{BendlBytes, DirectoryEntryField, HeaderField};
use binary_ensemble::BenVariant;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

fn expand_ben(bytes: &[u8]) -> Vec<Vec<u16>> {
    BenStreamReader::from_ben(bytes)
        .unwrap()
        .silent(true)
        .flat_map(|record| {
            let (assignment, count) = record.unwrap();
            std::iter::repeat_n(assignment, count as usize)
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
        directory.extend(std::iter::repeat_n(0u8, directory_len_adjustment as usize));
    }
    bytes.extend_from_slice(&directory);

    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        alignment_padding: 0,
        flags: 0,
        stream_checksum: 0,
        directory_offset,
        directory_len: (directory.len() as i64 + directory_len_adjustment.min(0)) as u64,
        stream_offset: directory_offset,
        stream_len: 0,
        sample_count: 0,
    };
    bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
    bytes
}

fn expect_bendl_open_err(
    bytes: impl Into<Vec<u8>>,
) -> binary_ensemble::io::bundle::format::BendlFormatError {
    match BendlReader::open(Cursor::new(bytes.into())) {
        Ok(_) => panic!("expected BendlReader::open to fail"),
        Err(err) => err,
    }
}

#[derive(Debug)]
struct CrashState {
    bytes: Vec<u8>,
    pos: u64,
    initial_len: usize,
}

#[derive(Debug, Clone)]
struct HeaderPatchCrashCursor {
    state: Rc<RefCell<CrashState>>,
}

impl HeaderPatchCrashCursor {
    fn new(bytes: Vec<u8>) -> (Self, Rc<RefCell<CrashState>>) {
        let state = Rc::new(RefCell::new(CrashState {
            initial_len: bytes.len(),
            bytes,
            pos: 0,
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
        if state.bytes.len() > state.initial_len && state.pos < HEADER_SIZE as u64 {
            return Err(std::io::Error::other(
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
    let mut session = writer.into_stream_session().unwrap();
    session.write_all(b"STANDARD BEN FILE\x00\x01\x02").unwrap();
    let writer = session.finish_into_writer(1);
    writer.finish().unwrap().into_inner()
}

fn assert_ben_bytes_do_not_panic(bytes: Vec<u8>) {
    let outcome = std::panic::catch_unwind(|| {
        if let Ok(reader) = BenStreamReader::from_ben(bytes.as_slice()) {
            for record in reader.silent(true).take(16) {
                let _ = record;
            }
        }
    });
    assert!(outcome.is_ok(), "BEN parser panicked for bytes: {bytes:?}");
}

fn assert_xben_bytes_do_not_panic(bytes: Vec<u8>) {
    let outcome = std::panic::catch_unwind(|| {
        if let Ok(reader) = BenStreamReader::from_xben(bytes.as_slice()) {
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
        let mut writer = BenStreamWriter::for_ben(&mut ben, BenVariant::Standard).unwrap();
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
        let mut writer = BenStreamWriter::for_ben(&mut ben, BenVariant::MkvChain).unwrap();
        for _ in 0..(u16::MAX as usize + 1) {
            writer.write_assignment(sample.clone()).unwrap();
        }
        writer.finish().unwrap();
    }

    let mut reader = BenStreamReader::from_ben(ben.as_slice())
        .unwrap()
        .silent(true);
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
        let mut writer = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        for _ in 0..(u16::MAX as usize + 1) {
            writer.write_assignment(sample.clone()).unwrap();
        }
        writer.finish().unwrap();
    }

    let mut total = 0usize;
    let mut unique_frames = 0usize;
    BenStreamReader::from_ben(ben.as_slice())
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
        None,
    )
    .unwrap();

    let mut reader = BenStreamReader::from_xben(xben.as_slice())
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
    let err = BenStreamReader::from_ben(ben.as_slice())
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
    let anchor = BenEncodeFrame::from_assignment(vec![1u16, 2], BenVariant::MkvChain, Some(1));
    let mut ben = TWODELTA_BEN_BANNER.to_vec();
    ben.extend_from_slice(anchor.as_slice());
    ben.extend_from_slice(&[0, 1, 0, 2, 0, 0, 0, 0, 0, 1]);

    let mut reader = BenStreamReader::from_ben(ben.as_slice())
        .unwrap()
        .silent(true);
    assert_eq!(reader.next().unwrap().unwrap(), (vec![1, 2], 1));
    let err = reader.next().unwrap().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

    let frame = BenEncodeFrame::from_run_lengths((1, 2), vec![1, 1], Some(1));
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
            Err(std::io::Error::other("boom"))
        }
    }
    impl std::io::BufRead for FailingReader {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            Err(std::io::Error::other("boom"))
        }
        fn consume(&mut self, _amt: usize) {}
    }

    let err = xz_compress(FailingReader, Vec::new(), Some(1), Some(0), None).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
}

#[test]
fn relabel_map_out_of_range_old_indices_error_cleanly() {
    let mut ben = Vec::new();
    {
        let mut writer = BenStreamWriter::for_ben(&mut ben, BenVariant::Standard).unwrap();
        writer.write_assignment(vec![10, 20]).unwrap();
        writer.finish().unwrap();
    }

    let out_of_range_old = HashMap::from([(0usize, 0usize), (1, 2)]);
    let err = relabel_ben_file(
        ben.as_slice(),
        Vec::new(),
        RelabelOptions::node_permutation(out_of_range_old),
    )
    .unwrap_err();
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
        None,
    )
    .unwrap();

    let mut reader = BenStreamReader::from_xben(xben.as_slice()).unwrap();
    let err = reader.next().unwrap().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn zero_count_frames_are_rejected() {
    let frame = BenEncodeFrame::from_assignment(vec![1u16], BenVariant::MkvChain, Some(0));
    let mut ben = MKVCHAIN_BEN_BANNER.to_vec();
    ben.extend_from_slice(frame.as_slice());
    let err = BenStreamReader::from_ben(ben.as_slice())
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
        None,
    )
    .unwrap();
    let err = BenStreamReader::from_xben(xben.as_slice())
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
        let mut writer =
            BenStreamWriter::for_ben(&mut valid_standard, BenVariant::Standard).unwrap();
        writer.write_assignment(vec![1, 1, 2, 3]).unwrap();
        writer.write_assignment(vec![3, 3, 2, 1]).unwrap();
        writer.finish().unwrap();
    }

    let mut valid_mkv = Vec::new();
    {
        let mut writer = BenStreamWriter::for_ben(&mut valid_mkv, BenVariant::MkvChain).unwrap();
        writer.write_assignment(vec![4, 4, 5]).unwrap();
        writer.write_assignment(vec![4, 4, 5]).unwrap();
        writer.write_assignment(vec![5, 4, 4]).unwrap();
        writer.finish().unwrap();
    }

    let mut valid_twodelta = Vec::new();
    {
        let mut writer =
            BenStreamWriter::for_ben(&mut valid_twodelta, BenVariant::TwoDelta).unwrap();
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

    // Explicit no-panic seed: misaligned frame size. mvb=8, mlb=8 means a pair is 2 bytes; this
    // frame claims n_bytes=3 with 2 real-pair bytes plus 1 phantom byte. The decoder must reject
    // (InvalidData) without panicking.
    let mut misaligned_standard = STANDARD_BEN_BANNER.to_vec();
    misaligned_standard.extend_from_slice(&[8u8, 8, 0, 0, 0, 3, 0x01, 0x03, 0xff]);
    assert_ben_bytes_do_not_panic(misaligned_standard);

    // Explicit no-panic seed: interior zero-length run. mvb=4, mlb=4 → 1 pair per byte. Byte 1 =
    // (val=1, len=0) (zero-length pair), byte 2 = (val=2, len=3) (real pair). The decoder must
    // reject (InvalidData) for the interior zero-length run without panicking.
    let mut interior_zero_standard = STANDARD_BEN_BANNER.to_vec();
    interior_zero_standard.extend_from_slice(&[4u8, 4, 0, 0, 0, 2, 0x10, 0x23]);
    assert_ben_bytes_do_not_panic(interior_zero_standard);
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
            None,
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
        None,
    )
    .unwrap();
    assert_xben_bytes_do_not_panic(unknown_tag_xben);
}

#[test]
fn bendl_append_header_patch_crash_preserves_old_directory() {
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
    let reader = BendlReader::open(Cursor::new(damaged.clone())).unwrap();
    assert!(reader.find_asset_by_name("base.bin").is_some());
    assert!(reader.find_asset_by_name("after-crash.bin").is_none());
    assert!(BendlAppender::open(Cursor::new(damaged)).is_ok());
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

// =====================================================================
// BENDL adversarial-bytes fuzz
// =====================================================================

/// Mint a valid BENDL bundle that exercises every public surface the no-panic harness will drive:
/// a finalized header with `HEADER_FLAG_STREAM_CHECKSUM`, an xz-compressed graph asset, a raw JSON
/// metadata asset, a raw custom asset, and an XBEN assignment stream. This is the seed used by
/// `seeded_malformed_bendl_bytes_do_not_panic`.
fn valid_bendl_seed() -> Vec<u8> {
    let mut writer = BendlWriter::new(Cursor::new(Vec::new()), AssignmentFormat::Xben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_GRAPH,
            "graph.json",
            br#"{"nodes":4,"edges":[[0,1],[1,2],[2,3]]}"#,
            AddAssetOptions::defaults().json().compress(),
        )
        .unwrap();
    writer
        .add_asset(
            binary_ensemble::io::bundle::format::ASSET_TYPE_METADATA,
            "metadata.json",
            br#"{"variant":"standard","bundle_version":1}"#,
            AddAssetOptions::defaults().json().raw(),
        )
        .unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "extra.bin",
            b"trailing custom asset",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();

    let mut session = writer.into_stream_session().unwrap();
    encode_jsonl_to_xben(
        Cursor::new(
            b"{\"assignment\":[1,1,2,2],\"sample\":1}\n{\"assignment\":[1,2,1,2],\"sample\":2}\n"
                .as_slice(),
        ),
        &mut session,
        BenVariant::Standard,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap();
    let writer = session.finish_into_writer(2);
    writer.finish().unwrap().into_inner()
}

/// Open the bundle and drive every public read accessor. Any panic from any reader path fails the
/// test loudly. Errors are expected (the input is adversarial) and are silently discarded; only
/// panics matter here.
fn assert_bendl_bytes_do_not_panic(bytes: impl Into<Vec<u8>>) {
    let bytes = bytes.into();
    let outcome = std::panic::catch_unwind(|| {
        let mut reader = match BendlReader::open(Cursor::new(bytes)) {
            Ok(r) => r,
            Err(_) => return,
        };

        // Header / sample-count getters never read further bytes; they should always be safe.
        let _ = reader.is_finalized();
        let _ = reader.sample_count();
        let _ = reader.assignment_format();

        // Stream range computation; may seek to EOF on unfinalized bundles but never panics.
        let _ = reader.assignment_stream_range();

        // Drive each asset accessor with a bounded read so a wildly inflated payload_len cannot
        // OOM the test process. We cap at 1 MiB per asset; legitimate fixtures here are well
        // under that.
        let entries: Vec<_> = reader.assets().to_vec();
        for entry in &entries {
            if let Ok(mut r) = reader.asset_reader(entry) {
                let mut buf = [0u8; 1024];
                for _ in 0..1024 {
                    match r.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            }
            if let Ok(mut r) = reader.asset_reader_unverified(entry) {
                let mut buf = [0u8; 1024];
                for _ in 0..1024 {
                    match r.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            }
            if let Ok(mut r) = reader.asset_payload_reader_unverified(entry) {
                let mut buf = [0u8; 1024];
                for _ in 0..1024 {
                    match r.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            }
            let _ = reader.verify_asset_checksum(entry);
        }

        // verify_all_asset_checksums short-circuits on first mismatch; bounded by directory size.
        let _ = reader.verify_all_asset_checksums();

        // Stream accessors. The verified raw path may surface ChecksumError; that's fine — we
        // only care about absence of panics.
        if let Ok(mut r) = reader.assignment_stream_reader() {
            let mut buf = [0u8; 1024];
            for _ in 0..1024 {
                match r.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
        if let Ok(mut r) = reader.assignment_stream_reader_unverified() {
            let mut buf = [0u8; 1024];
            for _ in 0..1024 {
                match r.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
        if let Ok(decoded) = reader.open_assignment_reader() {
            // Take a bounded prefix of the iterator to avoid spinning on adversarial frame
            // counts. The frame-payload cap in decode_ben_line already bounds per-frame work.
            for record in decoded.silent(true).take(64) {
                let _ = record;
            }
        }
        let _ = reader.verify_stream_checksum();
    });
    assert!(
        outcome.is_ok(),
        "BENDL reader panicked on adversarial input"
    );
}

/// Sanity cap used when fuzzing length fields. Inflating to `u32::MAX` would turn this into an OOM
/// stress test; legitimate fixtures here are kilobytes at most. The cap is large enough that the
/// inflated frames still exercise "value far past end of input" paths, but small enough that the
/// resulting allocations are negligible.
const ADVERSARIAL_LEN_CAP: u32 = 1 << 16; // 64 KiB

#[test]
fn seeded_malformed_bendl_bytes_do_not_panic() {
    let seed = valid_bendl_seed();

    // Truncation at every length, including zero and full size.
    for len in 0..=seed.len() {
        assert_bendl_bytes_do_not_panic(seed[..len].to_vec());
    }

    // Single-byte XOR mutations everywhere. This covers every byte of the header, directory, and
    // payload regions — the same coverage pattern the BEN/XBEN fuzz tests use.
    for idx in 0..seed.len() {
        let mut mutated = seed.clone();
        mutated[idx] ^= 0xA5;
        assert_bendl_bytes_do_not_panic(mutated);
    }

    // Header length-field inflation. Each fixture patches one named header field; the capped
    // values keep the "value far past end of input" paths reachable without turning this into an
    // OOM stress test.
    let inflate_header = |field: HeaderField, value: u64| {
        BendlBytes::new(seed.clone()).with_header_u64(field, value)
    };

    // directory_offset past EOF.
    assert_bendl_bytes_do_not_panic(inflate_header(HeaderField::DirectoryOffset, u64::MAX));
    // directory_len past EOF (capped to avoid OOM if the implementation pre-allocates).
    assert_bendl_bytes_do_not_panic(inflate_header(
        HeaderField::DirectoryLen,
        ADVERSARIAL_LEN_CAP as u64,
    ));
    // stream_offset past EOF.
    assert_bendl_bytes_do_not_panic(inflate_header(HeaderField::StreamOffset, u64::MAX));
    // stream_len past EOF (capped).
    assert_bendl_bytes_do_not_panic(inflate_header(
        HeaderField::StreamLen,
        ADVERSARIAL_LEN_CAP as u64,
    ));
    // stream_offset + stream_len overflowing u64.
    assert_bendl_bytes_do_not_panic(
        BendlBytes::new(seed.clone())
            .with_header_u64(HeaderField::StreamOffset, u64::MAX - 1)
            .with_header_u64(HeaderField::StreamLen, u64::MAX),
    );
    // Reserved header flag bits set.
    assert_bendl_bytes_do_not_panic(inflate_header(HeaderField::Flags, u32::MAX as u64));
    // Non-zero alignment_padding (writers zero it; readers must ignore non-zero bytes there).
    assert_bendl_bytes_do_not_panic(inflate_header(
        HeaderField::AlignmentPadding,
        u16::MAX as u64,
    ));

    // Directory-entry length-field inflation: walk each entry and inflate its per-entry length
    // fields one at a time, plus an inflated entry count.
    let entry_count = BendlBytes::new(seed.clone()).entry_count();
    assert!(entry_count > 0, "valid_bendl_seed must contain entries");

    // entry_count inflation (capped to keep test runtime bounded — the reader must not try to
    // pre-allocate a Vec with u32::MAX capacity, but we don't want to find out the hard way here).
    assert_bendl_bytes_do_not_panic(
        BendlBytes::new(seed.clone()).with_entry_count(ADVERSARIAL_LEN_CAP),
    );

    for index in 0..entry_count as usize {
        let inflate_entry = |field: DirectoryEntryField, value: u64| {
            BendlBytes::new(seed.clone()).with_directory_entry_field(index, field, value)
        };

        // name_len inflation (capped).
        assert_bendl_bytes_do_not_panic(inflate_entry(
            DirectoryEntryField::NameLen,
            ADVERSARIAL_LEN_CAP as u64,
        ));
        // checksum_len inflation (capped).
        assert_bendl_bytes_do_not_panic(inflate_entry(
            DirectoryEntryField::ChecksumLen,
            ADVERSARIAL_LEN_CAP as u64,
        ));
        // payload_len inflation to u64::MAX. ExactLen at read time, plus the per-frame decode cap,
        // prevent any actual allocation.
        assert_bendl_bytes_do_not_panic(inflate_entry(DirectoryEntryField::PayloadLen, u64::MAX));
        // payload_offset past EOF.
        assert_bendl_bytes_do_not_panic(inflate_entry(
            DirectoryEntryField::PayloadOffset,
            u64::MAX,
        ));
    }
}

// =====================================================================
// Open-rejected variant-pinning. Each fixture must fail BendlReader::open
// with a specific BendlFormatError variant, not just an unspecified Err.
// =====================================================================

#[test]
fn bendl_open_rejects_directory_offset_past_eof() {
    // directory_offset claims a position well past the actual file. Cursor seek succeeds (its
    // position is u64) but the subsequent read returns Ok(0); read_directory's read_exact for the
    // entry count fails with UnexpectedEof, which becomes BendlFormatError::Io.
    let seed = valid_bendl_seed();
    let past_eof = seed.len() as u64 + 4096;
    let bytes = BendlBytes::new(seed).with_header_u64(HeaderField::DirectoryOffset, past_eof);
    let err = expect_bendl_open_err(bytes);
    assert!(
        matches!(err, BendlFormatError::Io(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof),
        "expected BendlFormatError::Io(UnexpectedEof), got {err:?}"
    );
}

#[test]
fn bendl_open_rejects_directory_offset_plus_directory_len_overflow() {
    // directory_offset + directory_len overflows u64. The reader has no chance to read anything at
    // u64::MAX - 4; the failure surface is the same UnexpectedEof from the bounded read attempt.
    let bytes = BendlBytes::new(valid_bendl_seed())
        .with_header_u64(HeaderField::DirectoryOffset, u64::MAX - 4)
        .with_header_u64(HeaderField::DirectoryLen, 100);
    let err = expect_bendl_open_err(bytes);
    assert!(
        matches!(err, BendlFormatError::Io(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof),
        "expected BendlFormatError::Io(UnexpectedEof), got {err:?}"
    );
}

#[test]
fn bendl_open_rejects_name_len_longer_than_remaining_directory_bytes() {
    // Build a one-entry directory by hand whose name_len field claims more bytes than the
    // directory range provides. The bounded Take in BendlReader::open prevents the read from
    // escaping into the asset region; read_exact for the name buffer then fails inside the bound.
    let entries = vec![BendlDirectoryEntry {
        asset_type: ASSET_TYPE_CUSTOM,
        asset_flags: 0,
        name: "ab".to_string(),
        payload_offset: HEADER_SIZE as u64,
        payload_len: 0,
        checksum: None,
    }];
    // Patch the sole entry's name_len from 2 to a huge value that exceeds the directory's declared
    // length, so read_exact for the name buffer fails inside the bounded directory region.
    let bytes = BendlBytes::new(minimal_bendl_with_entries(entries, 0)).with_directory_entry_field(
        0,
        DirectoryEntryField::NameLen,
        u16::MAX as u64,
    );

    let err = expect_bendl_open_err(bytes);
    assert!(
        matches!(err, BendlFormatError::Io(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof),
        "expected BendlFormatError::Io(UnexpectedEof), got {err:?}"
    );
}

// =====================================================================
// Openable behavioral pins. Each fixture must let BendlReader::open succeed and then
// surface the documented behavior through the accessors.
// =====================================================================

#[test]
fn bendl_unknown_header_flag_bits_are_ignored() {
    // Forward-compat contract: bits 1..31 of `flags` are reserved. Setting them on a finalized
    // bundle must not change anything observable — open succeeds, directory entries are intact,
    // verify_stream_checksum passes, asset access works.
    let seed = BendlBytes::new(valid_bendl_seed());
    let original_flags = seed.header_u64(HeaderField::Flags) as u32;
    assert!(
        original_flags & HEADER_FLAG_STREAM_CHECKSUM != 0,
        "seed must have STREAM_CHECKSUM set; otherwise this test is testing the wrong contract"
    );
    let polluted_flags = original_flags | (1u32 << 5) | (1u32 << 31);
    let bytes = seed.with_header_u64(HeaderField::Flags, polluted_flags as u64);

    let mut reader = BendlReader::open(Cursor::new(bytes.into_bytes()))
        .expect("unknown flag bits must not block open");
    assert!(reader.is_finalized());
    assert_eq!(
        reader.assets().len(),
        3,
        "all three seed assets must be present"
    );

    // Stream CRC must still pass — the verifier doesn't inspect reserved bits.
    reader
        .verify_stream_checksum()
        .expect("stream CRC must still verify with unknown flag bits set");

    // Asset access must work for both compressed and uncompressed entries.
    for entry in reader.assets().to_vec() {
        reader.asset_bytes(&entry).unwrap_or_else(|e| {
            panic!(
                "asset {} read failed with unknown flags set: {e:?}",
                entry.name
            )
        });
    }
}

#[test]
fn bendl_clear_stream_checksum_flag_with_nonzero_bytes_returns_unavailable_not_mismatch() {
    // Plan-mandated contract: when HEADER_FLAG_STREAM_CHECKSUM is clear, verified stream APIs
    // must return Unavailable regardless of what's in bytes 20..24. Pin this by clearing the flag
    // but leaving non-zero garbage in the stream_checksum slot — a buggy reader that interpreted
    // bytes 20..24 unconditionally would return Mismatch (since the garbage would not match the
    // actual CRC).
    let seed = BendlBytes::new(valid_bendl_seed());
    let cleared_flags = seed.header_u64(HeaderField::Flags) as u32 & !HEADER_FLAG_STREAM_CHECKSUM;
    let bytes = seed
        .with_header_u64(HeaderField::Flags, cleared_flags as u64)
        .with_header_u64(HeaderField::StreamChecksum, 0xDEADBEEF);

    let mut reader = BendlReader::open(Cursor::new(bytes.into_bytes())).expect("open must succeed");

    let expect_unavailable = |result: Result<_, BendlReadError>| match result {
        Err(BendlReadError::Checksum(ChecksumError::Unavailable {
            target: ChecksumTarget::Stream,
        })) => {}
        Err(other) => panic!("expected Unavailable(Stream), got {other:?}"),
        Ok(_) => panic!("expected Err, got Ok"),
    };

    expect_unavailable(reader.assignment_stream_reader().map(|_| ()));
    expect_unavailable(reader.open_assignment_reader().map(|_| ()));
    expect_unavailable(reader.verify_stream_checksum());

    // Asset access is an independent checksum domain and must still verify normally.
    for entry in reader.assets().to_vec() {
        reader
            .asset_bytes(&entry)
            .unwrap_or_else(|e| panic!("asset {} read failed: {e:?}", entry.name));
    }
}

#[test]
fn bendl_nonzero_alignment_padding_is_ignored() {
    // alignment_padding occupies bytes 14..16. Writers zero it; readers must ignore non-zero bytes
    // there. Forward-compat insurance: a future writer that accidentally stamps something into the
    // padding region must not break readers.
    let bytes = BendlBytes::new(valid_bendl_seed())
        .with_header_u64(HeaderField::AlignmentPadding, u16::MAX as u64);

    let mut reader = BendlReader::open(Cursor::new(bytes.into_bytes()))
        .expect("non-zero alignment_padding must not block open");
    reader
        .verify_stream_checksum()
        .expect("stream CRC must still verify with non-zero alignment_padding");
    for entry in reader.assets().to_vec() {
        reader
            .asset_bytes(&entry)
            .unwrap_or_else(|e| panic!("asset {} read failed: {e:?}", entry.name));
    }
}

#[test]
fn bendl_stream_offset_plus_stream_len_overflow_surfaces_short_range() {
    // stream_offset + stream_len overflows u64. BendlReader::open does not validate stream range
    // (intentional — keeps metadata inspection cheap), so open succeeds. Each accessor must
    // surface the strict-EOF contract: the verified raw stream reader returns UnexpectedEof from
    // read; verify_stream_checksum returns BendlReadError::Io(UnexpectedEof);
    // open_assignment_reader either fails at construction or surfaces UnexpectedEof during
    // iteration; assignment_stream_reader_unverified surfaces UnexpectedEof on read.
    let bytes = BendlBytes::new(valid_bendl_seed())
        .with_header_u64(HeaderField::StreamOffset, u64::MAX - 5)
        .with_header_u64(HeaderField::StreamLen, u64::MAX);

    let mut reader = BendlReader::open(Cursor::new(bytes.into_bytes())).expect("open must succeed");

    match reader.verify_stream_checksum() {
        Err(BendlReadError::Io(ref e)) => assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof),
        other => panic!("expected BendlReadError::Io(UnexpectedEof), got {other:?}"),
    }

    let mut buf = [0u8; 64];
    let mut raw = reader
        .assignment_stream_reader()
        .expect("constructor must succeed");
    let err = raw.read(&mut buf).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    drop(raw);

    let mut raw_unverified = reader
        .assignment_stream_reader_unverified()
        .expect("constructor must succeed");
    let err = raw_unverified.read(&mut buf).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    drop(raw_unverified);

    // open_assignment_reader either fails at construction (banner read into short range) or fails
    // during iteration. Both surfaces are acceptable per the 0.1c contract; both must be Io-not-
    // Decode.
    match reader.open_assignment_reader() {
        Err(BendlReadError::Io(ref e)) => assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof),
        Err(other) => panic!("expected Io(UnexpectedEof) at construction, got {other:?}"),
        Ok(mut decoded) => {
            let mut saw_unexpected_eof = false;
            for record in (&mut decoded).take(64) {
                if let Err(e) = record {
                    assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
                    saw_unexpected_eof = true;
                    break;
                }
            }
            assert!(
                saw_unexpected_eof,
                "decoded iterator must surface UnexpectedEof"
            );
        }
    }
}

#[test]
fn bendl_stream_offset_past_eof_surfaces_short_range() {
    // stream_offset alone points past EOF. Same surface contract as the overflow case — open
    // succeeds; every stream accessor reports UnexpectedEof on read.
    let seed = valid_bendl_seed();
    let past_eof = seed.len() as u64 + 4096;
    let bytes = BendlBytes::new(seed).with_header_u64(HeaderField::StreamOffset, past_eof);

    let mut reader = BendlReader::open(Cursor::new(bytes.into_bytes())).expect("open must succeed");

    match reader.verify_stream_checksum() {
        Err(BendlReadError::Io(ref e)) => assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof),
        other => panic!("expected BendlReadError::Io(UnexpectedEof), got {other:?}"),
    }

    let mut buf = [0u8; 64];
    let mut raw_unverified = reader
        .assignment_stream_reader_unverified()
        .expect("constructor must succeed");
    let err = raw_unverified.read(&mut buf).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
}
