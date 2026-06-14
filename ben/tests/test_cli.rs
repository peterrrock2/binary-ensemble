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
        "bendl" => env!("CARGO_BIN_EXE_bendl"),
        _ => panic!("unknown binary {name}"),
    }
}

/// Build a `Command` for one of the workspace CLIs, honoring any cross-compilation runner cargo
/// was configured with (e.g. `CARGO_TARGET_S390X_UNKNOWN_LINUX_GNU_RUNNER` inside a `cross`
/// container running the suite under QEMU). Cargo routes the *test binary itself* through that
/// runner automatically, but subprocesses spawned by tests exec directly; without this shim, a
/// foreign-architecture CLI binary is handed straight to the host kernel, which rejects it (or
/// hands it to a shell that mangles the ELF as a script). The variable is only ever set in
/// cross-compilation environments, so native runs take the plain-exec path.
fn cli_command(bin: &str) -> Command {
    let runner = std::env::vars().find_map(|(key, value)| {
        (key.starts_with("CARGO_TARGET_") && key.ends_with("_RUNNER") && !value.trim().is_empty())
            .then_some(value)
    });
    match runner {
        Some(runner) => {
            let mut parts = runner.split_whitespace();
            let mut cmd = Command::new(parts.next().expect("runner value is non-empty"));
            cmd.args(parts);
            cmd.arg(bin_path(bin));
            cmd
        }
        None => Command::new(bin_path(bin)),
    }
}

fn run(bin: &str, args: &[&str], cwd: &Path) -> Output {
    cli_command(bin)
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}

