"""Binary Ensemble (BEN/XBEN) Python API.

The public surface mirrors the CLI's ``ben`` vs ``bendl`` split:

- :mod:`binary_ensemble.bundle` — the recommended single-file ``.bendl`` format:
  :class:`~binary_ensemble.bundle.BendlEncoder`,
  :class:`~binary_ensemble.bundle.BendlDecoder`, and
  :func:`~binary_ensemble.bundle.compress_stream`.
- :mod:`binary_ensemble.stream` — plain BEN/XBEN streams:
  :class:`~binary_ensemble.stream.BenEncoder`,
  :class:`~binary_ensemble.stream.BenDecoder`.
- :mod:`binary_ensemble.codec` — whole-file JSONL ↔ BEN ↔ XBEN transforms.
- :mod:`binary_ensemble.graph` — graph reordering utilities.

All public symbols are re-exported here for convenience.
"""

from binary_ensemble import bundle, codec, graph, stream
from binary_ensemble.bundle import BendlDecoder, BendlEncoder, compress_stream
from binary_ensemble.codec import (
    decode_ben_to_jsonl,
    decode_xben_to_ben,
    decode_xben_to_jsonl,
    encode_ben_to_xben,
    encode_jsonl_to_ben,
    encode_jsonl_to_xben,
)
from binary_ensemble.stream import BenDecoder, BenEncoder

__all__ = [
    # Submodules
    "stream",
    "bundle",
    "codec",
    "graph",
    # Bundle (recommended)
    "BendlEncoder",
    "BendlDecoder",
    "compress_stream",
    # Stream
    "BenEncoder",
    "BenDecoder",
    # Codec
    "encode_jsonl_to_ben",
    "encode_jsonl_to_xben",
    "encode_ben_to_xben",
    "decode_ben_to_jsonl",
    "decode_xben_to_jsonl",
    "decode_xben_to_ben",
]
