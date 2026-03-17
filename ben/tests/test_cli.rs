use binary_ensemble::codec::decode::decode_ben_to_jsonl;
use binary_ensemble::codec::encode::encode_jsonl_to_ben;
use binary_ensemble::BenVariant;
use serde_json::Value;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("binary-ensemble-cli-{name}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn bin_path(name: &str) -> &'static str {
    match name {
        "ben" => env!("CARGO_BIN_EXE_ben"),
        "pben" => env!("CARGO_BIN_EXE_pben"),
        "reben" => env!("CARGO_BIN_EXE_reben"),
        _ => panic!("unknown binary {name}"),
    }
}

fn run(bin: &str, args: &[&str], cwd: &Path) -> Output {
    Command::new(bin_path(bin))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}

fn run_with_stdin(bin: &str, args: &[&str], cwd: &Path, stdin: &[u8]) -> Output {
    let mut child = Command::new(bin_path(bin))
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut input = child.stdin.take().unwrap();
        use std::io::Write;
        input.write_all(stdin).unwrap();
    }
    child.wait_with_output().unwrap()
}

fn run_stdin_stdout(bin: &str, args: &[&str], cwd: &Path, stdin: &[u8]) -> Output {
    run_with_stdin(bin, args, cwd, stdin)
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn assert_failure(output: &Output) {
    assert!(
        !output.status.success(),
        "expected failure\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn sample_jsonl() -> &'static str {
    r#"{"assignment":[1,1,2],"sample":1}
{"assignment":[2,2,3],"sample":2}
"#
}

fn sample_graph() -> &'static str {
    r#"{
  "nodes": [
    {"id": 2, "GEOID20": "B"},
    {"id": 0, "GEOID20": "A"},
    {"id": 1, "GEOID20": "C"}
  ],
  "adjacency": [
    [{"id": 0}, {"id": 1}],
    [{"id": 2}],
    [{"id": 2}]
  ]
}"#
}

#[test]
fn all_clis_report_help_and_package_version() {
    for bin in ["ben", "pben", "reben"] {
        let help = run(bin, &["--help"], Path::new("."));
        assert_success(&help);
        let help_text = String::from_utf8_lossy(&help.stdout);
        assert!(help_text.contains("Usage:"));

        let version = run(bin, &["--version"], Path::new("."));
        assert_success(&version);
        let version_text = String::from_utf8_lossy(&version.stdout);
        assert!(version_text.contains(env!("CARGO_PKG_VERSION")));
    }
}

