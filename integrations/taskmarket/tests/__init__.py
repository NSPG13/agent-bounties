"""Test package. Bootstraps sys.path so the tests run under both pytest and
`python -m unittest discover` without requiring installation."""
import pathlib
import sys

_SRC = pathlib.Path(__file__).resolve().parents[1] / "src"
if str(_SRC) not in sys.path:
    sys.path.insert(0, str(_SRC))
