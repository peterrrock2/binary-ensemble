"""Plain BEN/XBEN stream encoding and decoding.

``BenEncoder`` writes a plain ``.ben`` stream; ``BenDecoder`` iterates a plain
``.ben`` / ``.xben`` stream. Both are stream-only: opening a decoder on a
``.bendl`` bundle, or trying to read bundle assets, raises and points you at
:mod:`binary_ensemble.bundle`. For the recommended single-file bundle format,
use :class:`binary_ensemble.bundle.BendlEncoder` /
:class:`binary_ensemble.bundle.BendlDecoder`.
"""

from __future__ import annotations

from binary_ensemble._core import BenDecoder, BenEncoder

__all__ = ["BenEncoder", "BenDecoder"]
