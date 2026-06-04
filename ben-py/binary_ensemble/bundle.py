"""The ``.bendl`` bundle format — the recommended single-file container.

A bundle wraps a BEN/XBEN assignment stream together with front-loaded assets: a
dual ``graph.json``, a ``node_permutation_map.json``, a ``metadata.json``, and
arbitrary custom blobs. :class:`BendlEncoder` writes one; :class:`BendlDecoder`
reads and iterates one.

Typical write::

    with BendlEncoder(path, overwrite=True) as enc:
        enc.add_graph(graph, preprocess_method="rcm")   # None => store raw
        enc.add_metadata({"seed": 1234})
        with enc.stream("ben") as stream:
            for assignment in chain:
                stream.write(assignment)

Typical read::

    dec = BendlDecoder(path)
    graph = dec.read_graph()
    for assignment in dec:
        ...
"""

from __future__ import annotations

import json
import os
import tempfile
from typing import Any, Optional, Union

from binary_ensemble._core import BendlDecoder
from binary_ensemble._core import BendlEncoder as _CoreBendlEncoder
from binary_ensemble._core import recompress_bundle as _recompress_bundle
from binary_ensemble._core import relabel_bundle as _relabel_bundle

__all__ = ["BendlEncoder", "BendlDecoder", "compress_stream", "relabel_bundle"]


def _atomic_or_out(transform, path, out_file, in_place, suffix=".bendl"):
    """Shared in_place-swap / out_file dispatch for whole-bundle transforms.

    ``transform(src, dst, overwrite)`` writes the result. Exactly one of
    ``in_place`` / ``out_file`` must be given.
    """
    if in_place and out_file is not None:
        raise ValueError("pass either in_place=True or out_file, not both")
    if not in_place and out_file is None:
        raise ValueError("pass either in_place=True or out_file")

    if in_place:
        directory = os.path.dirname(os.path.abspath(os.fspath(path)))
        fd, tmp = tempfile.mkstemp(suffix=suffix, dir=directory)
        os.close(fd)
        try:
            transform(path, tmp, True)
            os.replace(tmp, path)
        except BaseException:
            if os.path.exists(tmp):
                os.remove(tmp)
            raise
    else:
        transform(path, out_file, False)


def _coerce_bytes(payload: Union[bytes, bytearray, memoryview, str]) -> bytes:
    """Coerce an ``add_asset`` payload to bytes (``str`` is UTF-8 encoded)."""
    if isinstance(payload, str):
        return payload.encode("utf-8")
    if isinstance(payload, (bytes, bytearray, memoryview)):
        return bytes(payload)
    raise TypeError(f"asset payload must be bytes or str, got {type(payload).__name__}")


