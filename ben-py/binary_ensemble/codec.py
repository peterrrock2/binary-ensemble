"""Whole-file stream/JSONL transforms.

These helpers convert entire files in one call, without an iterator: JSONL ↔
BEN ↔ XBEN. For streaming sample-by-sample access use
:class:`binary_ensemble.stream.BenDecoder`; for the single-file bundle format
use :mod:`binary_ensemble.bundle`.
"""

from __future__ import annotations

from binary_ensemble._core import (
    decode_ben_to_jsonl,
    decode_xben_to_ben,
    decode_xben_to_jsonl,
    encode_ben_to_xben,
    encode_jsonl_to_ben,
    encode_jsonl_to_xben,
)

__all__ = [
    "encode_jsonl_to_ben",
    "encode_jsonl_to_xben",
    "encode_ben_to_xben",
    "decode_ben_to_jsonl",
    "decode_xben_to_jsonl",
    "decode_xben_to_ben",
]
