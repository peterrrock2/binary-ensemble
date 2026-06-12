"""Re-execute tutorial notebooks in place, refreshing their committed outputs.

The docs site renders the outputs committed inside each ``.ipynb`` (Sphinx runs with
``nb_execution_mode = "off"`` by default), so whenever a notebook's code cells change, this
script must be run to regenerate those outputs. Use the ``docs-refresh-notebooks`` task, which
runs it with the docs execution extras installed.

Each notebook executes with its own directory as the working directory, so relative paths
(``example_data/``) behave exactly as they do in CI.
"""

import sys
from pathlib import Path

import nbformat
from nbclient import NotebookClient


def refresh(path: Path) -> None:
    nb = nbformat.read(path, as_version=4)
    client = NotebookClient(
        nb,
        timeout=1800,
        kernel_name="python3",
        resources={"metadata": {"path": str(path.parent)}},
    )
    client.execute()
    nbformat.write(nb, path)
    print(f"refreshed {path}")


if __name__ == "__main__":
    for arg in sys.argv[1:]:
        refresh(Path(arg))
