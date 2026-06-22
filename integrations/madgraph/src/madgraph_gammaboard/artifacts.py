from __future__ import annotations

import importlib
import sys
from pathlib import Path
from typing import Any, Callable

import numpy as np

MatrixElement = Callable[[np.ndarray], np.ndarray]


def load_matrix_element(config: dict[str, Any]) -> MatrixElement:
    """Load one generated MadGraph F2PY subprocess artifact.

    The expected artifact is the `matrix2py` module built from a standalone
    MadGraph subprocess directory. For flavor-dependent artifacts, use the
    wrapper-side `function = "py_matrix_sum"` mode.
    """

    _prepend_config_paths(config)
    module_name = str(config.get("module", "matrix2py"))
    module = importlib.import_module(module_name)
    initialize_path = config.get("initialize_path") or config.get("param_card")
    initialize_name = str(config.get("initialize_function", "py_initialisemodel"))
    if initialize_path is not None:
        initialize = getattr(module, initialize_name, None)
        if initialize is None:
            raise ValueError(f"{module_name}.{initialize_name} is not available")
        initialize(str(initialize_path))

    function_name = str(config.get("function", "py_matrix_sum"))
    if function_name == "py_smatrix":
        raise ValueError(
            "matrix_element.function = 'py_smatrix' is disabled because generated "
            "wrappers can miss the flavor argument and crash the worker; use "
            "'py_matrix_sum', 'py_get_value', or 'py_smatrixhel'"
        )

    if function_name == "py_matrix_sum":
        flavor = _required_int_array(config.get("flavor"), "flavor")
        helicities = _helicity_arrays(config.get("helicities"), int(flavor.size))
        normalization = float(config.get("normalization", 1.0))
        py_matrix = getattr(module, "py_matrix", None)
        if py_matrix is None or not callable(py_matrix):
            raise ValueError(
                f"{module_name}.py_matrix is required for function = 'py_matrix_sum'"
            )

        def evaluate(momenta: np.ndarray) -> np.ndarray:
            values = []
            for point in np.asarray(momenta, dtype=np.float64):
                p = _madgraph_momenta(point)
                total = 0.0
                for helicity in helicities:
                    total += float(py_matrix(p, helicity, flavor))
                values.append(total / normalization)
            return np.asarray(values, dtype=np.float64)

        return evaluate

    function = getattr(module, function_name, None)
    if function is None or not callable(function):
        raise ValueError(f"{module_name}.{function_name} is not callable")
    nhel = int(config.get("nhel", -1))
    alphas = float(config.get("alphas", 0.118))

    def evaluate(momenta: np.ndarray) -> np.ndarray:
        values = []
        for point in np.asarray(momenta, dtype=np.float64):
            p = _madgraph_momenta(point)
            if function_name == "py_smatrixhel":
                values.append(function(p, nhel))
            elif function_name == "py_get_value":
                values.append(function(p, alphas, nhel))
            else:
                values.append(function(p))
        return np.asarray(values, dtype=np.float64)

    return evaluate


def _prepend_config_paths(config: dict[str, Any]) -> None:
    for key in ("search_path", "artifact_dir"):
        value = config.get(key)
        if value:
            _prepend_sys_path(value)


def _madgraph_momenta(point: np.ndarray) -> np.ndarray:
    point = np.asarray(point, dtype=np.float64)
    if point.ndim != 2 or point.shape[1] != 4:
        raise ValueError(
            f"MadGraph momenta must have shape (nexternal, 4), got {point.shape}"
        )
    return np.ascontiguousarray(point.T)


def _required_int_array(value: Any, key: str) -> np.ndarray:
    if value is None:
        raise ValueError(f"matrix_element.{key} is required")
    array = np.asarray(value, dtype=np.int32)
    if array.ndim != 1 or array.size == 0:
        raise ValueError(f"matrix_element.{key} must be a non-empty integer list")
    return array


def _helicity_arrays(value: Any, nexternal: int) -> list[np.ndarray]:
    if value is None or value == "all_pm":
        return [
            np.asarray(helicity, dtype=np.int32)
            for helicity in _all_plus_minus_helicities(nexternal)
        ]
    arrays = [np.asarray(helicity, dtype=np.int32) for helicity in value]
    if not arrays:
        raise ValueError("matrix_element.helicities must not be empty")
    for array in arrays:
        if array.ndim != 1 or array.size != nexternal:
            raise ValueError(
                "each matrix_element.helicities entry must match flavor length"
            )
    return arrays


def _all_plus_minus_helicities(nexternal: int) -> list[list[int]]:
    if nexternal <= 0:
        return []
    rest = _all_plus_minus_helicities(nexternal - 1)
    if not rest:
        return [[-1], [1]]
    return [[-1, *helicity] for helicity in rest] + [
        [1, *helicity] for helicity in rest
    ]


def _prepend_sys_path(path: Any) -> None:
    raw_paths = path if isinstance(path, list) else [path]
    for raw in reversed(raw_paths):
        resolved = str(Path(str(raw)).expanduser().resolve())
        if resolved not in sys.path:
            sys.path.insert(0, resolved)