#[test]
fn ben_cli_encode_decode_read_and_x_modes_roundtrip() {
    let temp = TempDir::new("ben-workflow");
    let jsonl_path = temp.path().join("samples.jsonl");
    let ben_path = temp.path().join("samples.ben");
    let decoded_path = temp.path().join("decoded.jsonl");
    let xben_path = temp.path().join("samples.xben");
    let xdecoded_path = temp.path().join("xdecoded.jsonl");

    fs::write(&jsonl_path, sample_jsonl()).unwrap();

    let encode = run(
        "ben",
        &[
            "--mode",
            "encode",
            jsonl_path.to_str().unwrap(),
            "--output-file",
            ben_path.to_str().unwrap(),
            "--save-all",
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&encode);

    let decode = run(
        "ben",
        &[
            "--mode",
            "decode",
            ben_path.to_str().unwrap(),
            "--output-file",
            decoded_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&decode);
    assert_eq!(fs::read_to_string(&decoded_path).unwrap(), sample_jsonl());

    let read = run(
        "ben",
        &[
            "--mode",
            "read",
            ben_path.to_str().unwrap(),
            "--sample-number",
            "2",
            "--print",
        ],
        temp.path(),
    );
    assert_success(&read);
    assert_eq!(String::from_utf8(read.stdout).unwrap(), "[2, 2, 3]\n");

    let xencode = run(
        "ben",
        &[
            "--mode",
            "x-encode",
            jsonl_path.to_str().unwrap(),
            "--output-file",
            xben_path.to_str().unwrap(),
            "--jsonl-and-xben",
            "--save-all",
            "--n-cpus",
            "1",
            "--compression-level",
            "1",
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&xencode);

    let xdecode = run(
        "ben",
        &[
            "--mode",
            "x-decode",
            xben_path.to_str().unwrap(),
            "--output-file",
            xdecoded_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&xdecode);
    assert_eq!(fs::read_to_string(&xdecoded_path).unwrap(), sample_jsonl());
}

#[test]
fn ben_cli_supports_stdin_stdout_workflows() {
    let temp = TempDir::new("ben-streams");

    let encode = run_stdin_stdout(
        "ben",
        &["--mode", "encode", "--save-all"],
        temp.path(),
        sample_jsonl().as_bytes(),
    );
    assert_success(&encode);

    let decode = run_stdin_stdout(
        "ben",
        &["--mode", "decode", "--jsonl-and-ben"],
        temp.path(),
        &encode.stdout,
    );
    assert_success(&decode);
    assert_eq!(String::from_utf8(decode.stdout).unwrap(), sample_jsonl());

    let xencode_jsonl = run_stdin_stdout(
        "ben",
        &[
            "--mode",
            "x-encode",
            "--jsonl-and-xben",
            "--save-all",
            "--n-cpus",
            "1",
            "--compression-level",
            "1",
        ],
        temp.path(),
        sample_jsonl().as_bytes(),
    );
    assert_success(&xencode_jsonl);

    let xdecode_jsonl = run_stdin_stdout(
        "ben",
        &["--mode", "x-decode"],
        temp.path(),
        &xencode_jsonl.stdout,
    );
    assert_success(&xdecode_jsonl);
    assert_eq!(
        String::from_utf8(xdecode_jsonl.stdout).unwrap(),
        sample_jsonl()
    );

    let mut ben_bytes = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(sample_jsonl().as_bytes()),
        &mut ben_bytes,
        BenVariant::MkvChain,
    )
    .unwrap();

    let xencode_ben = run_stdin_stdout(
        "ben",
        &[
            "--mode",
            "x-encode",
            "--ben-and-xben",
            "--n-cpus",
            "1",
            "--compression-level",
            "1",
        ],
        temp.path(),
        &ben_bytes,
    );
    assert_success(&xencode_ben);

    let decode_ben = run_stdin_stdout(
        "ben",
        &["--mode", "decode", "--ben-and-xben"],
        temp.path(),
        &xencode_ben.stdout,
    );
    assert_success(&decode_ben);

    let mut roundtrip_jsonl = Vec::new();
    decode_ben_to_jsonl(BufReader::new(&decode_ben.stdout[..]), &mut roundtrip_jsonl).unwrap();
    let mut original_jsonl = Vec::new();
    decode_ben_to_jsonl(BufReader::new(&ben_bytes[..]), &mut original_jsonl).unwrap();
    assert_eq!(roundtrip_jsonl, original_jsonl);
}

#[test]
fn ben_cli_xz_roundtrip_and_overwrite_prompt() {
    let temp = TempDir::new("ben-xz");
    let input_path = temp.path().join("samples.jsonl");
    let xz_path = temp.path().join("samples.jsonl.xz");
    let restored_path = temp.path().join("samples.jsonl.restored");

    fs::write(&input_path, sample_jsonl()).unwrap();

    let compress = run(
        "ben",
        &[
            "--mode",
            "xz-compress",
            input_path.to_str().unwrap(),
            "--output-file",
            xz_path.to_str().unwrap(),
            "--n-cpus",
            "1",
            "--compression-level",
            "1",
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&compress);

    fs::write(&restored_path, "stale output").unwrap();
    let decompress = run_with_stdin(
        "ben",
        &[
            "--mode",
            "xz-decompress",
            xz_path.to_str().unwrap(),
            "--output-file",
            restored_path.to_str().unwrap(),
        ],
        temp.path(),
        b"y\n",
    );
    assert_success(&decompress);
    assert_eq!(fs::read_to_string(&restored_path).unwrap(), sample_jsonl());
}

#[test]
fn ben_cli_supports_ben_to_xben_and_xben_to_ben_paths() {
    let temp = TempDir::new("ben-xben-paths");
    let jsonl_path = temp.path().join("samples.jsonl");
    let ben_path = temp.path().join("samples.ben");
    let xben_path = temp.path().join("samples.xben");
    let roundtrip_ben_path = temp.path().join("roundtrip.ben");

    fs::write(&jsonl_path, sample_jsonl()).unwrap();

    let mut ben_bytes = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(fs::File::open(&jsonl_path).unwrap()),
        &mut ben_bytes,
        BenVariant::MkvChain,
    )
    .unwrap();
    fs::write(&ben_path, ben_bytes).unwrap();

    let xencode = run(
        "ben",
        &[
            "--mode",
            "x-encode",
            ben_path.to_str().unwrap(),
            "--output-file",
            xben_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&xencode);

    let decode = run(
        "ben",
        &[
            "--mode",
            "decode",
            xben_path.to_str().unwrap(),
            "--output-file",
            roundtrip_ben_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&decode);

    let mut original_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&ben_path).unwrap()),
        &mut original_jsonl,
    )
    .unwrap();
    let mut roundtrip_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&roundtrip_ben_path).unwrap()),
        &mut roundtrip_jsonl,
    )
    .unwrap();
    assert_eq!(original_jsonl, roundtrip_jsonl);
}

