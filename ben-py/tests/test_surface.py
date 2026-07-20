"""Public-surface and type-stub drift tests.

These guard the packaging contract: that every documented import resolves, that
the ``_core.pyi`` stub and the facade stubs match the runtime signatures (the
check that would have caught the historical missing ``graph`` / ``ben_file_only``
/ ``allow_unfinalized`` drift), and that the retired ``PyBen*`` / ``compress_*``
names are not re-exported.
"""

from __future__ import annotations

import ast
import inspect
from pathlib import Path

import pytest

import binary_ensemble
from binary_ensemble import _core, bundle, codec, graph, stream

PKG_DIR = Path(binary_ensemble.__file__).parent


# ---------------------------------------------------------------------------
# Import surface
# ---------------------------------------------------------------------------


def test_top_level_exports() -> None:
    expected = {
        "stream",
        "bundle",
        "codec",
        "graph",
        "BendlEncoder",
        "BendlDecoder",
        "compress_stream",
        "decompress_stream",
        "relabel_bundle",
        "BenEncoder",
        "BenDecoder",
        "encode_jsonl_to_ben",
        "encode_jsonl_to_xben",
        "encode_ben_to_xben",
        "decode_ben_to_jsonl",
        "decode_xben_to_jsonl",
        "decode_xben_to_ben",
    }
    assert expected.issubset(set(binary_ensemble.__all__))
    for name in expected:
        assert hasattr(binary_ensemble, name)


def test_stream_module_exports() -> None:
    assert set(stream.__all__) == {"BenEncoder", "BenDecoder"}
    assert stream.BenEncoder is _core.BenEncoder
    assert stream.BenDecoder is _core.BenDecoder


def test_bundle_module_exports() -> None:
    assert set(bundle.__all__) == {
        "BendlEncoder",
        "BendlDecoder",
        "BendlStreamSession",
        "compress_stream",
        "decompress_stream",
        "relabel_bundle",
    }
    assert bundle.BendlDecoder is _core.BendlDecoder
    assert bundle.BendlStreamSession is _core.BendlStreamSession


def test_codec_module_exports() -> None:
    assert set(codec.__all__) == {
        "encode_jsonl_to_ben",
        "encode_jsonl_to_xben",
        "encode_ben_to_xben",
        "decode_ben_to_jsonl",
        "decode_xben_to_jsonl",
        "decode_xben_to_ben",
    }
    for name in codec.__all__:
        assert getattr(codec, name) is getattr(_core, name)


def test_graph_module_exports() -> None:
    assert set(graph.__all__) == {
        "reorder",
        "reorder_multi_level_cluster",
        "reorder_reverse_cuthill_mckee",
        "reorder_by_key",
    }


def test_core_submodule_is_accessible() -> None:
    # _core stays importable for power users, but is not advertised in __all__.
    assert hasattr(binary_ensemble, "_core")
    assert "_core" not in binary_ensemble.__all__


# ---------------------------------------------------------------------------
# Negative imports: retired names must not be re-exported.
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "name",
    [
        "PyBenEncoder",
        "PyBenDecoder",
        "PyBendlEncoder",
        "PyBendlDecoder",
        "compress_jsonl_to_ben",
        "compress_jsonl_to_xben",
        "compress_ben_to_xben",
        "decompress_ben_to_jsonl",
        "decompress_xben_to_jsonl",
        "decompress_xben_to_ben",
    ],
)
def test_retired_names_not_exported(name: str) -> None:
    assert not hasattr(binary_ensemble, name)
    assert name not in binary_ensemble.__all__


# ---------------------------------------------------------------------------
# Signature-drift machinery.
# ---------------------------------------------------------------------------

_SKIP = {"self", "cls", "$self", "$cls"}


def _params_from_text_sig(text_sig):
    """Normalize a PyO3 ``__text_signature__`` into ``[(name, has_default), ...]``."""
    if not text_sig:
        return None
    inner = text_sig.strip()
    inner = inner[inner.index("(") + 1 : inner.rindex(")")]
    out = []
    for part in inner.split(","):
        part = part.strip()
        if not part or part in ("/", "*"):
            continue
        if part.startswith("*"):
            continue  # *args / **kwargs
        name = part.split("=")[0].split(":")[0].strip()
        if name in _SKIP:
            continue
        out.append((name, "=" in part))
    return out


