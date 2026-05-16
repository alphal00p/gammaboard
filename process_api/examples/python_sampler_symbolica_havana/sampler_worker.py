import importlib
import json
import os
import sys
import traceback

import numpy as np

sampler = None
discrete_cardinalities = None
continuous_dims = None
sampler_args = {}
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
                f"failed to import sampler module {module_name!r}; "
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
        raise ValueError("python sampler args.module must be a non-empty string")
    if not isinstance(class_name, str) or not class_name:
        raise ValueError("python sampler args.class must be a non-empty string")
    init_args = dict(args)
    init_args.pop("module", None)
    init_args.pop("class", None)
    return module_name, class_name, init_args


def require_homogeneous_offsets(params, field, nr_samples, width):
    expected = [index * width for index in range(nr_samples + 1)]
    raw = params.get(field)
    if raw is None:
        return
    offsets = [int(value) for value in raw]
    if offsets != expected:
        raise ValueError(
            "python sampler wrapper only supports homogeneous batches; "
            f"{field}={offsets} does not match fixed width {width}"
        )


def fixed_domain_shape(domain):
    if not isinstance(domain, dict):
        raise ValueError("domain must be an object")
    if "Continuous" in domain:
        return [], int(domain["Continuous"]["dims"])
    if "Discrete" not in domain:
        raise ValueError(f"unsupported domain shape: {domain!r}")
    branches = domain["Discrete"].get("branches") or []
    if not branches:
        raise ValueError(
            "homogeneous Python wrapper requires non-empty discrete branches"
        )
    tail_cardinalities = None
    tail_continuous_dims = None
    for branch in branches:
        cardinalities, continuous = fixed_domain_shape(branch["domain"])
        if tail_cardinalities is None:
            tail_cardinalities = cardinalities
            tail_continuous_dims = continuous
        elif cardinalities != tail_cardinalities or continuous != tail_continuous_dims:
            raise ValueError(
                "homogeneous Python wrapper does not support inhomogeneous domains"
            )
    return [len(branches), *tail_cardinalities], tail_continuous_dims


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
            if params.get("role") != "sampler":
                raise ValueError(f"expected sampler role, got {params.get('role')!r}")
            print("GB_E2E_PY_SAMPLER_INIT symbolica_havana", flush=True)
            module_name, class_name, init_args = parse_python_wrapper_args(params)
            module = import_configured_module(module_name)
            cls = getattr(module, class_name)
            discrete_cardinalities, continuous_dims = fixed_domain_shape(
                params["domain"]
            )
            if any(value <= 0 for value in discrete_cardinalities):
                raise ValueError(
                    "discrete_cardinalities must contain only positive integers"
                )
            discrete_dims = len(discrete_cardinalities)
            sampler_args = init_args
            snapshot = params.get("snapshot")
            if snapshot is not None:
                if not hasattr(cls, "from_snapshot"):
                    raise TypeError(
                        "class must define from_snapshot(...) when restoring from snapshot"
                    )
                if (
                    isinstance(snapshot, dict)
                    and "save_path" not in snapshot
                    and "save_path" in sampler_args
                ):
                    snapshot = {**snapshot, "save_path": sampler_args["save_path"]}
                sampler = cls.from_snapshot(
                    snapshot=snapshot,
                    discrete_cardinalities=discrete_cardinalities,
                    continuous_dims=continuous_dims,
                    init_args=sampler_args,
                )
            elif hasattr(cls, "from_config"):
                sampler = cls.from_config(
                    discrete_cardinalities=discrete_cardinalities,
                    continuous_dims=continuous_dims,
                    init_args=sampler_args,
                )
            elif sampler_args:
                sampler = cls(**sampler_args)
            else:
                sampler = cls()
            maybe_discrete_cardinalities = getattr(
                sampler, "discrete_cardinalities", None
            )
            if (
                maybe_discrete_cardinalities is not None
                and [int(value) for value in maybe_discrete_cardinalities]
                != discrete_cardinalities
            ):
                raise ValueError(
                    f"sampler discrete_cardinalities mismatch: expected {discrete_cardinalities}, got {maybe_discrete_cardinalities}"
                )
            maybe_continuous_dims = getattr(sampler, "continuous_dims", None)
            if (
                maybe_continuous_dims is not None
                and int(maybe_continuous_dims) != continuous_dims
            ):
                raise ValueError(
                    f"sampler continuous_dims mismatch: expected {continuous_dims}, got {int(maybe_continuous_dims)}"
                )
            send_result(req_id, {"ok": True})
        elif method == "sample_plan":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            plan = sampler.sample_plan()
            send_result(req_id, {"plan": plan})
        elif method == "training_samples_remaining":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            remaining = sampler.training_samples_remaining()
            if remaining is not None:
                remaining = int(remaining)
            send_result(req_id, {"remaining": remaining})
        elif method == "produce_latent_batch":
            if (
                sampler is None
                or discrete_cardinalities is None
                or continuous_dims is None
            ):
                raise RuntimeError("worker not initialized")
            nr_samples = int(params["nr_samples"])
            batch = sampler.produce_latent_batch(nr_samples)
            if isinstance(batch, tuple):
                raise TypeError(
                    "produce_latent_batch must return an object with xs_discrete, xs_continuous, and weights attributes, not a tuple"
                )
            xs_discrete = np.asarray(batch.xs_discrete, dtype=np.int64)
            xs_continuous = np.asarray(batch.xs_continuous, dtype=np.float64)
            weights = np.asarray(batch.weights, dtype=np.float64).reshape((nr_samples,))
            discrete_dims = len(discrete_cardinalities)
            if xs_discrete.shape != (nr_samples, discrete_dims):
                raise ValueError(
                    f"produce_latent_batch returned discrete shape {xs_discrete.shape}, expected ({nr_samples}, {discrete_dims})"
                )
            if xs_continuous.shape != (nr_samples, continuous_dims):
                raise ValueError(
                    f"produce_latent_batch returned continuous shape {xs_continuous.shape}, expected ({nr_samples}, {continuous_dims})"
                )
            if not np.isfinite(xs_continuous).all():
                raise ValueError(
                    "produce_latent_batch returned non-finite continuous values"
                )
            if not np.isfinite(weights).all():
                raise ValueError("produce_latent_batch returned non-finite weights")
            if (weights <= 0.0).any():
                raise ValueError("produce_latent_batch returned non-positive weights")
            for axis, cardinality in enumerate(discrete_cardinalities):
                axis_values = xs_discrete[:, axis]
                if ((axis_values < 0) | (axis_values >= cardinality)).any():
                    raise ValueError(
                        f"produce_latent_batch returned discrete values outside [0, {cardinality}) on axis {axis}"
                    )
            send_result(
                req_id,
                {
                    "xs_discrete_row_major": xs_discrete.reshape(
                        (nr_samples * discrete_dims,)
                    ).tolist(),
                    "xs_discrete_offsets": [
                        index * discrete_dims for index in range(nr_samples + 1)
                    ],
                    "xs_continuous_row_major": xs_continuous.reshape(
                        (nr_samples * continuous_dims,)
                    ).tolist(),
                    "xs_continuous_offsets": [
                        index * continuous_dims for index in range(nr_samples + 1)
                    ],
                    "weights": weights.tolist(),
                },
            )
        elif method == "ingest_training_values":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            training_values = np.asarray(
                params["training_values"], dtype=np.float64
            ).reshape((-1,))
            sampler.ingest_training_values(training_values)
            send_result(req_id, {"ok": True})
        elif method == "pdf":
            if (
                sampler is None
                or discrete_cardinalities is None
                or continuous_dims is None
            ):
                raise RuntimeError("worker not initialized")
            nr_samples = int(params["nr_samples"])
            discrete_dims = len(discrete_cardinalities)
            require_homogeneous_offsets(
                params, "xs_discrete_offsets", nr_samples, discrete_dims
            )
            require_homogeneous_offsets(
                params, "xs_continuous_offsets", nr_samples, continuous_dims
            )
            xs_discrete = np.asarray(
                params["xs_discrete_row_major"], dtype=np.int64
            ).reshape((nr_samples, discrete_dims))
            xs_continuous = np.asarray(
                params["xs_continuous_row_major"], dtype=np.float64
            ).reshape((nr_samples, continuous_dims))
            pdf = (
                sampler.pdf(xs_discrete, xs_continuous)
                if hasattr(sampler, "pdf")
                else None
            )
            if pdf is None:
                send_result(req_id, {"values": None})
                continue
            pdf = np.asarray(pdf, dtype=np.float64).reshape((nr_samples,))
            send_result(req_id, {"values": pdf.tolist()})
        elif method == "snapshot":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            snapshot = sampler.snapshot()
            if (
                isinstance(snapshot, dict)
                and "save_path" not in snapshot
                and "save_path" in sampler_args
            ):
                snapshot = {**snapshot, "save_path": sampler_args["save_path"]}
            send_result(req_id, {"snapshot": snapshot})
        elif method == "get_diagnostics":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            diagnostics = (
                sampler.get_diagnostics() if hasattr(sampler, "get_diagnostics") else {}
            )
            send_result(req_id, {"diagnostics": diagnostics})
        else:
            raise ValueError(f"unknown method: {method}")
    except Exception as exc:
        send_error(req_id, exc)
