"""Smoke test for a freshly built binary_ensemble wheel.

Run inside a clean venv that has the wheel (and nothing else from this repo) installed: imports
the extension module and round-trips a tiny ensemble through encode/decode. Catches wheels that
build but cannot load (bad abi3 configuration, missing symbols) or that load but cannot reach the
Rust core, before they are published.
"""

import json
import pathlib
import tempfile

import binary_ensemble as be

LINES = [
    {"assignment": [1, 1, 2, 2], "sample": 1},
    {"assignment": [2, 2, 1, 1], "sample": 2},
    {"assignment": [2, 2, 1, 1], "sample": 3},
]


def main() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = pathlib.Path(tmp)
        src = tmp_path / "src.jsonl"
        ben = tmp_path / "out.ben"
        back = tmp_path / "round.jsonl"

        # Write bytes directly so the fixture is byte-identical to the LF JSONL the library emits.
        # Python's text-mode write translates "\n" to the platform line ending (CRLF on Windows),
        # which would break the byte-level round-trip comparison below.
        payload = "".join(json.dumps(line, separators=(",", ":")) + "\n" for line in LINES)
        src.write_bytes(payload.encode("utf-8"))
        be.encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")
        be.decode_ben_to_jsonl(ben, back, overwrite=True)

        assert src.read_bytes() == back.read_bytes(), (
            f"wheel round-trip mismatch:\n{src.read_bytes()!r}\n!=\n{back.read_bytes()!r}"
        )

    print("wheel smoke test passed")


if __name__ == "__main__":
    main()