class BendlEncoder:
    """Writer for a ``.bendl`` bundle (create mode) or an asset appender (append mode).

    In create mode (the constructor), assets may be added before or after a
    single-use ``stream()``. You do **not** need to use ``BendlEncoder`` itself as
    a context manager: closing the ``stream()`` context finalizes the bundle, so
    the common pattern is::

        enc = BendlEncoder(path, overwrite=True)
        graph = enc.add_graph(my_graph)          # MLC-reordered by default
        with enc.stream("ben") as stream:        # only the stream needs ``with``
            for assignment in chain:
                stream.write(assignment)
        # bundle is finalized here

    The encoder is still usable as a context manager if you prefer, and that is
    the easy way to finalize an *assets-only* bundle (one written with no
    ``stream()``): either ``with BendlEncoder(...) as enc: ...`` or an explicit
    :meth:`close`. In append mode (:meth:`append`), an existing finalized bundle
    is grown with new assets and ``stream()`` is unavailable.
    """

    def __init__(self, file_path, overwrite: bool = False) -> None:
        self._enc = _CoreBendlEncoder(file_path, overwrite=overwrite)

    @classmethod
    def append(cls, file_path) -> "BendlEncoder":
        """Open an existing *finalized* bundle to append new assets.

        ``stream()`` is unavailable in append mode; each ``add_*`` commits
        immediately.
        """
        self = cls.__new__(cls)
        self._enc = _CoreBendlEncoder.append(file_path)
        return self

    def add_graph(
        self, graph: Any, sort: Optional[str] = "mlc", key: Optional[str] = None
    ) -> Any:
        """Embed the dual ``graph.json`` and return the (possibly reordered) graph.

        ``sort`` selects how nodes are ordered and defaults to ``"mlc"`` (so the
        graph is reordered for better compression):

        - ``"mlc"`` — multi-level clustering,
        - ``"rcm"`` — reverse Cuthill-McKee,
        - ``"key"`` — sort by the node attribute named in ``key`` (e.g.
          ``sort="key", key="GEOID"``; ``key="id"`` sorts by the NetworkX node id),
        - ``None`` — store the graph as-is, with no permutation map.

        When reordering, both ``graph.json`` and ``node_permutation_map.json`` are
        stored and the reordered graph is returned so the chain runs on that
        ordering. Reordering is pre-stream only; a raw graph (``sort=None``) may
        also be attached post-stream / in append mode. ``key`` is only valid with
        ``sort="key"``.

        The graph is returned as a NetworkX graph (matching
        :meth:`BendlDecoder.read_graph`), so its node order is the order the
        chain should write assignments in.
        """
        return self._enc.add_graph(graph, sort, key)

    def add_metadata(self, metadata: Any) -> None:
        """Embed the canonical ``metadata.json`` asset (a dict/list, bytes, or path)."""
        self._enc.add_metadata(metadata)

    def add_asset(
        self,
        name: str,
        payload: Union[bytes, bytearray, memoryview, str],
        content_type: str,
    ) -> None:
        """Embed a custom asset under ``name``.

        ``content_type`` is ``"json"`` (payload must be valid UTF-8 JSON; the
        decoder will auto-parse it) or ``"text"`` (payload must be valid UTF-8).
        """
        data = _coerce_bytes(payload)
        if content_type == "json":
            try:
                json.loads(data.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                raise ValueError(
                    f"content_type='json' requires valid UTF-8 JSON: {exc}"
                ) from exc
        elif content_type == "text":
            try:
                data.decode("utf-8")
            except UnicodeDecodeError as exc:
                raise ValueError(
                    f"content_type='text' requires valid UTF-8: {exc}"
                ) from exc
        else:
            raise ValueError(
                f"content_type must be 'json' or 'text', got {content_type!r}"
            )
        self._enc.add_asset(name, data, content_type)

    def stream(self, format: str = "ben", variant: Optional[str] = None):
        """Open the single-use assignment stream context manager.

        Only ``"ben"`` is accepted; produce XBEN bundles via
        :func:`compress_stream`. ``variant`` selects the BEN variant
        (default ``"twodelta"``).
        """
        return self._enc.stream(format, variant)

    def close(self) -> None:
        """Finalize (create mode) or finish (append mode) the bundle. Idempotent."""
        self._enc.close()

    def __enter__(self) -> "BendlEncoder":
        return self

    def __exit__(self, exc_type, exc, tb) -> bool:
        self.close()
        return False


def compress_stream(
    path,
    out_file=None,
    in_place: bool = False,
) -> None:
    """Recompress a bundle's embedded BEN stream to XBEN, preserving every asset.

    Provide exactly one of ``in_place=True`` (recompress to a temp file and
    atomically swap it over ``path``) or ``out_file`` (write a new bundle).
    Passing both, or neither, raises.

    All assets (graph, metadata, node_permutation_map, custom blobs) are
    preserved by decoded payload, name, type, and JSON flag; storage compression
    is normalized to the writer's default policy. An assets-only bundle (empty
    stream) recompresses to an empty XBEN bundle.
    """
    _atomic_or_out(
        lambda src, dst, overwrite: _recompress_bundle(src, dst, overwrite=overwrite),
        path,
        out_file,
        in_place,
    )


def relabel_bundle(
    path,
    out_file=None,
    sort: str = "mlc",
    key: Optional[str] = None,
    in_place: bool = False,
) -> None:
    """Reorder a BEN bundle's graph and relabel its stream to match.

    ``sort`` selects the ordering — ``"mlc"`` (default), ``"rcm"``, or ``"key"``
    to sort by the node attribute named in ``key`` (e.g. ``sort="key",
    key="GEOID"``). It reorders the embedded ``graph.json``, rewrites every
    assignment into the new node order, and writes a fresh bundle storing the
    reordered graph and a ``node_permutation_map.json`` (so the reordering is
    reversible). Metadata and custom assets are preserved. This is the
    bundle-level form of the CLI's ``reben`` ordering flow — typically run to
    shrink a bundle before an XBEN recompress.

    Provide exactly one of ``in_place=True`` or ``out_file``. Only BEN bundles are
    supported (relabel before compressing to XBEN); the source must carry a graph.
    """
    _atomic_or_out(
        lambda src, dst, overwrite: _relabel_bundle(src, dst, sort, key, overwrite),
        path,
        out_file,
        in_place,
    )
