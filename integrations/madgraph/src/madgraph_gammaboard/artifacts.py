from __future__ import annotations

import importlib
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

import numpy as np


@dataclass
class MatrixElementBackend:
    callable: Callable[..., Any]
    parameters: dict[str, Any] | None = None

    def evaluate(self, momenta: np.ndarray) -> np.ndarray:
        try:
            values = self.callable(momenta, parameters=self.parameters)
        except TypeError:
            values = self.callable(momenta)
        return np.asarray(values, dtype=np.float64).reshape((momenta.shape[0],))


def load_matrix_element_backend(config: dict[str, Any]) -> MatrixElementBackend:
    kind = str(config.get("kind", "python_callable"))
    if kind == "python_callable":
        return _load_python_callable(config)
    if kind == "madgraph7":
        raise NotImplementedError(
            "matrix_element.kind = 'madgraph7' is reserved for a direct MadGraph7 "
            "builder/loader once the callable API is pinned. For now, generate or "
            "write a small Python adapter and use kind = 'python_callable'."
        )
    raise ValueError(f"unsupported matrix_element kind {kind!r}")


def _load_python_callable(config: dict[str, Any]) -> MatrixElementBackend:
    module_name = config.get("module")
    function_name = config.get("function", "matrix_element")
    if not module_name:
        raise ValueError("matrix_element.module is required for kind = 'python_callable'")

    for key in ("madgraph_root", "search_path", "cache_dir"):
        value = config.get(key)
        if value:
            _prepend_sys_path(value)

    module = importlib.import_module(str(module_name))
    function = getattr(module, str(function_name), None)
    if function is None or not callable(function):
        raise ValueError(f"{module_name}.{function_name} is not callable")
    parameters = config.get("parameters")
    if parameters is not None and not isinstance(parameters, dict):
        raise ValueError("matrix_element.parameters must be a table/object when provided")
    return MatrixElementBackend(function, parameters)


def _prepend_sys_path(path: Any) -> None:
    raw_paths = path if isinstance(path, list) else [path]
    for raw in reversed(raw_paths):
        resolved = str(Path(str(raw)).expanduser().resolve())
        if resolved not in sys.path:
            sys.path.insert(0, resolved)