#[test]
fn ben_cli_uses_default_output_names() {
    let temp = TempDir::new("ben-defaults");
    let jsonl_path = temp.path().join("samples.jsonl");
    let ben_path = temp.path().join("samples.jsonl.ben");
    let xz_path = temp.path().join("samples.jsonl.xz");

    fs::write(&jsonl_path, sample_jsonl()).unwrap();

    let encode = run(
        "ben",
        &[
            "--mode",
            "encode",
            jsonl_path.to_str().unwrap(),
            "--save-all",
        ],
        temp.path(),
    );
    assert_success(&encode);
    assert!(ben_path.exists());

    fs::remove_file(&jsonl_path).unwrap();
    let decode = run(
        "ben",
        &["--mode", "decode", ben_path.to_str().unwrap()],
        temp.path(),
    );
    assert_success(&decode);
    assert_eq!(fs::read_to_string(&jsonl_path).unwrap(), sample_jsonl());

    let compress = run(
        "ben",
        &["--mode", "xz-compress", jsonl_path.to_str().unwrap()],
        temp.path(),
    );
    assert_success(&compress);
    assert!(xz_path.exists());

    fs::remove_file(&jsonl_path).unwrap();
    let decompress = run(
        "ben",
        &["--mode", "xz-decompress", xz_path.to_str().unwrap()],
        temp.path(),
    );
    assert_success(&decompress);
    assert_eq!(fs::read_to_string(&jsonl_path).unwrap(), sample_jsonl());
}

#[test]
fn ben_cli_reports_expected_error_paths() {
    let temp = TempDir::new("ben-errors");
    let bogus_jsonl = temp.path().join("bogus.jsonl");
    let bogus_txt = temp.path().join("bogus.txt");
    let bogus_xz = temp.path().join("bogus.data");
    fs::write(&bogus_jsonl, sample_jsonl()).unwrap();
    fs::write(&bogus_txt, sample_jsonl()).unwrap();
    fs::write(&bogus_xz, "not xz").unwrap();

    let xencode = run(
        "ben",
        &["--mode", "x-encode", bogus_txt.to_str().unwrap()],
        temp.path(),
    );
    assert_success(&xencode);
    assert!(String::from_utf8_lossy(&xencode.stderr)
        .contains("Unsupported file type(s) for xencode mode"));

    let decode = run(
        "ben",
        &["--mode", "decode", bogus_jsonl.to_str().unwrap()],
        temp.path(),
    );
    assert_success(&decode);
    assert!(
        String::from_utf8_lossy(&decode.stderr).contains("Unsupported file type for decode mode")
    );

    let read = run(
        "ben",
        &["--mode", "read", bogus_jsonl.to_str().unwrap()],
        temp.path(),
    );
    assert_success(&read);
    assert!(
        String::from_utf8_lossy(&read.stderr).contains("Sample number is required in read mode")
    );

    let xz = run(
        "ben",
        &["--mode", "xz-decompress", bogus_xz.to_str().unwrap()],
        temp.path(),
    );
    assert_success(&xz);
    assert!(String::from_utf8_lossy(&xz.stderr)
        .contains("Unsupported file type for xz decompress mode"));

    let bad_xben = run_stdin_stdout("ben", &["--mode", "x-decode"], temp.path(), b"not-an-xben");
    assert_success(&bad_xben);
    assert!(String::from_utf8_lossy(&bad_xben.stderr).contains("Error:"));

    let bad_decode_ben = run_stdin_stdout(
        "ben",
        &["--mode", "decode", "--jsonl-and-ben"],
        temp.path(),
        b"not-a-ben",
    );
    assert_success(&bad_decode_ben);
    assert!(String::from_utf8_lossy(&bad_decode_ben.stderr).contains("Error:"));

    let bad_decode_xben = run_stdin_stdout(
        "ben",
        &["--mode", "decode", "--ben-and-xben"],
        temp.path(),
        b"not-an-xben",
    );
    assert_success(&bad_decode_xben);
    assert!(String::from_utf8_lossy(&bad_decode_xben.stderr).contains("Error:"));
}

