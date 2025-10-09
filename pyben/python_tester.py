import pyben
from pyben import read
import subprocess
from pathlib import Path
import os, shutil, sys

if __name__ == "__main__":
    # with pyben.PyBenEncoder(
    #     "../example/small_example_re_encode.jsonl.ben", overwrite=True
    # ) as encoder:

    out_path = Path("../example/small_example_re_encode.jsonl.ben")
    # encoder = pyben.PyBenEncoder(out_path, overwrite=True)
    for line in pyben.PyBenDecoder(
        "../example/small_example.jsonl.xben", mode="xben"
    ).subsample_indices([1, 1, 2, 3, 5]):
        print(line)
        # if line is not None:
        #     encoder.write(line)
    for i, line in enumerate(
        pyben.PyBenDecoder("../example/small_example.jsonl.xben", mode="xben")
    ):
        print(f"Line {i+1}: {line}")
        if i >= 4:
            break

    # encoder.close()

    in_file = str(
        Path(
            "/mnt/efs/h/Dropbox/MADLAB/Git_Repos/peter/binary-ensemble-pyben/example/small_example.jsonl.ben"
        ).resolve()
    )
    out_file = str(
        Path(
            "/mnt/efs/h/Dropbox/MADLAB/Git_Repos/peter/binary-ensemble-pyben/example/small_example_re_encode.jsonl.ben"
        ).resolve()
    )

    # for i in range(1, 5):
    #     print(read.read_single_assignment(in_file, i))

    cmp_bin = shutil.which("cmp") or "/usr/bin/cmp"

    # IMPORTANT: pass each token as a separate list element
    cmd = [cmp_bin, "-s", in_file, out_file]

    res = subprocess.run(cmd, capture_output=True, text=True, check=False)

    if res.returncode == 1:
        print("Files differ")
