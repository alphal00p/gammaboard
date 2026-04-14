import importlib
import json
import traceback
import numpy as np
import sys

integrand = None
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
            if hasattr(cls, "from_config"):
                integrand = cls.from_config(
                    discrete_dims=discrete_dims,
                    continuous_dims=continuous_dims,
                    init_args=init_args,
                )
            elif init_args:
                integrand = cls(**init_args)
            else:
                integrand = cls()
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
            send({"id": req_id, "ok": True})
        elif op == "eval_scalar":
            if integrand is None or discrete_dims is None or continuous_dims is None:
                raise RuntimeError("worker not initialized")
            nr_samples = int(req["nr_samples"])
            req_discrete_dims = int(req["discrete_dims"])
            req_continuous_dims = int(req["continuous_dims"])
            if req_discrete_dims != discrete_dims or req_continuous_dims != continuous_dims:
                raise ValueError(
                    f"dimension mismatch: worker=({discrete_dims}, {continuous_dims}) request=({req_discrete_dims}, {req_continuous_dims})"
                )
            xs_discrete = np.asarray(req["xs_discrete_row_major"], dtype=np.int64).reshape(
                (nr_samples, req_discrete_dims)
            )
            xs_continuous = np.asarray(
                req["xs_continuous_row_major"], dtype=np.float64
            ).reshape((nr_samples, req_continuous_dims))
            ys = np.asarray(
                integrand.eval(xs_discrete, xs_continuous), dtype=np.float64
            ).reshape((nr_samples,))
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