def _params_from_ast(func: ast.FunctionDef):
    a = func.args
    positional = list(a.posonlyargs) + list(a.args)
    n_def = len(a.defaults)
    has_default = [False] * (len(positional) - n_def) + [True] * n_def
    out = []
    for arg, hd in zip(positional, has_default):
        if arg.arg in _SKIP:
            continue
        out.append((arg.arg, hd))
    for arg, default in zip(a.kwonlyargs, a.kw_defaults):
        out.append((arg.arg, default is not None))
    return out


def _parse_stub(path: Path):
    """Parse a ``.pyi`` into ``{name: ('func', params) | ('class', {method: params})}``."""
    tree = ast.parse(path.read_text())
    symbols = {}
    for node in tree.body:
        if isinstance(node, ast.FunctionDef):
            symbols[node.name] = ("func", _params_from_ast(node))
        elif isinstance(node, ast.ClassDef):
            methods = {}
            for item in node.body:
                if isinstance(item, ast.FunctionDef):
                    methods[item.name] = _params_from_ast(item)
            symbols[node.name] = ("class", methods)
    return symbols


def _runtime_public_names(obj):
    return {n for n in dir(obj) if not n.startswith("_")}


# ---------------------------------------------------------------------------
# _core stub drift (catches missing / extra / changed parameters).
# ---------------------------------------------------------------------------


def test_core_stub_covers_runtime_and_matches_signatures() -> None:
    stub = _parse_stub(PKG_DIR / "_core.pyi")

    runtime_names = _runtime_public_names(_core)
    # Every runtime public symbol must be documented in the stub.
    for name in runtime_names:
        assert name in stub, f"_core.{name} is missing from _core.pyi"
    # Every stubbed symbol must exist at runtime.
    for name in stub:
        assert hasattr(_core, name), f"_core.pyi declares {name} but runtime lacks it"

    for name, (kind, payload) in stub.items():
        obj = getattr(_core, name)
        if kind == "func":
            runtime = _params_from_text_sig(obj.__text_signature__)
            assert runtime == payload, f"signature drift on _core.{name}"
        else:
            # __init__ is described by the class-level text signature.
            if "__init__" in payload:
                runtime_init = _params_from_text_sig(obj.__text_signature__)
                assert runtime_init == payload["__init__"], (
                    f"__init__ signature drift on _core.{name}"
                )
            # Public non-dunder methods declared in the stub must match.
            stub_methods = {m for m in payload if not m.startswith("__")}
            runtime_methods = _runtime_public_names(obj)
            assert stub_methods == runtime_methods, (
                f"method set drift on _core.{name}: stub={stub_methods} runtime={runtime_methods}"
            )
            for method in stub_methods:
                runtime = _params_from_text_sig(getattr(obj, method).__text_signature__)
                if runtime is None:
                    continue
                assert runtime == payload[method], f"signature drift on _core.{name}.{method}"


# ---------------------------------------------------------------------------
# Facade stub drift (pure-Python objects, via inspect).
# ---------------------------------------------------------------------------


def _params_from_inspect(func, *, drop_self: bool):
    out = []
    params = list(inspect.signature(func).parameters.values())
    for i, p in enumerate(params):
        if drop_self and i == 0 and p.name in ("self", "cls"):
            continue
        if p.kind in (p.VAR_POSITIONAL, p.VAR_KEYWORD):
            continue
        out.append((p.name, p.default is not inspect.Parameter.empty))
    return out


def test_bundle_facade_matches_stub() -> None:
    stub = _parse_stub(PKG_DIR / "bundle.pyi")

    # Module-level functions.
    assert (
        _params_from_inspect(bundle.compress_stream, drop_self=False) == stub["compress_stream"][1]
    )
    assert (
        _params_from_inspect(bundle.decompress_stream, drop_self=False)
        == stub["decompress_stream"][1]
    )
    assert _params_from_inspect(bundle.relabel_bundle, drop_self=False) == stub["relabel_bundle"][1]

    # BendlEncoder methods.
    enc_methods = stub["BendlEncoder"][1]
    for method, expected in enc_methods.items():
        if method.startswith("__"):
            continue
        runtime = getattr(bundle.BendlEncoder, method)
        assert _params_from_inspect(runtime, drop_self=True) == expected, (
            f"signature drift on bundle.BendlEncoder.{method}"
        )


def test_graph_facade_matches_stub() -> None:
    stub = _parse_stub(PKG_DIR / "graph.pyi")
    for name, (kind, params) in stub.items():
        if kind != "func":
            continue
        runtime = getattr(graph, name)
        assert _params_from_inspect(runtime, drop_self=False) == params, (
            f"signature drift on graph.{name}"
        )
