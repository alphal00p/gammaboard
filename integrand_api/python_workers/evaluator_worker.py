import importlib
import json
import traceback
import numpy as np
import sys

integrand = None
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
            if hasattr(cls, "from_config"):
                integrand = cls.from_config(input_dim=input_dim, init_args=init_args)
            elif init_args:
                integrand = cls(**init_args)
            else:
                integrand = cls()
            maybe_dim = getattr(integrand, "input_dim", None)
            if maybe_dim is not None and int(maybe_dim) != input_dim:
                raise ValueError(
                    f"integrand input_dim mismatch: expected {input_dim}, got {int(maybe_dim)}"
                )
            send({"id": req_id, "ok": True})
        elif op == "eval_scalar":
            if integrand is None or input_dim is None:
                raise RuntimeError("worker not initialized")
            nr_samples = int(req["nr_samples"])
            req_dim = int(req["input_dim"])
            if req_dim != input_dim:
                raise ValueError(f"input_dim mismatch: worker={input_dim} request={req_dim}")
            xs = np.asarray(req["xs_row_major"], dtype=np.float64).reshape((nr_samples, req_dim))
            ys = np.asarray(integrand.eval(xs), dtype=np.float64).reshape((nr_samples,))
            send({"id": req_id, "ok": True, "values": ys.tolist()})
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
