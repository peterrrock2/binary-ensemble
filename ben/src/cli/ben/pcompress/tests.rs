//! Tests for the `ben pcompress` subcommand. Ported from the former `pcben` CLI.

use super::super::args::{Cli, Command, PcompressDirection};
use super::paths::{derive_output_path, resolved_output_path, PcDirection};
use super::translate::{assignment_decode_ben, assignment_encode_ben, assignment_encode_xben};
use crate::codec::decode::{decode_ben_to_jsonl, decode_xben_to_jsonl};
use crate::codec::encode::encode_jsonl_to_ben;
use crate::BenVariant;
use clap::Parser;
use std::io::{self, BufReader, Cursor};

#[test]
fn parse_to_xben_args() {
    let cli = Cli::try_parse_from([
        "ben",
        "pcompress",
        "to-xben",
        "--output-file",
        "output.xben",
        "--verbose",
        "input.pc",
    ])
    .unwrap();

    assert_eq!(cli.globals.output_file.as_deref(), Some("output.xben"));
    assert!(cli.globals.verbose);
    match cli.command {
        Command::Pcompress(p) => match p.direction {
            PcompressDirection::ToXben(io) => {
                assert_eq!(io.input_file.as_deref(), Some("input.pc"))
            }
            other => panic!("expected to-xben, got {other:?}"),
        },
        other => panic!("expected pcompress, got {other:?}"),
    }
}

#[test]
fn derive_output_path_replaces_expected_suffixes() {
    assert_eq!(
        derive_output_path(PcDirection::FromBen, "plans.ben"),
        "plans.pcompress"
    );
    assert_eq!(
        derive_output_path(PcDirection::ToBen, "plans.pcompress"),
        "plans.ben"
    );
    assert_eq!(
        derive_output_path(PcDirection::ToXben, "plans.pc"),
        "plans.xben"
    );
}

#[test]
fn resolved_output_path_returns_none_when_both_paths_absent() {
    let result = resolved_output_path(PcDirection::FromBen, None, None, false).unwrap();
    assert!(result.is_none());
}

#[test]
fn assignment_decode_ben_writes_json_lines() {
    // BEN and PCOMPRESS are both zero-based, so ids transcode unchanged.
    let jsonl = br#"{"assignment":[0,0,1],"sample":1}
{"assignment":[1,2,2],"sample":2}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(BufReader::new(&jsonl[..]), &mut ben, BenVariant::Standard).unwrap();

    let mut out = Vec::new();
    assignment_decode_ben(Cursor::new(ben), &mut out).unwrap();

    assert_eq!(String::from_utf8(out).unwrap(), "[0,0,1]\n[1,2,2]\n");
}

#[test]
fn assignment_encode_ben_writes_ben_unchanged() {
    let input = b"[0,0,1]\n[1,1,2]\n";
    let mut ben = Vec::new();
    assignment_encode_ben(BufReader::new(&input[..]), &mut ben).unwrap();

    let mut out = Vec::new();
    decode_ben_to_jsonl(Cursor::new(ben), &mut out).unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains(r#""assignment":[0,0,1]"#));
    assert!(rendered.contains(r#""assignment":[1,1,2]"#));
}

#[test]
fn assignment_decode_ben_passes_through_id_zero_and_max() {
    // Both id 0 and id 65535 transcode straight through now that there is no ±1 shift.
    let jsonl = br#"{"assignment":[0,65535,1],"sample":1}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(BufReader::new(&jsonl[..]), &mut ben, BenVariant::Standard).unwrap();

    let mut out = Vec::new();
    assignment_decode_ben(Cursor::new(ben), &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "[0,65535,1]\n");
}

#[test]
fn assignment_encode_ben_accepts_id_65535() {
    let input = b"[0,65535]\n";
    let mut ben = Vec::new();
    assignment_encode_ben(BufReader::new(&input[..]), &mut ben).unwrap();

    let mut out = Vec::new();
    decode_ben_to_jsonl(Cursor::new(ben), &mut out).unwrap();
    assert!(String::from_utf8(out)
        .unwrap()
        .contains(r#""assignment":[0,65535]"#));
}

#[test]
fn assignment_encode_ben_rejects_malformed_line_without_panicking() {
    let input = b"[0,1]\nnot json at all\n";
    let mut ben = Vec::new();
    let err = assignment_encode_ben(BufReader::new(&input[..]), &mut ben).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("malformed"));
}

#[test]
fn assignment_encode_xben_rejects_malformed_line_without_panicking() {
    let input = b"[0,1,oops\n";
    let mut xben = Vec::new();
    let err = assignment_encode_xben(BufReader::new(&input[..]), &mut xben).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("malformed"));
}

#[test]
fn assignment_decode_ben_propagates_read_error() {
    struct AlwaysErrors;
    impl io::Read for AlwaysErrors {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken"))
        }
    }
    let mut out = Vec::new();
    let err = assignment_decode_ben(AlwaysErrors, &mut out).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn assignment_encode_xben_writes_xben_unchanged() {
    let input = b"[0,1,1]\n[2,2,0]\n";

    let mut xben = Vec::new();
    assignment_encode_xben(BufReader::new(&input[..]), &mut xben).unwrap();

    let mut out = Vec::new();
    decode_xben_to_jsonl(Cursor::new(xben), &mut out).unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains(r#""assignment":[0,1,1]"#));
    assert!(rendered.contains(r#""assignment":[2,2,0]"#));
}

#[test]
fn assignment_decode_ben_iterator_error_propagates() {
    use crate::format::banners::STANDARD_BEN_BANNER;
    use std::io::Read;

    struct BannerThenError {
        banner: &'static [u8],
        pos: usize,
    }
    impl Read for BannerThenError {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos < self.banner.len() {
                let n = buf.len().min(self.banner.len() - self.pos);
                buf[..n].copy_from_slice(&self.banner[self.pos..self.pos + n]);
                self.pos += n;
                Ok(n)
            } else {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken"))
            }
        }
    }

    let reader = BannerThenError {
        banner: STANDARD_BEN_BANNER,
        pos: 0,
    };
    let mut out = Vec::new();
    let err = assignment_decode_ben(reader, &mut out).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}
