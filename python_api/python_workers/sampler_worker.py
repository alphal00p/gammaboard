import importlib
import json
import traceback
import numpy as np
import sys
import os

sampler = None
discrete_cardinalities = None
continuous_dims = None
sampler_init_args = {}
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
            module = import_configured_module(params["module"])
            cls = getattr(module, params["class"])
            discrete_cardinalities = [int(value) for value in params["discrete_cardinalities"]]
            if any(value <= 0 for value in discrete_cardinalities):
                raise ValueError("discrete_cardinalities must contain only positive integers")
            discrete_dims = len(discrete_cardinalities)
            continuous_dims = int(params["continuous_dims"])
            init_args = params.get("init_args") or {}
            if not isinstance(init_args, dict):
                raise TypeError("init_args must be an object")
            sampler_init_args = init_args
            snapshot = params.get("snapshot")
            if snapshot is not None:
                if not hasattr(cls, "from_snapshot"):
                    raise TypeError("class must define from_snapshot(...) when restoring from snapshot")
                if isinstance(snapshot, dict) and "save_path" not in snapshot and "save_path" in init_args:
                    snapshot = {**snapshot, "save_path": init_args["save_path"]}
                sampler = cls.from_snapshot(
                    snapshot=snapshot,
                    discrete_cardinalities=discrete_cardinalities,
                    continuous_dims=continuous_dims,
                    init_args=init_args,
                )
            elif hasattr(cls, "from_config"):
                sampler = cls.from_config(
                    discrete_cardinalities=discrete_cardinalities,
                    continuous_dims=continuous_dims,
                    init_args=init_args,
                )
            elif init_args:
                sampler = cls(**init_args)
            else:
                sampler = cls()
            maybe_discrete_cardinalities = getattr(sampler, "discrete_cardinalities", None)
            if maybe_discrete_cardinalities is not None and [int(value) for value in maybe_discrete_cardinalities] != discrete_cardinalities:
                raise ValueError(
                    f"sampler discrete_cardinalities mismatch: expected {discrete_cardinalities}, got {maybe_discrete_cardinalities}"
                )
            maybe_continuous_dims = getattr(sampler, "continuous_dims", None)
            if maybe_continuous_dims is not None and int(maybe_continuous_dims) != continuous_dims:
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
            if sampler is None or discrete_cardinalities is None or continuous_dims is None:
                raise RuntimeError("worker not initialized")
            nr_samples = int(params["nr_samples"])
            batch = sampler.produce_latent_batch(nr_samples)
            if isinstance(batch, tuple):
                raise TypeError("produce_latent_batch must return an object with xs_discrete, xs_continuous, and weights attributes, not a tuple")
            xs_discrete = batch.xs_discrete
            xs_continuous = batch.xs_continuous
            weights = batch.weights
            xs_discrete = np.asarray(xs_discrete, dtype=np.int64)
            xs_continuous = np.asarray(xs_continuous, dtype=np.float64)
            weights = np.asarray(weights, dtype=np.float64).reshape((nr_samples,))
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
                raise ValueError("produce_latent_batch returned non-finite continuous values")
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
            send_result(req_id, {
                "xs_discrete_row_major": xs_discrete.reshape((nr_samples * discrete_dims,)).tolist(),
                "xs_continuous_row_major": xs_continuous.reshape((nr_samples * continuous_dims,)).tolist(),
                "weights": weights.tolist(),
            })
        elif method == "ingest_training_values":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            training_values = np.asarray(params["training_values"], dtype=np.float64).reshape((-1,))
            sampler.ingest_training_values(training_values)
            send_result(req_id, {"ok": True})
        elif method == "pdf":
            if sampler is None or discrete_cardinalities is None or continuous_dims is None:
                raise RuntimeError("worker not initialized")
            nr_samples = int(params["nr_samples"])
            discrete_dims = len(discrete_cardinalities)
            xs_discrete = np.asarray(params["xs_discrete_row_major"], dtype=np.int64).reshape(
                (nr_samples, discrete_dims)
            )
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
            if isinstance(snapshot, dict) and "save_path" not in snapshot and "save_path" in sampler_init_args:
                snapshot = {**snapshot, "save_path": sampler_init_args["save_path"]}
            send_result(req_id, {"snapshot": snapshot})
        elif method == "get_diagnostics":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            diagnostics = sampler.get_diagnostics() if hasattr(sampler, "get_diagnostics") else {}
            send_result(req_id, {"diagnostics": diagnostics})
        else:
            raise ValueError(f"unknown method: {method}")
    except Exception as exc:
        send_error(req_id, exc)
