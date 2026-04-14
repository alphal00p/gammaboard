import importlib
import json
import traceback
import numpy as np
import sys

sampler = None
discrete_dims = None
continuous_dims = None


def send(payload):
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()


for raw in sys.stdin:
    line = raw.strip()
    if not line:
        continue
    req = json.loads(line)
    req_id = req.get("id")
    try:
        op = req["op"]
        if op == "init":
            module = importlib.import_module(req["module"])
            cls = getattr(module, req["class"])
            discrete_dims = int(req["discrete_dims"])
            continuous_dims = int(req["continuous_dims"])
            init_args = req.get("init_args") or {}
            if not isinstance(init_args, dict):
                raise TypeError("init_args must be an object")
            snapshot = req.get("snapshot")
            if snapshot is not None:
                if not hasattr(cls, "from_snapshot"):
                    raise TypeError("class must define from_snapshot(...) when restoring from snapshot")
                sampler = cls.from_snapshot(
                    snapshot=snapshot,
                    discrete_dims=discrete_dims,
                    continuous_dims=continuous_dims,
                    init_args=init_args,
                )
            elif hasattr(cls, "from_config"):
                sampler = cls.from_config(
                    discrete_dims=discrete_dims,
                    continuous_dims=continuous_dims,
                    init_args=init_args,
                )
            elif init_args:
                sampler = cls(**init_args)
            else:
                sampler = cls()
            maybe_discrete_dims = getattr(sampler, "discrete_dims", None)
            if maybe_discrete_dims is not None and int(maybe_discrete_dims) != discrete_dims:
                raise ValueError(
                    f"sampler discrete_dims mismatch: expected {discrete_dims}, got {int(maybe_discrete_dims)}"
                )
            maybe_continuous_dims = getattr(sampler, "continuous_dims", None)
            if maybe_continuous_dims is not None and int(maybe_continuous_dims) != continuous_dims:
                raise ValueError(
                    f"sampler continuous_dims mismatch: expected {continuous_dims}, got {int(maybe_continuous_dims)}"
                )
            send({"id": req_id, "ok": True})
        elif op == "sample_plan":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            plan = sampler.sample_plan()
            send({"id": req_id, "ok": True, "plan": plan})
        elif op == "training_samples_remaining":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            remaining = sampler.training_samples_remaining()
            if remaining is not None:
                remaining = int(remaining)
            send({"id": req_id, "ok": True, "remaining": remaining})
        elif op == "produce_latent_batch":
            if sampler is None or discrete_dims is None or continuous_dims is None:
                raise RuntimeError("worker not initialized")
            nr_samples = int(req["nr_samples"])
            xs_discrete, xs_continuous = sampler.produce_latent_batch(nr_samples)
            xs_discrete = np.asarray(xs_discrete, dtype=np.int64)
            xs_continuous = np.asarray(xs_continuous, dtype=np.float64)
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
            send({
                "id": req_id,
                "ok": True,
                "xs_discrete_row_major": xs_discrete.reshape((nr_samples * discrete_dims,)).tolist(),
                "xs_continuous_row_major": xs_continuous.reshape((nr_samples * continuous_dims,)).tolist(),
            })
        elif op == "ingest_training_weights":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            training_weights = np.asarray(req["training_weights"], dtype=np.float64).reshape((-1,))
            sampler.ingest_training_weights(training_weights)
            send({"id": req_id, "ok": True})
        elif op == "pdf":
            if sampler is None or discrete_dims is None or continuous_dims is None:
                raise RuntimeError("worker not initialized")
            nr_samples = int(req["nr_samples"])
            xs_discrete = np.asarray(req["xs_discrete_row_major"], dtype=np.int64).reshape(
                (nr_samples, discrete_dims)
            )
            xs_continuous = np.asarray(
                req["xs_continuous_row_major"], dtype=np.float64
            ).reshape((nr_samples, continuous_dims))
            pdf = (
                sampler.pdf(xs_discrete, xs_continuous)
                if hasattr(sampler, "pdf")
                else None
            )
            if pdf is None:
                send({"id": req_id, "ok": True, "values": None})
                continue
            pdf = np.asarray(pdf, dtype=np.float64).reshape((nr_samples,))
            send({"id": req_id, "ok": True, "values": pdf.tolist()})
        elif op == "snapshot":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            snapshot = sampler.snapshot()
            send({"id": req_id, "ok": True, "snapshot": snapshot})
        elif op == "get_diagnostics":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            diagnostics = sampler.get_diagnostics() if hasattr(sampler, "get_diagnostics") else {}
            send({"id": req_id, "ok": True, "diagnostics": diagnostics})
        else:
            raise ValueError(f"unknown op: {op}")
    except Exception as exc:
        send(
            {
                "id": req_id,
                "ok": False,
                "error": f"{type(exc).__name__}: {exc}",
                "traceback": traceback.format_exc(limit=8),
            }
        )