fn run_with_stdin(bin: &str, args: &[&str], cwd: &Path, stdin: &[u8]) -> Output {
    let mut child = cli_command(bin)
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
    for bin in ["ben", "bendl"] {
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
            "lookup",
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
            "xencode",
            jsonl_path.to_str().unwrap(),
            "--output-file",
            xben_path.to_str().unwrap(),
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
            "xdecode",
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
        &["encode", "--save-all"],
        temp.path(),
        sample_jsonl().as_bytes(),
    );
    assert_success(&encode);

    let decode = run_stdin_stdout("ben", &["decode"], temp.path(), &encode.stdout);
    assert_success(&decode);
    assert_eq!(String::from_utf8(decode.stdout).unwrap(), sample_jsonl());

    let xencode_jsonl = run_stdin_stdout(
        "ben",
        &[
            "xencode",
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

    let xdecode_jsonl = run_stdin_stdout("ben", &["xdecode"], temp.path(), &xencode_jsonl.stdout);
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
            "xencode",
            "--from-ben",
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
        &["decode", "--from-xben"],
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
            "xencode",
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
    let ben_path = temp.path().join("samples.ben");
    let xz_path = temp.path().join("samples.jsonl.xz");

    fs::write(&jsonl_path, sample_jsonl()).unwrap();

    let encode = run(
        "ben",
        &["encode", jsonl_path.to_str().unwrap(), "--save-all"],
        temp.path(),
    );
    assert_success(&encode);
    assert!(ben_path.exists());

    fs::remove_file(&jsonl_path).unwrap();
    let decode = run("ben", &["decode", ben_path.to_str().unwrap()], temp.path());
    assert_success(&decode);
    assert_eq!(fs::read_to_string(&jsonl_path).unwrap(), sample_jsonl());

    let compress = run(
        "ben",
        &["xz-compress", jsonl_path.to_str().unwrap()],
        temp.path(),
    );
    assert_success(&compress);
    assert!(xz_path.exists());

    fs::remove_file(&jsonl_path).unwrap();
    let decompress = run(
        "ben",
        &["xz-decompress", xz_path.to_str().unwrap()],
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
    // xencode treats a non-.ben input as JSONL, so to force a failure the content must be invalid
    // JSONL rather than merely an unexpected extension.
    fs::write(&bogus_txt, "not valid json\n").unwrap();
    fs::write(&bogus_xz, "not xz").unwrap();

    let xencode = run(
        "ben",
        &["xencode", bogus_txt.to_str().unwrap()],
        temp.path(),
    );
    assert_failure(&xencode);
    assert!(String::from_utf8_lossy(&xencode.stderr).contains("Error:"));

    // decode now defaults to BEN -> JSONL; a JSONL file has no BEN banner, so it fails to decode.
    let decode = run(
        "ben",
        &["decode", bogus_jsonl.to_str().unwrap()],
        temp.path(),
    );
    assert_failure(&decode);
    assert!(String::from_utf8_lossy(&decode.stderr).contains("Error:"));

    // lookup requires --sample-number; omitting it is a clap parse error.
    let read = run(
        "ben",
        &["lookup", bogus_jsonl.to_str().unwrap()],
        temp.path(),
    );
    assert_failure(&read);
    assert!(String::from_utf8_lossy(&read.stderr).contains("sample-number"));

    let xz = run(
        "ben",
        &["xz-decompress", bogus_xz.to_str().unwrap()],
        temp.path(),
    );
    assert_failure(&xz);
    assert!(String::from_utf8_lossy(&xz.stderr)
        .contains("Unsupported file type for xz decompress mode"));

    let bad_xben = run_stdin_stdout("ben", &["xdecode"], temp.path(), b"not-an-xben");
    assert_failure(&bad_xben);
    assert!(String::from_utf8_lossy(&bad_xben.stderr).contains("Error:"));

    let bad_decode_ben = run_stdin_stdout("ben", &["decode"], temp.path(), b"not-a-ben");
    assert_failure(&bad_decode_ben);
    assert!(String::from_utf8_lossy(&bad_decode_ben.stderr).contains("Error:"));

    let bad_decode_xben = run_stdin_stdout(
        "ben",
        &["decode", "--from-xben"],
        temp.path(),
        b"not-an-xben",
    );
    assert_failure(&bad_decode_xben);
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
            "xencode",
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
            &["encode", "--output-file", occupied.to_str().unwrap()],
            temp.path(),
            sample_jsonl().as_bytes(),
        ),
        run_with_stdin(
            "ben",
            &[
                "xencode",
                ben_path.to_str().unwrap(),
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            b"n\n",
        ),
        run_with_stdin(
            "ben",
            &["xencode", "--output-file", occupied.to_str().unwrap()],
            temp.path(),
            sample_jsonl().as_bytes(),
        ),
        run_with_stdin(
            "ben",
            &["decode", "--output-file", occupied.to_str().unwrap()],
            temp.path(),
            b"n\n",
        ),
        run_with_stdin(
            "ben",
            &[
                "xdecode",
                xben_path.to_str().unwrap(),
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            b"n\n",
        ),
        run_with_stdin(
            "ben",
            &["xdecode", "--output-file", occupied.to_str().unwrap()],
            temp.path(),
            b"n\n",
        ),
        run_with_stdin(
            "ben",
            &[
                "lookup",
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
                "xz-decompress",
                xz_path.to_str().unwrap(),
                "--output-file",
                occupied.to_str().unwrap(),
            ],
            temp.path(),
            b"n\n",
        ),
    ] {
        assert_failure(&output);
        assert!(String::from_utf8_lossy(&output.stderr).contains("already"));
    }

    let invalid_ben_to_xben = run(
        "ben",
        &[
            "xencode",
            invalid_ben.to_str().unwrap(),
            "--output-file",
            temp.path().join("bad.xben").to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_failure(&invalid_ben_to_xben);
    assert!(String::from_utf8_lossy(&invalid_ben_to_xben.stderr).contains("Error:"));

    // Empty stdin decodes as BEN by default; with no banner it fails rather than rejecting by type.
    let unsupported_decode = run_stdin_stdout("ben", &["decode"], temp.path(), b"");
    assert_failure(&unsupported_decode);
    assert!(String::from_utf8_lossy(&unsupported_decode.stderr).contains("Error:"));

    let read_too_large = run(
        "ben",
        &[
            "lookup",
            ben_path.to_str().unwrap(),
            "--sample-number",
            "99",
            "--print",
        ],
        temp.path(),
    );
    assert_failure(&read_too_large);
    assert!(String::from_utf8_lossy(&read_too_large.stderr).contains("Error:"));

    let invalid_decode_ben = run(
        "ben",
        &[
            "decode",
            invalid_ben.to_str().unwrap(),
            "--output-file",
            temp.path().join("decoded.jsonl").to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_failure(&invalid_decode_ben);
    assert!(String::from_utf8_lossy(&invalid_decode_ben.stderr).contains("Error:"));

    let invalid_decode_xben = run(
        "ben",
        &[
            "decode",
            invalid_xben.to_str().unwrap(),
            "--output-file",
            temp.path().join("decoded.ben").to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_failure(&invalid_decode_xben);
    assert!(String::from_utf8_lossy(&invalid_decode_xben.stderr).contains("Error:"));

    let invalid_xdecode = run(
        "ben",
        &[
            "xdecode",
            invalid_xben.to_str().unwrap(),
            "--output-file",
            temp.path().join("decoded2.jsonl").to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_failure(&invalid_xdecode);
    assert!(String::from_utf8_lossy(&invalid_xdecode.stderr).contains("Error:"));

    let invalid_xz_decompress = run(
        "ben",
        &[
            "xz-decompress",
            invalid_xz.to_str().unwrap(),
            "--output-file",
            temp.path().join("decoded3.txt").to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_failure(&invalid_xz_decompress);
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
        "ben",
        &[
            "sort-graph",
            graph_path.to_str().unwrap(),
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
        "ben",
        &[
            "canonicalize",
            ben_path.to_str().unwrap(),
            "--output-file",
            canonical_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&canonicalize);

    let relabel = run(
        "ben",
        &[
            "relabel",
            ben_path.to_str().unwrap(),
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
    assert!(canonical_text.contains(r#""assignment":[0,0,1]"#));
    assert!(canonical_text.contains(r#""assignment":[0,1,1]"#));

    let mut relabeled_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&map_relabel_path).unwrap()),
        &mut relabeled_jsonl,
    )
    .unwrap();
    let relabeled_text = String::from_utf8(relabeled_jsonl).unwrap();
    assert!(relabeled_text.contains(r#""assignment":[9,9,4]"#));
}

#[test]
fn reben_cli_rejects_map_referencing_missing_assignment_index() {
    let temp = TempDir::new("reben-bad-map");
    let jsonl_path = temp.path().join("samples.jsonl");
    let ben_path = temp.path().join("samples.jsonl.ben");
    let map_path = temp.path().join("bad_map.json");
    let out_path = temp.path().join("out.ben");

    fs::write(
        &jsonl_path,
        r#"{"assignment":[9,4],"sample":1}
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

    fs::write(
        &map_path,
        r#"{"key":"map","node_permutation_old_to_new":{"0":0,"2":1}}"#,
    )
    .unwrap();

    let relabel = run(
        "ben",
        &[
            "relabel",
            ben_path.to_str().unwrap(),
            "--map-file",
            map_path.to_str().unwrap(),
            "--output-file",
            out_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_failure(&relabel);
    let stderr = String::from_utf8_lossy(&relabel.stderr);
    assert!(
        stderr.contains("Error: BEN relabeling with map")
            && stderr.contains("old index 2")
            && !stderr.contains("panicked"),
        "stderr:\n{stderr}"
    );

    let malformed_map_path = temp.path().join("malformed_map.json");
    fs::write(&malformed_map_path, r#"{"key":"map"}"#).unwrap();
    let malformed = run(
        "ben",
        &[
            "relabel",
            ben_path.to_str().unwrap(),
            "--map-file",
            malformed_map_path.to_str().unwrap(),
            "--output-file",
            out_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_failure(&malformed);
    let stderr = String::from_utf8_lossy(&malformed.stderr);
    assert!(
        stderr.contains("Error: Map file")
            && stderr.contains("node_permutation_old_to_new")
            && !stderr.contains("panicked"),
        "stderr:\n{stderr}"
    );
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
        "ben",
        &[
            "sort-graph",
            graph_path.to_str().unwrap(),
            "--key",
            "GEOID20",
        ],
        temp.path(),
    );
    assert_success(&sort_graph);
    assert!(map_path.exists());

    let canonicalize = run(
        "ben",
        &[
            "canonicalize",
            ben_path.to_str().unwrap(),
            "--n-items",
            "1",
            "--output-file",
            canonical_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&canonicalize);

    let relabel = run(
        "ben",
        &[
            "relabel",
            ben_path.to_str().unwrap(),
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
        "{\"assignment\":[0,0,1],\"sample\":1}\n"
    );

    let mut relabeled_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&map_relabel_path).unwrap()),
        &mut relabeled_jsonl,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(relabeled_jsonl).unwrap(),
        "{\"assignment\":[9,9,4],\"sample\":1}\n"
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
        "ben",
        &[
            "sort-graph",
            graph_path.to_str().unwrap(),
            "--key",
            "GEOID20",
        ],
        temp.path(),
    );
    assert_success(&sort_graph);

    let map_path = temp.path().join("dual_graph_sorted_by_GEOID20_map.json");
    assert!(map_path.exists());

    let canonicalize = run(
        "ben",
        &[
            "canonicalize",
            ben_path.to_str().unwrap(),
            "--output-file",
            canonical_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&canonicalize);

    let relabel = run(
        "ben",
        &[
            "relabel",
            ben_path.to_str().unwrap(),
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
        .contains(r#""assignment":[0,0,1]"#));

    let mut relabeled_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&map_relabel_path).unwrap()),
        &mut relabeled_jsonl,
    )
    .unwrap();
    assert!(String::from_utf8(relabeled_jsonl)
        .unwrap()
        .contains(r#""assignment":[2,1,1]"#));
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
        "ben",
        &[
            "reencode",
            ben_path.to_str().unwrap(),
            "--output-variant",
            "twodelta",
            "--output-file",
            twodelta_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&to_twodelta);

    let twodelta_bytes = fs::read(&twodelta_path).unwrap();
    assert_eq!(&twodelta_bytes[..17], b"TWODELTA BEN FILE");

    let to_mkv = run(
        "ben",
        &[
            "reencode",
            twodelta_path.to_str().unwrap(),
            "--output-variant",
            "mkv-chain",
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
fn reben_cli_can_limit_variant_conversion_to_first_n_items() {
    let temp = TempDir::new("reben-convert-limit");
    let ben_path = temp.path().join("samples.standard.ben");
    let twodelta_path = temp.path().join("samples.twodelta.ben");

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

    let limited_convert = run(
        "ben",
        &[
            "reencode",
            ben_path.to_str().unwrap(),
            "--output-variant",
            "twodelta",
            "--n-items",
            "2",
            "--output-file",
            twodelta_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&limited_convert);

    let twodelta_bytes = fs::read(&twodelta_path).unwrap();
    assert_eq!(&twodelta_bytes[..17], b"TWODELTA BEN FILE");

    let mut converted_jsonl = Vec::new();
    decode_ben_to_jsonl(
        BufReader::new(fs::File::open(&twodelta_path).unwrap()),
        &mut converted_jsonl,
    )
    .unwrap();

    assert_eq!(
        String::from_utf8(converted_jsonl).unwrap(),
        r#"{"assignment":[4,4,9],"sample":1}
{"assignment":[4,4,9],"sample":2}
"#
    );
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
        "ben",
        &[
            "canonicalize",
            ben_path.to_str().unwrap(),
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
    assert!(canonical_text.contains(r#""assignment":[0,0,1]"#));
    assert!(canonical_text.contains(r#""assignment":[0,1,1]"#));
}

#[test]
fn reben_cli_generates_map_from_dual_graph_and_reports_invalid_flag_combinations() {
    let temp = TempDir::new("reben-more");
    let graph_path = temp.path().join("dualgraph.json");
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
        "ben",
        &[
            "relabel",
            ben_path.to_str().unwrap(),
            "--key",
            "GEOID20",
            "--dualgraph",
            graph_path.to_str().unwrap(),
            "--output-file",
            relabeled_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&relabel);
    assert!(temp
        .path()
        .join("dualgraph_sorted_by_GEOID20_map.json")
        .exists());

    let generated_graph = temp.path().join("dualgraph_sorted_by_GEOID20.json");
    let generated_map = temp.path().join("dualgraph_sorted_by_GEOID20_map.json");
    let both = run(
        "ben",
        &[
            "relabel",
            ben_path.to_str().unwrap(),
            "--key",
            "GEOID20",
            "--dualgraph",
            graph_path.to_str().unwrap(),
            "--map-file",
            generated_map.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_failure(&both);
    assert!(String::from_utf8_lossy(&both.stderr)
        .contains("Cannot provide both a map file and a sorting option"));

    let missing_dual_graph = run(
        "ben",
        &["relabel", ben_path.to_str().unwrap(), "--key", "GEOID20"],
        temp.path(),
    );
    assert_failure(&missing_dual_graph);
    assert!(
        String::from_utf8_lossy(&missing_dual_graph.stderr).contains("No dual-graph file provided")
    );

    let sorted_json: Value =
        serde_json::from_str(&fs::read_to_string(generated_graph).unwrap()).unwrap();
    assert_eq!(sorted_json["nodes"][0]["GEOID20"], "A");
}

#[test]
fn reben_cli_supports_rcm_ordering() {
    let temp = TempDir::new("reben-orderings");
    let graph_path = temp.path().join("dualgraph.json");
    let rcm_path = temp.path().join("rcm.json");

    fs::write(&graph_path, sample_graph()).unwrap();

    let rcm = run(
        "ben",
        &[
            "sort-graph",
            graph_path.to_str().unwrap(),
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
        .join("dualgraph_sorted_by_reverse-cuthill-mckee_map.json")
        .exists());

    let rcm_json: Value = serde_json::from_str(&fs::read_to_string(&rcm_path).unwrap()).unwrap();
    assert!(!rcm_json["nodes"].as_array().unwrap().is_empty());
}

#[test]
fn reben_cli_supports_multi_level_cluster_ordering() {
    let temp = TempDir::new("reben-mlc");
    let graph_path = temp.path().join("dualgraph.json");
    let mlc_path = temp.path().join("mlc.json");

    fs::write(&graph_path, sample_graph()).unwrap();

    let mlc = run(
        "ben",
        &[
            "sort-graph",
            graph_path.to_str().unwrap(),
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
        .join("dualgraph_sorted_by_multi-level-cluster_map.json")
        .exists());

    let mlc_json: Value = serde_json::from_str(&fs::read_to_string(&mlc_path).unwrap()).unwrap();
    assert!(!mlc_json["nodes"].as_array().unwrap().is_empty());
}

#[test]
fn pcben_decodes_committed_foreign_pcompress_fixture() {
    // `interop.pcompress` was minted by the real PCompress implementation (the `pcompress`
    // crates.io dependency), so this pins the foreign-format interop contract: bytes produced by
    // genuine PCompress must keep converting to BEN that decodes back to the canonical JSONL.
    // The expected output is the committed `source.jsonl`, whose one-based ids are the fixture's
    // zero-based ids shifted by the bridge.
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("v1.0.0");
    let expected = fs::read_to_string(fixtures.join("source.jsonl")).unwrap();

    let temp = TempDir::new("pcben-interop");
    let ben_path = temp.path().join("interop.ben");
    let pc_to_ben = run(
        "ben",
        &[
            "pcompress",
            "to-ben",
            fixtures.join("interop.pcompress").to_str().unwrap(),
            "--output-file",
            ben_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&pc_to_ben);

    let mut jsonl = Vec::new();
    decode_ben_to_jsonl(fs::File::open(&ben_path).unwrap(), &mut jsonl).unwrap();
    assert_eq!(
        String::from_utf8(jsonl).unwrap(),
        expected,
        "foreign pcompress fixture no longer converts to the canonical ensemble"
    );
}

#[test]
fn pben_cli_converts_between_formats() {
    let temp = TempDir::new("pcben");
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
        "ben",
        &[
            "pcompress",
            "from-ben",
            ben_path.to_str().unwrap(),
            "--output-file",
            pc_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&ben_to_pc);
    assert!(pc_path.exists());

    let pc_to_ben = run(
        "ben",
        &[
            "pcompress",
            "to-ben",
            pc_path.to_str().unwrap(),
            "--output-file",
            roundtrip_ben_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_success(&pc_to_ben);

    let pc_to_xben = run(
        "ben",
        &[
            "pcompress",
            "to-xben",
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
        &["xdecode", xben_path.to_str().unwrap(), "--print"],
        temp.path(),
    );
    assert_success(&xdecode);
    let printed = String::from_utf8_lossy(&xdecode.stdout);
    assert!(printed.contains(r#""assignment":[2,2,3]"#));
}

#[test]
fn bendl_cli_create_inspect_extract_append_roundtrip() {
    let temp = TempDir::new("bendl-workflow");

    // Seed: a .ben assignment file to wrap.
    let jsonl_path = temp.path().join("samples.jsonl");
    let ben_path = temp.path().join("samples.ben");
    fs::write(&jsonl_path, sample_jsonl()).unwrap();
    assert_success(&run(
        "ben",
        &[
            "encode",
            jsonl_path.to_str().unwrap(),
            "--output-file",
            ben_path.to_str().unwrap(),
            "--save-all",
            "--overwrite",
        ],
        temp.path(),
    ));

    // Seed: a graph.json file to front-load as an asset.
    let graph_path = temp.path().join("graph.json");
    fs::write(&graph_path, sample_graph()).unwrap();

    // Seed: a small metadata.json file.
    let metadata_path = temp.path().join("metadata.json");
    fs::write(&metadata_path, r#"{"note":"hello"}"#).unwrap();

    // `bendl create`: build a finalized bundle.
    let bundle_path = temp.path().join("out.bendl");
    let create = run(
        "bendl",
        &[
            "create",
            "--input",
            ben_path.to_str().unwrap(),
            "--output",
            bundle_path.to_str().unwrap(),
            "--graph",
            graph_path.to_str().unwrap(),
            "--metadata",
            metadata_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&create);
    assert!(bundle_path.exists());

    // `bendl inspect`: header should report both assets and complete=true.
    let inspect = run(
        "bendl",
        &["inspect", bundle_path.to_str().unwrap()],
        temp.path(),
    );
    assert_success(&inspect);
    let inspect_out = String::from_utf8_lossy(&inspect.stdout);
    assert!(inspect_out.contains("finalized:         true"));
    assert!(inspect_out.contains("assignment_format: ben"));
    assert!(inspect_out.contains("graph.json"));
    assert!(inspect_out.contains("metadata.json"));

    // `bendl extract --stream`: recover the original .ben bytes exactly.
    let recovered_ben = temp.path().join("recovered.ben");
    let extract_stream = run(
        "bendl",
        &[
            "extract",
            bundle_path.to_str().unwrap(),
            "--stream",
            "--output",
            recovered_ben.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&extract_stream);
    assert_eq!(
        fs::read(&recovered_ben).unwrap(),
        fs::read(&ben_path).unwrap()
    );

    // `bendl extract --asset graph.json`: recover the decoded graph JSON.
    let recovered_graph = temp.path().join("recovered-graph.json");
    let extract_asset = run(
        "bendl",
        &[
            "extract",
            bundle_path.to_str().unwrap(),
            "--asset",
            "graph.json",
            "--output",
            recovered_graph.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&extract_asset);
    assert_eq!(
        fs::read_to_string(&recovered_graph).unwrap(),
        sample_graph()
    );

    // `bendl append`: add a custom asset to the already-finalized bundle.
    let custom_path = temp.path().join("notes.txt");
    fs::write(&custom_path, b"bundle notes").unwrap();
    let append = run(
        "bendl",
        &[
            "append",
            bundle_path.to_str().unwrap(),
            "--asset",
            &format!("notes={}", custom_path.display()),
        ],
        temp.path(),
    );
    assert_success(&append);

    // Inspect again: new asset should be present, old assets preserved.
    let inspect2 = run(
        "bendl",
        &["inspect", bundle_path.to_str().unwrap()],
        temp.path(),
    );
    assert_success(&inspect2);
    let inspect2_out = String::from_utf8_lossy(&inspect2.stdout);
    assert!(inspect2_out.contains("graph.json"));
    assert!(inspect2_out.contains("metadata.json"));
    assert!(inspect2_out.contains("notes"));

    // Stream bytes should still match after append.
    let recovered_ben2 = temp.path().join("recovered2.ben");
    let extract_stream2 = run(
        "bendl",
        &[
            "extract",
            bundle_path.to_str().unwrap(),
            "--stream",
            "--output",
            recovered_ben2.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&extract_stream2);
    assert_eq!(
        fs::read(&recovered_ben2).unwrap(),
        fs::read(&ben_path).unwrap()
    );

    // Appending a second graph.json is rejected: singleton constraint.
    let append_duplicate = run(
        "bendl",
        &[
            "append",
            bundle_path.to_str().unwrap(),
            "--graph",
            graph_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_failure(&append_duplicate);
}

#[test]
fn bendl_cli_remove_reclaims_bytes_and_compact_is_stable() {
    let temp = TempDir::new("bendl-remove");

    // Seed: a .ben assignment file plus a large incompressible custom asset.
    let jsonl_path = temp.path().join("samples.jsonl");
    let ben_path = temp.path().join("samples.ben");
    fs::write(&jsonl_path, sample_jsonl()).unwrap();
    assert_success(&run(
        "ben",
        &[
            "encode",
            jsonl_path.to_str().unwrap(),
            "--output-file",
            ben_path.to_str().unwrap(),
            "--save-all",
            "--overwrite",
        ],
        temp.path(),
    ));
    // xorshift32 output is effectively incompressible, so the blob genuinely occupies bytes
    // even though `bendl create` stores large assets xz-compressed by default.
    let mut state = 0x1234_5678u32;
    let blob: Vec<u8> = (0..65536u32)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 24) as u8
        })
        .collect();
    let blob_path = temp.path().join("bloat.bin");
    fs::write(&blob_path, &blob).unwrap();

    let bundle_path = temp.path().join("out.bendl");
    assert_success(&run(
        "bendl",
        &[
            "create",
            "--input",
            ben_path.to_str().unwrap(),
            "--output",
            bundle_path.to_str().unwrap(),
            "--asset",
            &format!("bloat.bin={}", blob_path.display()),
            "--overwrite",
        ],
        temp.path(),
    ));
    let bloated = fs::metadata(&bundle_path).unwrap().len();
    assert!(bloated > 60_000, "blob should dominate the file size");

    // `bendl remove` drops the asset AND reclaims its bytes (auto-compaction).
    assert_success(&run(
        "bendl",
        &[
            "remove",
            bundle_path.to_str().unwrap(),
            "--asset",
            "bloat.bin",
        ],
        temp.path(),
    ));
    let after = fs::metadata(&bundle_path).unwrap().len();
    assert!(
        after + 60_000 < bloated,
        "removal must reclaim the blob's bytes ({bloated} -> {after})"
    );

    // The bundle stays finalized, the asset is gone, and the stream is byte-identical.
    let inspect = run(
        "bendl",
        &["inspect", bundle_path.to_str().unwrap()],
        temp.path(),
    );
    assert_success(&inspect);
    let inspect_out = String::from_utf8_lossy(&inspect.stdout);
    assert!(!inspect_out.contains("bloat.bin"));
    assert!(inspect_out.contains("finalized:         true"));

    let recovered = temp.path().join("recovered.ben");
    assert_success(&run(
        "bendl",
        &[
            "extract",
            bundle_path.to_str().unwrap(),
            "--stream",
            "--output",
            recovered.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    ));
    assert_eq!(fs::read(&recovered).unwrap(), fs::read(&ben_path).unwrap());

    // Removing a missing asset fails and leaves the file byte-identical (the command is
    // atomic: nothing commits unless every removal succeeds).
    let before = fs::read(&bundle_path).unwrap();
    let missing = run(
        "bendl",
        &[
            "remove",
            bundle_path.to_str().unwrap(),
            "--asset",
            "nope.bin",
        ],
        temp.path(),
    );
    assert_failure(&missing);
    assert_eq!(fs::read(&bundle_path).unwrap(), before);

    // Standalone `bendl compact` on an already-compact bundle is byte-stable.
    assert_success(&run(
        "bendl",
        &["compact", bundle_path.to_str().unwrap()],
        temp.path(),
    ));
    assert_eq!(fs::read(&bundle_path).unwrap(), before);
}

// =====================================================================
// `ben encode --graph` and `ben xencode --graph`
// =====================================================================

#[test]
fn ben_encode_graph_requires_input_file_not_stdin() {
    // `--graph` is structurally incompatible with stdin input because the output container has
    // to seek to patch the header. The CLI must reject the bad combination explicitly.
    let temp = TempDir::new("ben-encode-graph-stdin");
    let graph_path = temp.path().join("graph.json");
    fs::write(&graph_path, sample_graph()).unwrap();

    let out = run(
        "ben",
        &["encode", "--graph", graph_path.to_str().unwrap()],
        temp.path(),
    );
    assert_failure(&out);
    let msg = String::from_utf8_lossy(&out.stderr);
    assert!(
        msg.contains("--graph") && msg.contains("input file"),
        "expected '--graph requires an input file' error, got stderr: {msg}"
    );
}

#[test]
fn ben_encode_graph_rejects_combination_with_print() {
    // `--print` writes to stdout; bendl output requires a seekable file. Combination is invalid
    // and must be rejected explicitly.
    let temp = TempDir::new("ben-encode-graph-print");
    let jsonl_path = temp.path().join("samples.jsonl");
    fs::write(&jsonl_path, sample_jsonl()).unwrap();
    let graph_path = temp.path().join("graph.json");
    fs::write(&graph_path, sample_graph()).unwrap();

    let out = run(
        "ben",
        &[
            "encode",
            jsonl_path.to_str().unwrap(),
            "--graph",
            graph_path.to_str().unwrap(),
            "--print",
        ],
        temp.path(),
    );
    assert_failure(&out);
    let msg = String::from_utf8_lossy(&out.stderr);
    assert!(
        msg.contains("--graph") && msg.contains("--print"),
        "expected '--graph is incompatible with --print' error, got stderr: {msg}"
    );
}

#[test]
fn ben_encode_graph_happy_path_produces_bendl() {
    // Happy path for `ben encode --graph`: produces a finalized .bendl whose decoded
    // stream round-trips the input JSONL and whose graph asset matches the source.
    let temp = TempDir::new("ben-encode-graph-happy");
    let jsonl_path = temp.path().join("samples.jsonl");
    fs::write(&jsonl_path, sample_jsonl()).unwrap();
    let graph_path = temp.path().join("graph.json");
    fs::write(&graph_path, sample_graph()).unwrap();
    let out_path = temp.path().join("out.bendl");

    let encode = run(
        "ben",
        &[
            "encode",
            jsonl_path.to_str().unwrap(),
            "--output-file",
            out_path.to_str().unwrap(),
            "--graph",
            graph_path.to_str().unwrap(),
            "--save-all",
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&encode);
    assert!(out_path.exists());

    // Recover the embedded BEN stream and confirm it decodes back to the canonical JSONL.
    let stream_path = temp.path().join("recovered.ben");
    let extract_stream = run(
        "bendl",
        &[
            "extract",
            out_path.to_str().unwrap(),
            "--stream",
            "--output",
            stream_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&extract_stream);

    let decoded_path = temp.path().join("decoded.jsonl");
    let decode = run(
        "ben",
        &[
            "decode",
            stream_path.to_str().unwrap(),
            "--output-file",
            decoded_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&decode);
    assert_eq!(fs::read_to_string(&decoded_path).unwrap(), sample_jsonl());

    // The graph asset should be embedded byte-equal.
    let recovered_graph = temp.path().join("recovered-graph.json");
    let extract_graph = run(
        "bendl",
        &[
            "extract",
            out_path.to_str().unwrap(),
            "--asset",
            "graph.json",
            "--output",
            recovered_graph.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&extract_graph);
    assert_eq!(
        fs::read_to_string(&recovered_graph).unwrap(),
        sample_graph()
    );
}

#[test]
fn ben_xencode_graph_requires_input_file_not_stdin() {
    let temp = TempDir::new("ben-xencode-graph-stdin");
    let graph_path = temp.path().join("graph.json");
    fs::write(&graph_path, sample_graph()).unwrap();

    let out = run(
        "ben",
        &["xencode", "--graph", graph_path.to_str().unwrap()],
        temp.path(),
    );
    assert_failure(&out);
    let msg = String::from_utf8_lossy(&out.stderr);
    assert!(
        msg.contains("--graph") && msg.contains("input file"),
        "expected '--graph requires an input file' error, got stderr: {msg}"
    );
}

#[test]
fn ben_xencode_graph_rejects_combination_with_print() {
    let temp = TempDir::new("ben-xencode-graph-print");
    let jsonl_path = temp.path().join("samples.jsonl");
    fs::write(&jsonl_path, sample_jsonl()).unwrap();
    let graph_path = temp.path().join("graph.json");
    fs::write(&graph_path, sample_graph()).unwrap();

    let out = run(
        "ben",
        &[
            "xencode",
            jsonl_path.to_str().unwrap(),
            "--graph",
            graph_path.to_str().unwrap(),
            "--print",
        ],
        temp.path(),
    );
    assert_failure(&out);
    let msg = String::from_utf8_lossy(&out.stderr);
    assert!(
        msg.contains("--graph") && msg.contains("--print"),
        "expected '--graph is incompatible with --print' error, got stderr: {msg}"
    );
}

#[test]
fn ben_xencode_graph_with_ben_input_round_trips() {
    // The `--graph` xencode handler dispatches on input extension: a `.ben` input takes the
    // `encode_ben_to_xben` path (cli/ben/bundle.rs line 127), a `.jsonl` input takes the
    // `encode_jsonl_to_xben` path. The happy-path test below only covers the `.jsonl` arm;
    // this companion exercises the `.ben` arm.
    let temp = TempDir::new("ben-xencode-graph-ben-input");
    let jsonl_path = temp.path().join("samples.jsonl");
    fs::write(&jsonl_path, sample_jsonl()).unwrap();

    // Encode JSONL to a BEN file first; this is what we'll feed into xencode.
    let ben_path = temp.path().join("samples.ben");
    let encode_ben = run(
        "ben",
        &[
            "encode",
            jsonl_path.to_str().unwrap(),
            "--output-file",
            ben_path.to_str().unwrap(),
            "--save-all",
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&encode_ben);

    let graph_path = temp.path().join("graph.json");
    fs::write(&graph_path, sample_graph()).unwrap();
    let out_path = temp.path().join("out.bendl");

    let xencode = run(
        "ben",
        &[
            "xencode",
            ben_path.to_str().unwrap(),
            "--output-file",
            out_path.to_str().unwrap(),
            "--graph",
            graph_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&xencode);
    assert!(out_path.exists());

    // Round-trip: extract the XBEN stream, decode it back to JSONL, compare to the original.
    let recovered_xben = temp.path().join("recovered.xben");
    let extract = run(
        "bendl",
        &[
            "extract",
            out_path.to_str().unwrap(),
            "--stream",
            "--output",
            recovered_xben.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&extract);

    let decoded_path = temp.path().join("decoded.jsonl");
    let decode = run(
        "ben",
        &[
            "xdecode",
            recovered_xben.to_str().unwrap(),
            "--output-file",
            decoded_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&decode);
    assert_eq!(fs::read_to_string(&decoded_path).unwrap(), sample_jsonl());
}

#[test]
fn ben_encode_graph_rejects_missing_graph_file() {
    // A graph path that does not exist must surface a clean error, not a panic.
    let temp = TempDir::new("ben-encode-graph-missing-file");
    let jsonl_path = temp.path().join("samples.jsonl");
    fs::write(&jsonl_path, sample_jsonl()).unwrap();
    let nonexistent_graph = temp.path().join("does-not-exist.json");
    let out_path = temp.path().join("out.bendl");

    let out = run(
        "ben",
        &[
            "encode",
            jsonl_path.to_str().unwrap(),
            "--output-file",
            out_path.to_str().unwrap(),
            "--graph",
            nonexistent_graph.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_failure(&out);
}

#[test]
fn ben_encode_graph_refuses_to_overwrite_existing_file_without_flag() {
    // Without --overwrite, an existing output path must be preserved.
    let temp = TempDir::new("ben-encode-graph-overwrite-guard");
    let jsonl_path = temp.path().join("samples.jsonl");
    fs::write(&jsonl_path, sample_jsonl()).unwrap();
    let graph_path = temp.path().join("graph.json");
    fs::write(&graph_path, sample_graph()).unwrap();
    let out_path = temp.path().join("out.bendl");
    fs::write(&out_path, b"prior contents").unwrap();

    let out = run(
        "ben",
        &[
            "encode",
            jsonl_path.to_str().unwrap(),
            "--output-file",
            out_path.to_str().unwrap(),
            "--graph",
            graph_path.to_str().unwrap(),
        ],
        temp.path(),
    );
    assert_failure(&out);
    // The prior file must remain untouched.
    assert_eq!(fs::read(&out_path).unwrap(), b"prior contents");
}

#[test]
fn ben_xencode_graph_happy_path_produces_bendl() {
    let temp = TempDir::new("ben-xencode-graph-happy");
    let jsonl_path = temp.path().join("samples.jsonl");
    fs::write(&jsonl_path, sample_jsonl()).unwrap();
    let graph_path = temp.path().join("graph.json");
    fs::write(&graph_path, sample_graph()).unwrap();
    let out_path = temp.path().join("out.bendl");

    let encode = run(
        "ben",
        &[
            "xencode",
            jsonl_path.to_str().unwrap(),
            "--output-file",
            out_path.to_str().unwrap(),
            "--graph",
            graph_path.to_str().unwrap(),
            "--save-all",
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&encode);
    assert!(out_path.exists());

    // Recover the embedded XBEN stream and decode it to confirm round-trip.
    let stream_path = temp.path().join("recovered.xben");
    let extract_stream = run(
        "bendl",
        &[
            "extract",
            out_path.to_str().unwrap(),
            "--stream",
            "--output",
            stream_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&extract_stream);

    let decoded_path = temp.path().join("decoded.jsonl");
    let decode = run(
        "ben",
        &[
            "xdecode",
            stream_path.to_str().unwrap(),
            "--output-file",
            decoded_path.to_str().unwrap(),
            "--overwrite",
        ],
        temp.path(),
    );
    assert_success(&decode);
    assert_eq!(fs::read_to_string(&decoded_path).unwrap(), sample_jsonl());
}
