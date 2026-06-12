"""Execute every Python code block in the Markdown docs so they can't silently drift.

The **docs are the single source of truth**. The sample data the recipes read
(``ensemble.bendl``, ``plans.jsonl``, ``chain.ben`` / ``chain.xben``, ``gerrymandria.json``)
is created by the "Sample data" snippet in ``docs/how-to/index.md`` — marked
``<!-- docs-test: setup -->`` — which is shown to readers *and* run by this test. This runner
contains no fixture-creation logic and no per-page knowledge: it discovers the docs, runs the
setup snippet(s), then runs each page's blocks. Editing the docs never requires editing this
test; if a recipe needs new sample data, that goes in the setup snippet (in the docs).

For each page the ```python fences run in order, sharing one namespace, in a fresh temp
working directory. A failing snippet fails the test with the page, block number, and source.
A page that imports GerryChain is skipped only when GerryChain isn't installed. A block may be
opted out with ``<!-- docs-test: skip ... -->`` (reserved for genuinely abstract fragments).
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

DOCS_DIR = Path(__file__).resolve().parent.parent / "docs"

try:
    import gerrychain  # noqa: F401

    HAS_GERRYCHAIN = True
except Exception:  # pragma: no cover - environment-dependent
    HAS_GERRYCHAIN = False

# A python fence, optionally preceded by a "setup"/"skip" docs-test directive comment.
_BLOCK = re.compile(
    r"(?:<!--\s*docs-test:\s*(?P<directive>setup|skip).*?-->\s*)?```python\n(?P<code>.*?)\n```",
    re.DOTALL,
)


def _blocks(text: str):
    """Yield ``(directive, code)`` for each python fence; directive is 'setup'/'skip'/None."""
    for match in _BLOCK.finditer(text):
        yield match.group("directive"), match.group("code")


def _markdown_files() -> list[Path]:
    return sorted(DOCS_DIR.rglob("*.md"))


# The shared "Sample data" setup, taken from the docs themselves (single source of truth).
_SETUP_CODE = "\n".join(
    code
    for path in _markdown_files()
    for directive, code in _blocks(path.read_text())
    if directive == "setup"
)


@pytest.mark.parametrize("doc", _markdown_files(), ids=lambda p: str(p.relative_to(DOCS_DIR)))
def test_markdown_snippets_execute(doc: Path, tmp_path, monkeypatch) -> None:
    runnable = [
        (i, code)
        for i, (directive, code) in enumerate(_blocks(doc.read_text()), start=1)
        if directive is None
    ]
    if not runnable:
        pytest.skip("no runnable python snippets")
    if not HAS_GERRYCHAIN and any("gerrychain" in code for _, code in runnable):
        pytest.skip("page needs GerryChain, which is not installed")

    monkeypatch.chdir(tmp_path)
    # Create the sample files from the docs' own setup snippet. It runs in a throwaway
    # namespace so only its files (not its variables) are visible to the page — a snippet
    # that relies on an undefined name then fails honestly instead of being masked.
    exec(
        compile(_SETUP_CODE, "docs:sample-data-setup", "exec"),
        {"__name__": "__setup__"},
    )

    namespace: dict = {"__name__": "__docs_snippet__"}
    for index, code in runnable:
        try:
            exec(compile(code, f"{doc.name}:block{index}", "exec"), namespace)
        except Exception as exc:  # noqa: BLE001 - surface as a readable test failure
            pytest.fail(
                f"{doc.relative_to(DOCS_DIR)} python block #{index} failed: "
                f"{type(exc).__name__}: {exc}\n\n--- snippet ---\n{code}\n"
            )
