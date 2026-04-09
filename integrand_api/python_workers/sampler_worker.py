import importlib
import json
import traceback
import numpy as np
import sys

sampler = None
input_dim = None


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
            input_dim = int(req["input_dim"])
            init_args = req.get("init_args") or {}
            if not isinstance(init_args, dict):
                raise TypeError("init_args must be an object")
            snapshot = req.get("snapshot")
            if snapshot is not None:
                if not hasattr(cls, "from_snapshot"):
                    raise TypeError("class must define from_snapshot(...) when restoring from snapshot")
                sampler = cls.from_snapshot(snapshot=snapshot, input_dim=input_dim, init_args=init_args)
            elif hasattr(cls, "from_config"):
                sampler = cls.from_config(input_dim=input_dim, init_args=init_args)
            elif init_args:
                sampler = cls(**init_args)
            else:
                sampler = cls()
            maybe_dim = getattr(sampler, "input_dim", None)
            if maybe_dim is not None and int(maybe_dim) != input_dim:
                raise ValueError(
                    f"sampler input_dim mismatch: expected {input_dim}, got {int(maybe_dim)}"
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
            if sampler is None or input_dim is None:
                raise RuntimeError("worker not initialized")
            nr_samples = int(req["nr_samples"])
            xs = np.asarray(sampler.produce_latent_batch(nr_samples), dtype=np.float64)
            if xs.shape != (nr_samples, input_dim):
                raise ValueError(
                    f"produce_latent_batch returned shape {xs.shape}, expected ({nr_samples}, {input_dim})"
                )
            if not np.isfinite(xs).all():
                raise ValueError("produce_latent_batch returned non-finite values")
            send({"id": req_id, "ok": True, "xs_row_major": xs.reshape((nr_samples * input_dim,)).tolist()})
        elif op == "ingest_training_weights":
            if sampler is None:
                raise RuntimeError("worker not initialized")
            training_weights = np.asarray(req["training_weights"], dtype=np.float64).reshape((-1,))
            sampler.ingest_training_weights(training_weights)
            send({"id": req_id, "ok": True})
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
