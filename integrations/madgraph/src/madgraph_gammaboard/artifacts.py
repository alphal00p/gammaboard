from __future__ import annotations

import importlib
import inspect
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

import numpy as np


@dataclass
class MatrixElementBackend:
    evaluate_fn: Callable[..., Any]
    parameters: dict[str, Any] | None = None

    def evaluate(self, momenta: np.ndarray) -> np.ndarray:
        if self.parameters is None:
            values = self.evaluate_fn(momenta)
        else:
            values = self.evaluate_fn(momenta, parameters=self.parameters)
        return np.asarray(values, dtype=np.float64).reshape((momenta.shape[0],))


def load_matrix_element_backend(config: dict[str, Any]) -> MatrixElementBackend:
    kind = str(config.get("kind", "python_callable"))
    if kind == "python_callable":
        return _load_python_callable(config)
    if kind == "madgraph_python_matrix":
        return _load_madgraph_python_matrix(config)
    if kind == "madgraph_f2py_subprocess":
        return _load_madgraph_f2py_subprocess(config)
    raise ValueError(f"unsupported matrix_element kind {kind!r}")


def _load_python_callable(config: dict[str, Any]) -> MatrixElementBackend:
    module_name = config.get("module")
    function_name = config.get("function", "matrix_element")
    if not module_name:
        raise ValueError(
            "matrix_element.module is required for kind = 'python_callable'"
        )

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
        raise ValueError(
            "matrix_element.parameters must be a table/object when provided"
        )
    return MatrixElementBackend(function, parameters)


def _load_madgraph_python_matrix(config: dict[str, Any]) -> MatrixElementBackend:
    """Load a MadGraph Python-export `Matrix_*` class.

    This targets modules produced from MadGraph's `export_python.py` path. The
    generated matrix class exposes `smatrix(p, model, flavor=None)` for a single
    phase-space point, where `p` is shaped `(4, nexternal)`.
    """

    _prepend_config_paths(config)
    module_name = _required_string(config, "module")
    module = importlib.import_module(module_name)
    matrix_class = _resolve_matrix_class(
        module, config.get("class") or config.get("matrix_class")
    )
    matrix = matrix_class()
    model = _load_model_object(config)
    flavor = config.get("flavor") or config.get("pdgs")
    if flavor is not None:
        flavor = [int(value) for value in flavor]

    def evaluate(
        momenta: np.ndarray, parameters: dict[str, Any] | None = None
    ) -> np.ndarray:
        values = []
        for point in np.asarray(momenta, dtype=np.float64):
            p = _madgraph_momenta(point)
            if flavor is None:
                values.append(matrix.smatrix(p, model))
            else:
                values.append(matrix.smatrix(p, model, flavor=flavor))
        return np.asarray(values, dtype=np.float64)

    return MatrixElementBackend(evaluate, None)


def _load_madgraph_f2py_subprocess(config: dict[str, Any]) -> MatrixElementBackend:
    """Load a generated subprocess `matrix2py` F2PY module.

    This targets the layout produced by standalone output directories with
    `SubProcesses/P.../f2py_matrix_wrapper.f`, whose module usually exposes
    `py_initialisemodel`, `py_smatrix`, `py_smatrixhel`, and `py_get_value`.
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
            "matrix_element.function = 'py_smatrix' is disabled for "
            "madgraph_f2py_subprocess because generated wrappers can have a "
            "flavor-argument mismatch that crashes the worker; use "
            "'py_get_value' or 'py_smatrixhel'"
        )
    nhel = int(config.get("nhel", -1))
    alphas = float(config.get("alphas", 0.118))
    flavor = config.get("flavor")
    normalization = float(config.get("normalization", 1.0))
    helicities = config.get("helicities")

    if function_name == "py_matrix_sum":
        flavor_array = _required_int_array(flavor, "flavor")
        helicity_arrays = _helicity_arrays(helicities, int(flavor_array.size))
        function = getattr(module, "py_matrix", None)
        if function is None or not callable(function):
            raise ValueError(
                f"{module_name}.py_matrix is required for function = 'py_matrix_sum'"
            )
    else:
        function = getattr(module, function_name, None)
        if function is None or not callable(function):
            raise ValueError(f"{module_name}.{function_name} is not callable")

    def evaluate(
        momenta: np.ndarray, parameters: dict[str, Any] | None = None
    ) -> np.ndarray:
        values = []
        for point in np.asarray(momenta, dtype=np.float64):
            p = _madgraph_momenta(point)
            if function_name == "py_matrix_sum":
                total = 0.0
                for helicity in helicity_arrays:
                    total += float(function(p, helicity, flavor_array))
                values.append(total / normalization)
            elif function_name == "py_smatrixhel":
                values.append(function(p, nhel))
            elif function_name == "py_get_value":
                values.append(function(p, alphas, nhel))
            else:
                values.append(function(p))
        return np.asarray(values, dtype=np.float64)

    return MatrixElementBackend(evaluate, None)


def _prepend_config_paths(config: dict[str, Any]) -> None:
    for key in ("madgraph_root", "search_path", "cache_dir", "artifact_dir"):
        value = config.get(key)
        if value:
            _prepend_sys_path(value)


def _required_string(config: dict[str, Any], key: str) -> str:
    value = config.get(key)
    if not value:
        raise ValueError(f"matrix_element.{key} is required")
    return str(value)


def _resolve_matrix_class(module: Any, configured: Any) -> type:
    if configured:
        matrix_class = getattr(module, str(configured), None)
        if matrix_class is None:
            raise ValueError(f"{module.__name__}.{configured} does not exist")
        if not inspect.isclass(matrix_class):
            raise ValueError(f"{module.__name__}.{configured} is not a class")
        return matrix_class

    candidates = [
        value
        for name, value in vars(module).items()
        if name.startswith("Matrix_") and inspect.isclass(value)
    ]
    if len(candidates) != 1:
        raise ValueError(
            f"expected exactly one Matrix_* class in {module.__name__}, found {len(candidates)}; "
            "set matrix_element.class explicitly"
        )
    return candidates[0]


def _load_model_object(config: dict[str, Any]) -> Any:
    module_name = config.get("model_module")
    if not module_name:
        return config.get("model")
    model_module = importlib.import_module(str(module_name))
    if config.get("model_factory"):
        factory = getattr(model_module, str(config["model_factory"]), None)
        if factory is None or not callable(factory):
            raise ValueError(f"{module_name}.{config['model_factory']} is not callable")
        return factory()
    if config.get("model_attribute"):
        attribute = getattr(model_module, str(config["model_attribute"]), None)
        if attribute is None:
            raise ValueError(
                f"{module_name}.{config['model_attribute']} does not exist"
            )
        return attribute
    return model_module


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