#[test]
fn ben_cli_reports_overwrite_denials_and_remaining_error_modes() {
    let temp = TempDir::new("ben-overwrite");
    let jsonl_path = temp.path().join("samples.jsonl");
    let ben_path = temp.path().join("samples.ben");
    let xben_path = temp.path().join("samples.xben");
    let xz_path = temp.path().join("samples.xz");
    let occupied = temp.path().join("occupied.out");
    let invalid_ben = temp.path().join("invalid.ben");
    let invalid_xben = temp.path().join("invalid.xben");
    let invalid_xz = temp.path().join("invalid.xz");

    fs::write(&jsonl_path, sample_jsonl()).unwrap();
    fs::write(&occupied, "occupied").unwrap();
    fs::write(&invalid_ben, "not ben").unwrap();
    fs::write(&invalid_xben, "not xben").unwrap();
    fs::write(&invalid_xz, "not xz").unwrap();

    let mut ben_bytes = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(sample_jsonl().as_bytes()),
        &mut ben_bytes,
        BenVariant::MkvChain,
    )
    .unwrap();
    fs::write(&ben_path, &ben_bytes).unwrap();

    let xencode_from_ben = run(
        "ben",
        &[
            "--mode",
            "x-encode",
            ben_path.to_str().unwrap(),
            "--output-file",
            xben_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&xencode_from_ben);

    let xz_compress = run(
        "ben",
        &[
            "--mode",
            "xz-compress",
            jsonl_path.to_str().unwrap(),
            "--output-file",
            xz_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&xz_compress);

    for output in [
        run_with_stdin(
            "ben",
            &[
                "--mode",
                "encode",
                jsonl_path.to_str().unwrap(),
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            b"n\n",
        ),
        run_with_stdin(
            "ben",
            &[
                "--mode",
                "encode",
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            sample_jsonl().as_bytes(),
        ),
        run_with_stdin(
            "ben",
            &[
                "--mode",
                "x-encode",
                ben_path.to_str().unwrap(),
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            b"n\n",
        ),
        run_with_stdin(
            "ben",
            &[
                "--mode",
                "x-encode",
                "--jsonl-and-xben",
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            sample_jsonl().as_bytes(),
        ),
        run_with_stdin(
            "ben",
            &[
                "--mode",
                "decode",
                "--jsonl-and-ben",
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            b"n\n",
        ),
        run_with_stdin(
            "ben",
            &[
                "--mode",
                "x-decode",
                xben_path.to_str().unwrap(),
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            b"n\n",
        ),
        run_with_stdin(
            "ben",
            &[
                "--mode",
                "x-decode",
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            b"n\n",
        ),
        run_with_stdin(
            "ben",
            &[
                "--mode",
                "read",
                ben_path.to_str().unwrap(),
                "--sample-number",
                "1",
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            b"n\n",
        ),
        run_with_stdin(
            "ben",
            &[
                "--mode",
                "xz-compress",
                jsonl_path.to_str().unwrap(),
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            b"n\n",
        ),
        run_with_stdin(
            "ben",
            &[
                "--mode",
                "xz-decompress",
                xz_path.to_str().unwrap(),
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            b"n\n",
        ),
    ] {
        assert_success(&output);
        assert!(String::from_utf8_lossy(&output.stderr).contains("AlreadyExists"));
    }

    let invalid_ben_to_xben = run(
        "ben",
        &[
            "--mode",
            "x-encode",
            invalid_ben.to_str().unwrap(),
            "--output-file",
            temp.path().join("bad.xben").to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&invalid_ben_to_xben);
    assert!(String::from_utf8_lossy(&invalid_ben_to_xben.stderr).contains("Error:"));

    let unsupported_decode = run_stdin_stdout("ben", &["--mode", "decode"], temp.path(), b"");
    assert_success(&unsupported_decode);
    assert!(String::from_utf8_lossy(&unsupported_decode.stderr)
        .contains("Unsupported file type(s) for decode mode"));

    let read_too_large = run(
        "ben",
        &[
            "--mode",
            "read",
            ben_path.to_str().unwrap(),
            "--sample-number",
            "99",
            "--print",
        ],
        temp.path(),
    );
    assert_success(&read_too_large);
    assert!(String::from_utf8_lossy(&read_too_large.stderr).contains("Error:"));

    let invalid_decode_ben = run(
        "ben",
        &[
            "--mode",
            "decode",
            invalid_ben.to_str().unwrap(),
            "--output-file",
            temp.path().join("decoded.jsonl").to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&invalid_decode_ben);
    assert!(String::from_utf8_lossy(&invalid_decode_ben.stderr).contains("Error:"));

    let invalid_decode_xben = run(
        "ben",
        &[
            "--mode",
            "decode",
            invalid_xben.to_str().unwrap(),
            "--output-file",
            temp.path().join("decoded.ben").to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&invalid_decode_xben);
    assert!(String::from_utf8_lossy(&invalid_decode_xben.stderr).contains("Error:"));

    let invalid_xdecode = run(
        "ben",
        &[
            "--mode",
            "x-decode",
            invalid_xben.to_str().unwrap(),
            "--output-file",
            temp.path().join("decoded2.jsonl").to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&invalid_xdecode);
    assert!(String::from_utf8_lossy(&invalid_xdecode.stderr).contains("Error:"));

    let invalid_xz_decompress = run(
        "ben",
        &[
            "--mode",
            "xz-decompress",
            invalid_xz.to_str().unwrap(),
            "--output-file",
            temp.path().join("decoded3.txt").to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&invalid_xz_decompress);
}

#[test]
fn reben_cli_json_and_ben_modes_work() {
    let temp = TempDir::new("reben-workflow");
    let graph_path = temp.path().join("dual_graph.json");
    let sorted_path = temp.path().join("sorted_graph.json");
    let jsonl_path = temp.path().join("samples.jsonl");
    let ben_path = temp.path().join("samples.jsonl.ben");
    let canonical_path = temp.path().join("canonicalized.ben");
    let map_relabel_path = temp.path().join("map_relabel.ben");

    fs::write(&graph_path, sample_graph()).unwrap();
    fs::write(
        &jsonl_path,
        r#"{"assignment":[9,9,4],"sample":1}
{"assignment":[4,7,7],"sample":2}
"#,
    )
    .unwrap();

    let mut ben_bytes = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(fs::File::open(&jsonl_path).unwrap()),
        &mut ben_bytes,
        BenVariant::Standard,
    )
    .unwrap();
    fs::write(&ben_path, ben_bytes).unwrap();

    let sort_graph = run(
        "reben",
        &[
            graph_path.to_str().unwrap(),
            "--mode",
            "json",
            "--key",
            "GEOID20",
            "--output-file",
            sorted_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&sort_graph);

    let sorted_graph = fs::read_to_string(&sorted_path).unwrap();
    assert!(sorted_graph.contains(r#""id":0"#));
    assert!(sorted_graph.contains(r#""GEOID20":"A"#));

    let map_path = temp.path().join("dual_graph_sorted_by_GEOID20_map.json");
    assert!(map_path.exists());

    let canonicalize = run(
        "reben",
        &[
            ben_path.to_str().unwrap(),
            "--mode",
            "ben",
            "--output-file",
            canonical_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&canonicalize);

    let relabel = run(
        "reben",
        &[
            ben_path.to_str().unwrap(),
            "--mode",
            "ben",
            "--map-file",
            map_path.to_str().unwrap(),
            "--output-file",
            map_relabel_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&relabel);

    let mut canonical_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&canonical_path).unwrap()),
        &mut canonical_jsonl,
    )
    .unwrap();
    let canonical_text = String::from_utf8(canonical_jsonl).unwrap();
    assert!(canonical_text.contains(r#""assignment":[1,1,2]"#));
    assert!(canonical_text.contains(r#""assignment":[1,2,2]"#));

    let mut relabeled_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&map_relabel_path).unwrap()),
        &mut relabeled_jsonl,
    )
    .unwrap();
    let relabeled_text = String::from_utf8(relabeled_jsonl).unwrap();
    assert!(relabeled_text.contains(r#""assignment":[9,4,9]"#));
}

#[test]
fn reben_cli_can_limit_ben_relabeling_to_first_n_items() {
    let temp = TempDir::new("reben-limit");
    let graph_path = temp.path().join("dual_graph.json");
    let jsonl_path = temp.path().join("samples.jsonl");
    let ben_path = temp.path().join("samples.jsonl.ben");
    let canonical_path = temp.path().join("canonicalized_first_one.ben");
    let map_path = temp.path().join("dual_graph_sorted_by_GEOID20_map.json");
    let map_relabel_path = temp.path().join("map_relabel_first_one.ben");

    fs::write(&graph_path, sample_graph()).unwrap();
    fs::write(
        &jsonl_path,
        r#"{"assignment":[9,9,4],"sample":1}
{"assignment":[4,7,7],"sample":2}
"#,
    )
    .unwrap();

    let mut ben_bytes = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(fs::File::open(&jsonl_path).unwrap()),
        &mut ben_bytes,
        BenVariant::Standard,
    )
    .unwrap();
    fs::write(&ben_path, ben_bytes).unwrap();

    let sort_graph = run(
        "reben",
        &[
            graph_path.to_str().unwrap(),
            "--mode",
            "json",
            "--key",
            "GEOID20",
        ],
        temp.path(),
    );
    assert_success(&sort_graph);
    assert!(map_path.exists());

    let canonicalize = run(
        "reben",
        &[
            ben_path.to_str().unwrap(),
            "--mode",
            "ben",
            "--n-items",
            "1",
            "--output-file",
            canonical_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&canonicalize);

    let relabel = run(
        "reben",
        &[
            ben_path.to_str().unwrap(),
            "--mode",
            "ben",
            "--map-file",
            map_path.to_str().unwrap(),
            "--n-items",
            "1",
            "--output-file",
            map_relabel_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&relabel);

    let mut canonical_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&canonical_path).unwrap()),
        &mut canonical_jsonl,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(canonical_jsonl).unwrap(),
        "{\"assignment\":[1,1,2],\"sample\":1}\n"
    );

    let mut relabeled_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&map_relabel_path).unwrap()),
        &mut relabeled_jsonl,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(relabeled_jsonl).unwrap(),
        "{\"assignment\":[9,4,9],\"sample\":1}\n"
    );
}

#[test]
fn reben_cli_supports_twodelta_ben_mode() {
    let temp = TempDir::new("reben-twodelta");
    let graph_path = temp.path().join("dual_graph.json");
    let ben_path = temp.path().join("samples.twodelta.ben");
    let canonical_path = temp.path().join("canonicalized_twodelta.ben");
    let map_relabel_path = temp.path().join("map_relabel_twodelta.ben");

    fs::write(&graph_path, sample_graph()).unwrap();

    let mut ben_bytes = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(
            r#"{"assignment":[1,1,2],"sample":1}
{"assignment":[1,1,2],"sample":2}
{"assignment":[1,2,1],"sample":3}
{"assignment":[2,2,1],"sample":4}
"#
            .as_bytes(),
        ),
        &mut ben_bytes,
        BenVariant::TwoDelta,
    )
    .unwrap();
    fs::write(&ben_path, ben_bytes).unwrap();

    let sort_graph = run(
        "reben",
        &[
            graph_path.to_str().unwrap(),
            "--mode",
            "json",
            "--key",
            "GEOID20",
        ],
        temp.path(),
    );
    assert_success(&sort_graph);

    let map_path = temp.path().join("dual_graph_sorted_by_GEOID20_map.json");
    assert!(map_path.exists());

    let canonicalize = run(
        "reben",
        &[
            ben_path.to_str().unwrap(),
            "--mode",
            "ben",
            "--output-file",
            canonical_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&canonicalize);

    let relabel = run(
        "reben",
        &[
            ben_path.to_str().unwrap(),
            "--mode",
            "ben",
            "--map-file",
            map_path.to_str().unwrap(),
            "--output-file",
            map_relabel_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&relabel);

    let mut canonical_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&canonical_path).unwrap()),
        &mut canonical_jsonl,
    )
    .unwrap();
    assert!(String::from_utf8(canonical_jsonl)
        .unwrap()
        .contains(r#""assignment":[1,1,2]"#));

    let mut relabeled_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&map_relabel_path).unwrap()),
        &mut relabeled_jsonl,
    )
    .unwrap();
    assert!(String::from_utf8(relabeled_jsonl)
        .unwrap()
        .contains(r#""assignment":[1,2,1]"#));
}

#[test]
fn reben_cli_can_convert_between_ben_variants() {
    let temp = TempDir::new("reben-convert");
    let ben_path = temp.path().join("samples.standard.ben");
    let twodelta_path = temp.path().join("samples.twodelta.ben");
    let mkv_path = temp.path().join("samples.mkv.ben");

    let source_jsonl = r#"{"assignment":[4,4,9],"sample":1}
{"assignment":[4,4,9],"sample":2}
{"assignment":[4,9,4],"sample":3}
{"assignment":[9,9,4],"sample":4}
"#;

    let mut ben_bytes = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(source_jsonl.as_bytes()),
        &mut ben_bytes,
        BenVariant::Standard,
    )
    .unwrap();
    fs::write(&ben_path, ben_bytes).unwrap();

    let to_twodelta = run(
        "reben",
        &[
            ben_path.to_str().unwrap(),
            "--mode",
            "ben",
            "--output-variant",
            "twodelta",
            "--convert-only",
            "--output-file",
            twodelta_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&to_twodelta);

    let twodelta_bytes = fs::read(&twodelta_path).unwrap();
    assert_eq!(&twodelta_bytes[..17], b"TWODELTA BEN FILE");

    let to_mkv = run(
        "reben",
        &[
            twodelta_path.to_str().unwrap(),
            "--mode",
            "ben",
            "--output-variant",
            "mkv-chain",
            "--convert-only",
            "--output-file",
            mkv_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&to_mkv);

    let mkv_bytes = fs::read(&mkv_path).unwrap();
    assert_eq!(&mkv_bytes[..17], b"MKVCHAIN BEN FILE");

    let mut original_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&ben_path).unwrap()),
        &mut original_jsonl,
    )
    .unwrap();

    let mut converted_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&mkv_path).unwrap()),
        &mut converted_jsonl,
    )
    .unwrap();

    assert_eq!(original_jsonl, converted_jsonl);
}

#[test]
fn reben_cli_can_canonicalize_into_a_different_ben_variant() {
    let temp = TempDir::new("reben-canonicalize-convert");
    let ben_path = temp.path().join("samples.standard.ben");
    let output_path = temp.path().join("canonicalized.twodelta.ben");

    let mut ben_bytes = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(
            r#"{"assignment":[9,9,4],"sample":1}
{"assignment":[4,7,7],"sample":2}
"#
            .as_bytes(),
        ),
        &mut ben_bytes,
        BenVariant::Standard,
    )
    .unwrap();
    fs::write(&ben_path, ben_bytes).unwrap();

    let canonicalize = run(
        "reben",
        &[
            ben_path.to_str().unwrap(),
            "--mode",
            "ben",
            "--output-variant",
            "twodelta",
            "--output-file",
            output_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&canonicalize);

    let bytes = fs::read(&output_path).unwrap();
    assert_eq!(&bytes[..17], b"TWODELTA BEN FILE");

    let mut canonical_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&output_path).unwrap()),
        &mut canonical_jsonl,
    )
    .unwrap();
    let canonical_text = String::from_utf8(canonical_jsonl).unwrap();
    assert!(canonical_text.contains(r#""assignment":[1,1,2]"#));
    assert!(canonical_text.contains(r#""assignment":[1,2,2]"#));
}

#[test]
fn reben_cli_generates_map_from_shape_file_and_reports_invalid_flag_combinations() {
    let temp = TempDir::new("reben-more");
    let graph_path = temp.path().join("shape.json");
    let ben_path = temp.path().join("samples.jsonl.ben");
    let relabeled_path = temp.path().join("rekeyed.ben");

    fs::write(&graph_path, sample_graph()).unwrap();
    let mut ben_bytes = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(sample_jsonl().as_bytes()),
        &mut ben_bytes,
        BenVariant::Standard,
    )
    .unwrap();
    fs::write(&ben_path, ben_bytes).unwrap();

    let relabel = run(
        "reben",
        &[
            ben_path.to_str().unwrap(),
            "--mode",
            "ben",
            "--key",
            "GEOID20",
            "--shape-file",
            graph_path.to_str().unwrap(),
            "--output-file",
            relabeled_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&relabel);
    assert!(temp
        .path()
        .join("shape_sorted_by_GEOID20_map.json")
        .exists());

    let generated_graph = temp.path().join("shape_sorted_by_GEOID20.json");
    let generated_map = temp.path().join("shape_sorted_by_GEOID20_map.json");
    let both = run(
        "reben",
        &[
            ben_path.to_str().unwrap(),
            "--mode",
            "ben",
            "--key",
            "GEOID20",
            "--shape-file",
            graph_path.to_str().unwrap(),
            "--map-file",
            generated_map.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_failure(&both);
    assert!(String::from_utf8_lossy(&both.stderr)
        .contains("Cannot provide both a map file and a sorting option"));

    let missing_shape = run(
        "reben",
        &[
            ben_path.to_str().unwrap(),
            "--mode",
            "ben",
            "--key",
            "GEOID20",
        ],
        temp.path(),
    );
    assert_failure(&missing_shape);
    assert!(String::from_utf8_lossy(&missing_shape.stderr).contains("No shape file provided"));

    let sorted_json: Value =
        serde_json::from_str(&fs::read_to_string(generated_graph).unwrap()).unwrap();
    assert_eq!(sorted_json["nodes"][0]["GEOID20"], "A");
}

#[test]
fn reben_cli_supports_mla_and_rcm_orderings() {
    let temp = TempDir::new("reben-orderings");
    let graph_path = temp.path().join("shape.json");
    let mla_path = temp.path().join("mla.json");
    let rcm_path = temp.path().join("rcm.json");

    fs::write(&graph_path, sample_graph()).unwrap();

    let mla = run(
        "reben",
        &[
            graph_path.to_str().unwrap(),
            "--mode",
            "json",
            "--ordering",
            "minimum-linear-arrangement",
            "--output-file",
            mla_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&mla);
    assert!(temp
        .path()
        .join("shape_sorted_by_minimum-linear-arrangement_map.json")
        .exists());

    let rcm = run(
        "reben",
        &[
            graph_path.to_str().unwrap(),
            "--mode",
            "json",
            "--ordering",
            "reverse-cuthill-mckee",
            "--output-file",
            rcm_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&rcm);
    assert!(temp
        .path()
        .join("shape_sorted_by_reverse-cuthill-mckee_map.json")
        .exists());

    let mla_json: Value = serde_json::from_str(&fs::read_to_string(&mla_path).unwrap()).unwrap();
    let rcm_json: Value = serde_json::from_str(&fs::read_to_string(&rcm_path).unwrap()).unwrap();
    assert_eq!(
        mla_json["nodes"].as_array().unwrap().len(),
        rcm_json["nodes"].as_array().unwrap().len()
    );
    assert!(!mla_json["nodes"].as_array().unwrap().is_empty());
}

#[test]
fn reben_cli_supports_multi_level_cluster_ordering() {
    let temp = TempDir::new("reben-mlc");
    let graph_path = temp.path().join("shape.json");
    let mlc_path = temp.path().join("mlc.json");

    fs::write(&graph_path, sample_graph()).unwrap();

    let mlc = run(
        "reben",
        &[
            graph_path.to_str().unwrap(),
            "--mode",
            "json",
            "--ordering",
            "multi-level-cluster",
            "--output-file",
            mlc_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&mlc);
    assert!(temp
        .path()
        .join("shape_sorted_by_multi-level-cluster_map.json")
        .exists());

    let mlc_json: Value = serde_json::from_str(&fs::read_to_string(&mlc_path).unwrap()).unwrap();
    assert!(!mlc_json["nodes"].as_array().unwrap().is_empty());
}

#[test]
fn pben_cli_converts_between_formats() {
    let temp = TempDir::new("pben");
    let jsonl_path = temp.path().join("samples.jsonl");
    let ben_path = temp.path().join("samples.ben");
    let pc_path = temp.path().join("samples.pc");
    let roundtrip_ben_path = temp.path().join("roundtrip.ben");
    let xben_path = temp.path().join("samples.xben");

    fs::write(&jsonl_path, sample_jsonl()).unwrap();
    let mut ben_bytes = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(fs::File::open(&jsonl_path).unwrap()),
        &mut ben_bytes,
        BenVariant::MkvChain,
    )
    .unwrap();
    fs::write(&ben_path, ben_bytes).unwrap();

    let ben_to_pc = run(
        "pben",
        &[
            "--mode",
            "ben-to-pc",
            "--input-file",
            ben_path.to_str().unwrap(),
            "--output-file",
            pc_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&ben_to_pc);
    assert!(pc_path.exists());

    let pc_to_ben = run(
        "pben",
        &[
            "--mode",
            "pc-to-ben",
            "--input-file",
            pc_path.to_str().unwrap(),
            "--output-file",
            roundtrip_ben_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&pc_to_ben);

    let pc_to_xben = run(
        "pben",
        &[
            "--mode",
            "pc-to-xben",
            "--input-file",
            pc_path.to_str().unwrap(),
            "--output-file",
            xben_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&pc_to_xben);

    let mut roundtrip_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&roundtrip_ben_path).unwrap()),
        &mut roundtrip_jsonl,
    )
    .unwrap();
    assert!(String::from_utf8(roundtrip_jsonl)
        .unwrap()
        .contains(r#""assignment":[2,2,3]"#));

    let xdecode = run(
        "ben",
        &["--mode", "x-decode", xben_path.to_str().unwrap(), "--print"],
        temp.path(),
    );
    assert_success(&xdecode);
    let printed = String::from_utf8_lossy(&xdecode.stdout);
    assert!(printed.contains(r#""assignment":[2,2,3]"#));
}
