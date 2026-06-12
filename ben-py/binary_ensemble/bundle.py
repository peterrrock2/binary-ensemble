"""The ``.bendl`` bundle format — the recommended single-file container.

A bundle wraps a BEN/XBEN assignment stream together with front-loaded assets: a dual
``graph.json``, a ``node_permutation_map.json``, a ``metadata.json``, and arbitrary custom blobs.
:class:`BendlEncoder` writes one; :class:`BendlDecoder` reads and iterates one.

Typical write::

    with BendlEncoder(path, overwrite=True) as enc:
        enc.add_graph(graph, sort="rcm")                # sort=None => store raw
        enc.add_metadata({"seed": 1234})
        with enc.stream() as stream:
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
from collections.abc import Callable
from typing import TYPE_CHECKING, Literal, cast, overload

from binary_ensemble._core import BendlDecoder, BendlStreamSession
from binary_ensemble._core import BendlEncoder as _CoreBendlEncoder
from binary_ensemble._core import recompress_bundle as _recompress_bundle
from binary_ensemble._core import relabel_bundle as _relabel_bundle
from binary_ensemble.types import (
    BinaryAssetPayload,
    GraphInput,
    JsonAssetPayload,
    MetadataInput,
    SortMethod,
    StrPath,
    TextAssetPayload,
    Variant,
)

if TYPE_CHECKING:
    from types import TracebackType

    import networkx as nx

__all__ = [
    "BendlEncoder",
    "BendlDecoder",
    "BendlStreamSession",
    "compress_stream",
    "relabel_bundle",
]


def _atomic_or_out(
    transform: Callable[[StrPath, StrPath, bool], None],
    path: StrPath,
    out_file: StrPath | None,
    overwrite: bool,
) -> None:
    """Shared in-place / out_file dispatch for whole-bundle transforms.

    ``transform(src, dst, overwrite)`` writes the result. The ``_core`` bindings own the swap
    discipline: the destination is written via a uniquely named temp file that inherits an
    existing destination's permissions, fsynced, and atomically renamed into place — so a
    destination is never visible half-written and an error leaves it exactly as it was.
    ``out_file=None`` means in place: the transform's destination is ``path`` itself.
    """
    if out_file is not None:
        transform(path, out_file, overwrite)
        return
    transform(path, path, True)


def _coerce_asset_payload(payload: object, content_type: str) -> bytes:
    """Coerce an ``add_asset`` payload to bytes.

    Accepted forms:

    - ``dict`` / ``list`` — serialized via ``json.dumps`` (requires ``content_type="json"``).
    - ``str`` — UTF-8 encoded **content** (not a path; pass a ``pathlib.Path`` to read a file —
      this deliberately differs from :meth:`BendlEncoder.add_metadata`, whose payloads are never
      plain text, so there a ``str`` is a path).
    - ``bytes`` / ``bytearray`` / ``memoryview`` — used verbatim.
    - any object with a ``.read()`` method (open files, ``io.BytesIO``) — read, with ``str``
      results UTF-8 encoded.
    - ``os.PathLike`` (e.g. ``pathlib.Path``) — the file at that path is read.
    """
    if isinstance(payload, (dict, list)):
        if content_type != "json":
            raise ValueError(
                "dict/list payloads are serialized as JSON and require "
                f"content_type='json', got {content_type!r}"
            )
        return json.dumps(payload).encode("utf-8")
    if isinstance(payload, str):
        return payload.encode("utf-8")
    if isinstance(payload, (bytes, bytearray, memoryview)):
        return bytes(payload)
    reader = getattr(payload, "read", None)
    if callable(reader):
        data = reader()
        if isinstance(data, str):
            return data.encode("utf-8")
        if isinstance(data, (bytes, bytearray, memoryview)):
            return bytes(data)
        raise TypeError(
            f"asset payload .read() must return bytes or str, got {type(data).__name__}"
        )
    if isinstance(payload, os.PathLike):
        with open(os.fspath(payload), "rb") as f:
            return f.read()
    raise TypeError(
        "asset payload must be bytes, str, dict/list (JSON), a file-like with "
        f".read(), or a path, got {type(payload).__name__}"
    )


class BendlEncoder:
    """Writer for a ``.bendl`` bundle (create mode) or an asset appender (append mode).

    In create mode (the constructor), assets may be added before or after a single-use
    ``stream()``. You do **not** need to use ``BendlEncoder`` itself as a context manager: closing
    the ``stream()`` context finalizes the bundle, so the common pattern is::

        enc = BendlEncoder(path, overwrite=True)
        graph = enc.add_graph(my_graph)          # MLC-reordered by default
        with enc.stream() as stream:             # only the stream needs ``with``
            for assignment in chain:
                stream.write(assignment)
        # bundle is finalized here

    The encoder is still usable as a context manager if you prefer, and that is the easy way to
    finalize an *assets-only* bundle (one written with no ``stream()``): either
    ``with BendlEncoder(...) as enc: ...`` or an explicit :meth:`close`. In append mode
    (:meth:`append`), an existing finalized bundle is grown with new assets and ``stream()`` is
    unavailable.

    Args:
        file_path (StrPath): Output path for the new bundle (``str`` or ``os.PathLike``, e.g.
            ``pathlib.Path``). Must not exist unless ``overwrite=True``.
        overwrite (bool, optional): Replace an existing file at ``file_path``. Default is
            ``False``.

    Raises:
        OSError: If ``file_path`` exists and ``overwrite`` is ``False``, or it cannot be created.
    """

    def __init__(self, file_path: StrPath, overwrite: bool = False) -> None:
        self._path = file_path
        self._enc = _CoreBendlEncoder(file_path, overwrite=overwrite)

    @classmethod
    def append(cls, file_path: StrPath) -> "BendlEncoder":
        """Open an existing *finalized* bundle to append new assets.

        ``stream()`` is unavailable in append mode; each ``add_*`` commits immediately.

        Args:
            file_path (StrPath): Path to an existing, finalized ``.bendl`` bundle (``str`` or
                ``os.PathLike``).

        Returns:
            BendlEncoder: An encoder in append mode.

        Raises:
            Exception: If the file is missing, is not a bundle, or is not finalized.
        """
        self = cls.__new__(cls)
        self._path = file_path
        self._enc = _CoreBendlEncoder.append(file_path)
        return self

    def add_graph(
        self,
        graph: GraphInput,
        sort: SortMethod | None = "mlc",
        key: str | None = None,
    ) -> "nx.Graph":
        """Embed the dual ``graph.json`` and return the (possibly reordered) graph.

        When reordering, both ``graph.json`` and ``node_permutation_map.json`` are stored and the
        reordered graph is returned so the chain runs on that ordering. Reordering is pre-stream
        only; a raw graph (``sort=None``) may also be attached post-stream / in append mode.

        Args:
            graph (GraphInput): The dual graph (:data:`~binary_ensemble.types.GraphInput`): a
                live ``networkx.Graph`` (subclasses such as ``gerrychain.Graph`` count; its node
                iteration order is preserved), or adjacency-format JSON as a parsed ``dict`` or
                ``list``, raw ``bytes``, a file-like object with ``.read()``, or a ``str`` /
                ``os.PathLike`` path to a JSON file. A plain ``str`` is a *path* here.
            sort (SortMethod | None, optional): How to order the nodes
                (:data:`~binary_ensemble.types.SortMethod` or ``None``): ``"mlc"`` (multi-level
                clustering — reorders the graph for better compression), ``"rcm"`` (reverse
                Cuthill-McKee), ``"key"`` (sort by the node attribute named in ``key``), or
                ``None`` to store the graph as-is with no permutation map. Default is ``"mlc"``.
            key (str | None, optional): Node attribute to sort by, e.g. ``key="GEOID"``;
                ``key="id"`` sorts by the NetworkX node id. Required with — and only valid with —
                ``sort="key"``. Default is ``None``.

        Returns:
            networkx.Graph: The stored graph after any reordering (matching
            :meth:`BendlDecoder.read_graph`). Its node iteration order is the order the chain
            must write assignments in.

        Raises:
            ValueError: If ``sort`` / ``key`` is invalid.
            Exception: If a reordering graph is added after the stream has started.
        """
        return self._enc.add_graph(graph, sort, key)

    def add_metadata(self, metadata: MetadataInput) -> None:
        """Embed the canonical ``metadata.json`` asset (run provenance).

        Args:
            metadata (MetadataInput): The JSON payload
                (:data:`~binary_ensemble.types.MetadataInput`): a ``dict`` or ``list``
                (serialized for you), raw JSON ``bytes``, a file-like object with ``.read()``, or
                a ``str`` / ``os.PathLike`` path to a JSON file. A plain ``str`` is a *path*
                here, never inline JSON.

        Raises:
            Exception: If the payload cannot be converted to JSON bytes, or the encoder is in an
                invalid state.
        """
        self._enc.add_metadata(metadata)

    @overload
    def add_asset(
        self, name: str, payload: JsonAssetPayload, content_type: Literal["json"]
    ) -> None: ...
    @overload
    def add_asset(
        self, name: str, payload: TextAssetPayload, content_type: Literal["text"]
    ) -> None: ...
    @overload
    def add_asset(
        self, name: str, payload: BinaryAssetPayload, content_type: Literal["binary"]
    ) -> None: ...
    @overload
    def add_asset(self, name: str, payload: StrPath, content_type: Literal["file"]) -> None: ...
    def add_asset(
        self,
        name: str,
        payload: object,
        content_type: str,
    ) -> None:
        """Embed a custom asset under ``name``.

        Every asset carries a CRC32C integrity checksum, and payloads of 1 KiB or more are
        xz-compressed on disk by default (both transparent on read).

        Args:
            name (str): Asset name, the key used to read it back (e.g. ``"params.json"``).
            payload (JsonAssetPayload | TextAssetPayload | BinaryAssetPayload | StrPath):
                The asset content; the accepted shapes depend on ``content_type``:

                - for ``"json"`` (:data:`~binary_ensemble.types.JsonAssetPayload`): a ``dict`` /
                  ``list`` (serialized via ``json.dumps``), a JSON ``str``, bytes-like JSON, a
                  file-like object with ``.read()``, or an ``os.PathLike`` whose file is read.
                  Must yield valid UTF-8 JSON; the decoder will auto-parse it.
                - for ``"text"`` (:data:`~binary_ensemble.types.TextAssetPayload`): the same
                  shapes, minus ``dict`` / ``list``; must yield valid UTF-8.
                - for ``"binary"`` (:data:`~binary_ensemble.types.BinaryAssetPayload`): the same
                  shapes as ``"text"``; stored verbatim (e.g. a zipped shapefile or a
                  GeoPackage).
                - for ``"file"`` (:data:`~binary_ensemble.types.StrPath`): a ``str`` or
                  ``os.PathLike`` naming a file whose contents are read and stored as binary.

                Outside ``content_type="file"``, a plain ``str`` is always *content*, never a
                path — pass a ``pathlib.Path`` to read from disk (e.g. a ``Path`` with
                ``content_type="json"`` stores a JSON file the decoder will auto-parse).
            content_type (AssetContentType): One of ``"json"``, ``"text"``, ``"binary"``, or
                ``"file"`` (:data:`~binary_ensemble.types.AssetContentType`).

        Raises:
            ValueError: If the payload does not satisfy ``content_type`` (e.g. malformed JSON,
                non-UTF-8 text, an unknown content type).
            TypeError: If the payload shape is not accepted (e.g. a ``dict`` with
                ``content_type="text"``, or a non-path with ``content_type="file"``).
        """
        if content_type == "file":
            if not isinstance(payload, (str, os.PathLike)):
                raise TypeError(
                    "content_type='file' requires a str or os.PathLike payload, "
                    f"got {type(payload).__name__}"
                )
            with open(os.fspath(payload), "rb") as f:
                data = f.read()
            self._enc.add_asset(name, data, "binary")
            return
        data = _coerce_asset_payload(payload, content_type)
        if content_type == "json":
            try:
                json.loads(data.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                raise ValueError(f"content_type='json' requires valid UTF-8 JSON: {exc}") from exc
        elif content_type == "text":
            try:
                data.decode("utf-8")
            except UnicodeDecodeError as exc:
                raise ValueError(f"content_type='text' requires valid UTF-8: {exc}") from exc
        elif content_type != "binary":
            raise ValueError(
                f"content_type must be 'json', 'text', 'binary', or 'file', got {content_type!r}"
            )
        # The branches above leave only the core-supported literals.
        core_type = cast('Literal["json", "text", "binary"]', content_type)
        self._enc.add_asset(name, data, core_type)

    def remove_asset(self, name: str) -> None:
        """Remove a named asset from a finalized bundle, reclaiming its bytes.

        Available wherever :meth:`add_asset` commits immediately: append mode, or create mode
        after the stream has closed. The directory drop and the compaction commit as one
        operation, so the asset's payload bytes are actually gone from the file — not just
        unreferenced — and on any error the bundle is left untouched, the asset still present
        for a retry. The name (and any singleton-type claim, e.g. ``metadata.json``) becomes
        free again, so remove-then-add is the way to replace an asset's payload.

        Removing appended (post-stream) assets is cheap at any scale: the compaction rebuilds
        only the small post-stream tail and never touches the assignment stream, even when the
        stream is tens of gigabytes. Removing a *pre-stream* asset (the graph, or metadata
        added before streaming) costs one whole-file rewrite instead. Note that each
        immediate-commit ``add_asset`` (append mode, or create mode after the stream) leaves
        the superseded directory behind as a few dead bytes; the compaction here reclaims
        those too. For a bundle that arrives with dead space from other tooling, the raw
        ``_core.compact_bundle_in_place`` reclaims it directly, and the raw
        ``_core.BendlEncoder.remove_asset`` drops only the directory entry if you specifically
        need that form.

        Args:
            name (str): The asset's name, as listed by
                :meth:`~binary_ensemble._core.BendlDecoder.asset_names`.

        Raises:
            KeyError: If no asset with that name exists in the bundle.
            Exception: If the encoder is in create mode before the stream (just don't add the
                asset), is currently streaming, or is closed.
        """
        self._enc.remove_asset_compacting(name)

    def stream(self, *, variant: Variant = "twodelta") -> BendlStreamSession:
        """Open the single-use assignment stream context manager.

        The embedded stream is always written in the BEN wire format; produce an XBEN bundle with
        :func:`compress_stream` after writing (XBEN is a whole-stream LZMA2 wrap, so it cannot be
        written live sample-by-sample).

        Args:
            variant (Variant, optional): BEN encoding variant
                (:data:`~binary_ensemble.types.Variant`): ``"standard"``, ``"mkv_chain"``, or
                ``"twodelta"``. Default is ``"twodelta"``.

        Returns:
            BendlStreamSession: A single-use context manager. ``write`` each assignment inside
            the ``with`` block; a clean close finalizes the bundle, an exception leaves it
            unfinalized.

        Raises:
            ValueError: If ``variant`` is invalid.
            Exception: If a stream was already written, append mode is active, or the encoder is
                closed.
        """
        return self._enc.stream(variant=variant)

    def close(self) -> None:
        """Finalize (create mode) or finish (append mode) the bundle. Idempotent."""
        self._enc.close()

    def __enter__(self) -> "BendlEncoder":
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: "TracebackType | None",
    ) -> bool:
        self.close()
        return False


def compress_stream(
    path: StrPath,
    out_file: StrPath | None = None,
    overwrite: bool = False,
) -> None:
    """Recompress a bundle's embedded BEN stream to XBEN, preserving every asset.

    All assets (graph, metadata, node_permutation_map, custom blobs) are preserved by decoded
    payload, name, type, and JSON flag; storage compression is normalized to the writer's default
    policy. An assets-only bundle (empty stream) recompresses to an empty XBEN bundle.

    Args:
        path (StrPath): Path to the source ``.bendl`` bundle (``str`` or ``os.PathLike``).
        out_file (StrPath | None, optional): Destination path for the recompressed bundle
            (``str`` or ``os.PathLike``), leaving ``path`` untouched. Default is ``None`` which
            recompresses in place: the result is written to a temp file and atomically swapped
            over ``path``.
        overwrite (bool, optional): Replace ``out_file`` if it already exists. Irrelevant in
            place, which always replaces ``path``. Default is ``False``.

    Raises:
        OSError: If ``out_file`` exists and ``overwrite`` is ``False``.
    """
    _atomic_or_out(
        lambda src, dst, ow: _recompress_bundle(src, dst, overwrite=ow),
        path,
        out_file,
        overwrite,
    )


def relabel_bundle(
    path: StrPath,
    out_file: StrPath | None = None,
    sort: SortMethod = "mlc",
    key: str | None = None,
    overwrite: bool = False,
) -> None:
    """Reorder a BEN bundle's graph and relabel its stream to match.

    Reorders the embedded ``graph.json``, rewrites every assignment into the new node order, and
    writes a fresh bundle storing the reordered graph and a ``node_permutation_map.json`` (so the
    reordering is reversible). Metadata and custom assets are preserved. This is the bundle-level
    form of the CLI's ``reben`` ordering flow — typically run to shrink a bundle before an XBEN
    recompress.

    Only BEN bundles are supported (relabel before compressing to XBEN); the source must carry a
    graph.

    Args:
        path (StrPath): Path to the source ``.bendl`` bundle (``str`` or ``os.PathLike``). Must
            hold a BEN (not XBEN) stream and a ``graph.json``.
        out_file (StrPath | None, optional): Destination path for the relabeled bundle (``str``
            or ``os.PathLike``), leaving ``path`` untouched. Default is ``None`` which relabels
            in place: the result is written to a temp file and atomically swapped over ``path``.
        sort (SortMethod, optional): The ordering (:data:`~binary_ensemble.types.SortMethod`):
            ``"mlc"`` (multi-level clustering), ``"rcm"`` (reverse Cuthill-McKee), or ``"key"``
            (sort by the node attribute named in ``key``). Default is ``"mlc"``.
        key (str | None, optional): Node attribute to sort by, e.g. ``key="GEOID"``. Required
            with — and only valid with — ``sort="key"``. Default is ``None``.
        overwrite (bool, optional): Replace ``out_file`` if it already exists. Irrelevant in
            place, which always replaces ``path``. Default is ``False``.

    Raises:
        ValueError: If ``sort`` / ``key`` is invalid, or if the bundle has no graph or a non-BEN
            stream.
        OSError: If ``out_file`` exists and ``overwrite`` is ``False``.
    """
    _atomic_or_out(
        lambda src, dst, ow: _relabel_bundle(src, dst, sort, key, ow),
        path,
        out_file,
        overwrite,
    )
