use super::*;
use crate::codec::decode::{decode_ben_to_jsonl, decode_xben_to_jsonl};
use crate::codec::encode::encode_jsonl_to_ben;
use clap::{CommandFactory, Parser};
use std::io::{BufReader, Cursor};

#[test]
fn clap_metadata_uses_package_version() {
    let mut command = Args::command();
    let help = command.render_long_help().to_string();

    assert_eq!(command.get_version(), Some(env!("CARGO_PKG_VERSION")));
    assert!(help.contains("PCOMPRESS"));
    assert!(help.contains("--mode"));
}

#[test]
fn parse_pc_to_xben_args() {
    let args = Args::try_parse_from([
        "pben",
        "--mode",
        "pc-to-xben",
        "--input-file",
        "input.pc",
        "--output-file",
        "output.xben",
        "--verbose",
    ])
    .unwrap();

    assert_eq!(args.mode, Mode::PcToXben);
    assert_eq!(args.input_file.as_deref(), Some("input.pc"));
    assert_eq!(args.output_file.as_deref(), Some("output.xben"));
    assert!(args.verbose);
}

#[test]
fn derive_output_path_replaces_expected_suffixes() {
    assert_eq!(
        derive_output_path(Mode::BenToPc, "plans.ben"),
        "plans.pcompress"
    );
    assert_eq!(
        derive_output_path(Mode::PcToBen, "plans.pcompress"),
        "plans.ben"
    );
    assert_eq!(derive_output_path(Mode::PcToXben, "plans.pc"), "plans.xben");
}

#[test]
fn assignment_decode_ben_writes_json_lines() {
    let jsonl = br#"{"assignment":[1,1,2],"sample":1}
{"assignment":[2,3,3],"sample":2}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(BufReader::new(&jsonl[..]), &mut ben, BenVariant::Standard).unwrap();

    let mut out = Vec::new();
    assignment_decode_ben(Cursor::new(ben), &mut out).unwrap();

    assert_eq!(String::from_utf8(out).unwrap(), "[0,0,1]\n[1,2,2]\n");
}

#[test]
fn assignment_encode_ben_offsets_values_and_writes_ben() {
    let input = b"[0,0,1]\n[1,1,2]\n";
    let mut ben = Vec::new();
    assignment_encode_ben(BufReader::new(&input[..]), &mut ben).unwrap();

    let mut out = Vec::new();
    decode_ben_to_jsonl(Cursor::new(ben), &mut out).unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains(r#""assignment":[1,1,2]"#));
    assert!(rendered.contains(r#""assignment":[2,2,3]"#));
}

#[test]
fn resolved_output_path_returns_none_when_both_paths_absent() {
    // When neither output_file nor input_file is given, stdout mode: Ok(None).
    let result = resolved_output_path(Mode::BenToPc, None, None, false).unwrap();
    assert!(result.is_none());
}

#[test]
fn assignment_decode_ben_propagates_read_error() {
    // assignment_decode_ben propagates I/O errors from the BEN reader.
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
fn assignment_encode_xben_offsets_values_and_writes_xben() {
    let input = b"[0,1,1]\n[2,2,0]\n";

    let mut xben = Vec::new();
    assignment_encode_xben(BufReader::new(&input[..]), &mut xben).unwrap();

    let mut out = Vec::new();
    decode_xben_to_jsonl(Cursor::new(xben), &mut out).unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains(r#""assignment":[1,2,2]"#));
    assert!(rendered.contains(r#""assignment":[3,3,1]"#));
}

#[test]
fn assignment_decode_ben_iterator_error_propagates() {
    // Provides a valid BEN banner so AssignmentReader::new succeeds,
    // then returns a non-EOF error on the next read so the iterator
    // fires the Err(e) => return Err(e) arm (line 204).
    use std::io::Read;
    use crate::format::banners::STANDARD_BEN_BANNER;

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

    let reader = BannerThenError { banner: STANDARD_BEN_BANNER, pos: 0 };
    let mut out = Vec::new();
    let err = assignment_decode_ben(reader, &mut out).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}
