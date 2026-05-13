import importlib
import json
import traceback
import numpy as np
import sys
import os

integrand = None
discrete_cardinalities = None
continuous_dims = None
protocol_stdout = os.fdopen(os.dup(1), "wb", buffering=0)
sys.stdout = sys.stderr


def send_frame(payload):
    body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    protocol_stdout.write(f"Content-Length: {len(body)}\r\n\r\n".encode("ascii"))
    protocol_stdout.write(body)


def read_frame():
    content_length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.decode("ascii", errors="replace").strip()
        if not line:
            if content_length is not None:
                break
            continue
        name, sep, value = line.partition(":")
        if sep and name.lower() == "content-length":
            content_length = int(value.strip())
    return json.loads(sys.stdin.buffer.read(content_length).decode("utf-8"))


def send_result(req_id, result):
    send_frame({"jsonrpc": "2.0", "id": req_id, "result": result})


def send_error(req_id, exc):
    send_frame(
        {
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {
                "code": -32000,
                "message": f"{type(exc).__name__}: {exc}",
                "data": {"traceback": traceback.format_exc(limit=8)},
            },
        }
    )


def import_configured_module(module_name):
    try:
        return importlib.import_module(module_name)
    except ModuleNotFoundError as exc:
        if exc.name == module_name or module_name.startswith(f"{exc.name}."):
            raise ModuleNotFoundError(
                f"failed to import evaluator module {module_name!r}; "
                f"PYTHONPATH={os.environ.get('PYTHONPATH', '')!r}; sys.path={sys.path!r}"
            ) from exc
        raise


def parse_python_wrapper_args(params):
    args = params.get("args") or {}
    if not isinstance(args, dict):
        raise TypeError("args must be an object")
    module_name = args.get("module")
    class_name = args.get("class")
    if not isinstance(module_name, str) or not module_name:
        raise ValueError("python evaluator args.module must be a non-empty string")
    if not isinstance(class_name, str) or not class_name:
        raise ValueError("python evaluator args.class must be a non-empty string")
    init_args = dict(args)
    init_args.pop("module", None)
    init_args.pop("class", None)
    return module_name, class_name, init_args


while True:
    req = read_frame()
    if req is None:
        break
    req_id = req.get("id")
    try:
        method = req["method"]
        params = req.get("params") or {}
        if method == "initialize":
            if params.get("protocol") != "gammaboard-jsonrpc-v1":
                raise ValueError(f"unsupported protocol: {params.get('protocol')!r}")
            if params.get("role") != "evaluator":
                raise ValueError(f"expected evaluator role, got {params.get('role')!r}")
            module_name, class_name, init_args = parse_python_wrapper_args(params)
            module = import_configured_module(module_name)
            cls = getattr(module, class_name)
            discrete_cardinalities = [int(value) for value in params["discrete_cardinalities"]]
            if any(value <= 0 for value in discrete_cardinalities):
                raise ValueError(
                    "discrete_cardinalities must contain only positive integers"
                )
            discrete_dims = len(discrete_cardinalities)
            continuous_dims = int(params["continuous_dims"])
            if hasattr(cls, "from_config"):
                integrand = cls.from_config(
                    discrete_cardinalities=discrete_cardinalities,
                    continuous_dims=continuous_dims,
                    init_args=init_args,
                )
            elif init_args:
                integrand = cls(**init_args)
            else:
                integrand = cls()
            maybe_discrete_cardinalities = getattr(
                integrand, "discrete_cardinalities", None
            )
            if maybe_discrete_cardinalities is not None:
                parsed = [int(value) for value in maybe_discrete_cardinalities]
                if parsed != discrete_cardinalities:
                    raise ValueError(
                        "integrand discrete_cardinalities mismatch: "
                        f"expected {discrete_cardinalities}, got {maybe_discrete_cardinalities}"
                    )
            maybe_discrete_dims = getattr(integrand, "discrete_dims", None)
            if maybe_discrete_dims is not None and int(maybe_discrete_dims) != discrete_dims:
                raise ValueError(
                    f"integrand discrete_dims mismatch: expected {discrete_dims}, got {int(maybe_discrete_dims)}"
                )
            maybe_continuous_dims = getattr(integrand, "continuous_dims", None)
            if maybe_continuous_dims is not None and int(maybe_continuous_dims) != continuous_dims:
                raise ValueError(
                    f"integrand continuous_dims mismatch: expected {continuous_dims}, got {int(maybe_continuous_dims)}"
                )
            send_result(req_id, {"ok": True})
        elif method == "eval_scalar":
            if (
                integrand is None
                or discrete_cardinalities is None
                or continuous_dims is None
            ):
                raise RuntimeError("worker not initialized")
            nr_samples = int(params["nr_samples"])
            req_discrete_cardinalities = [
                int(value) for value in params["discrete_cardinalities"]
            ]
            req_discrete_dims = len(req_discrete_cardinalities)
            req_continuous_dims = int(params["continuous_dims"])
            if (
                req_discrete_cardinalities != discrete_cardinalities
                or req_continuous_dims != continuous_dims
            ):
                raise ValueError(
                    "dimension mismatch: "
                    f"worker=({discrete_cardinalities}, {continuous_dims}) "
                    f"request=({req_discrete_cardinalities}, {req_continuous_dims})"
                )
            xs_discrete = np.asarray(params["xs_discrete_row_major"], dtype=np.int64).reshape(
                (nr_samples, req_discrete_dims)
            )
            xs_continuous = np.asarray(
                params["xs_continuous_row_major"], dtype=np.float64
            ).reshape((nr_samples, req_continuous_dims))
            for axis, cardinality in enumerate(discrete_cardinalities):
                axis_values = xs_discrete[:, axis]
                if (axis_values < 0).any() or (axis_values >= cardinality).any():
                    raise ValueError(
                        f"xs_discrete axis {axis} out of bounds for cardinality {cardinality}"
                    )
            ys = np.asarray(
                integrand.eval(xs_discrete, xs_continuous), dtype=np.float64
            ).reshape((nr_samples,))
            send_result(req_id, {"values": ys.tolist()})
        else:
            raise ValueError(f"unknown method: {method}")
    except Exception as exc:
        send_error(req_id, exc)
