from __future__ import annotations

import json
import os
import sys
import traceback
from typing import Any

PROTOCOL = "gammaboard-jsonrpc-v1"
JSON_RPC_VERSION = "2.0"


class ProtocolIo:
    def __init__(self) -> None:
        self.stdout = os.fdopen(os.dup(1), "wb", buffering=0)
        sys.stdout = sys.stderr

    def send_frame(self, payload: dict[str, Any]) -> None:
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        self.stdout.write(f"Content-Length: {len(body)}\r\n\r\n".encode("ascii"))
        self.stdout.write(body)

    def read_frame(self) -> dict[str, Any] | None:
        content_length = None
        while True:
            line = sys.stdin.buffer.readline()
            if not line:
                return None
            decoded = line.decode("ascii", errors="replace").strip()
            if not decoded:
                if content_length is not None:
                    break
                continue
            name, sep, value = decoded.partition(":")
            if sep and name.lower() == "content-length":
                content_length = int(value.strip())
        return json.loads(sys.stdin.buffer.read(content_length).decode("utf-8"))

    def send_result(self, req_id: Any, result: dict[str, Any]) -> None:
        self.send_frame({"jsonrpc": JSON_RPC_VERSION, "id": req_id, "result": result})

    def send_error(self, req_id: Any, exc: BaseException) -> None:
        self.send_frame(
            {
                "jsonrpc": JSON_RPC_VERSION,
                "id": req_id,
                "error": {
                    "code": -32000,
                    "message": f"{type(exc).__name__}: {exc}",
                    "data": {"traceback": traceback.format_exc(limit=8)},
                },
            }
        )


def fixed_domain_shape(domain: dict[str, Any]) -> tuple[list[int], int]:
    if not isinstance(domain, dict):
        raise ValueError("domain must be an object")
    if "continuous" in domain:
        return [], int(domain["continuous"]["dims"])
    if "rectangular" in domain:
        rectangular = domain["rectangular"]
        return (
            [int(value) for value in rectangular.get("discrete_cardinalities", [])],
            int(rectangular["continuous_dims"]),
        )
    if "discrete" not in domain:
        raise ValueError(f"unsupported domain shape: {domain!r}")
    branches = domain["discrete"].get("branches") or []
    if not branches:
        raise ValueError("homogeneous Python wrapper requires non-empty discrete branches")
    tail_cardinalities = None
    tail_continuous_dims = None
    for branch in branches:
        cardinalities, continuous = fixed_domain_shape(branch["domain"])
        if tail_cardinalities is None:
            tail_cardinalities = cardinalities
            tail_continuous_dims = continuous
        elif cardinalities != tail_cardinalities or continuous != tail_continuous_dims:
            raise ValueError("homogeneous Python wrapper does not support inhomogeneous domains")
    return [len(branches), *tail_cardinalities], int(tail_continuous_dims)


def require_homogeneous_offsets(
    params: dict[str, Any], field: str, nr_samples: int, width: int, role: str
) -> None:
    expected = [index * width for index in range(nr_samples + 1)]
    raw = params.get(field)
    if raw is None:
        return
    offsets = [int(value) for value in raw]
    if offsets != expected:
        raise ValueError(
            f"python {role} wrapper only supports homogeneous batches; "
            f"{field}={offsets} does not match fixed width {width}"
        )


def instantiate_user_object(target: Any, *, discrete_cardinalities: list[int], continuous_dims: int, init_args: dict[str, Any], snapshot: Any = None) -> Any:
    if not isinstance(init_args, dict):
        raise TypeError("args must be an object")
    if not isinstance(target, type):
        if snapshot is not None:
            raise TypeError("snapshot restore requires passing a sampler class to run_sampler, not an instance")
        return target
    if snapshot is not None:
        if not hasattr(target, "from_snapshot"):
            raise TypeError("class must define from_snapshot(...) when restoring from snapshot")
        if isinstance(snapshot, dict) and "save_path" not in snapshot and "save_path" in init_args:
            snapshot = {**snapshot, "save_path": init_args["save_path"]}
        return target.from_snapshot(
            snapshot=snapshot,
            discrete_cardinalities=discrete_cardinalities,
            continuous_dims=continuous_dims,
            init_args=init_args,
        )
    if hasattr(target, "from_config"):
        return target.from_config(
            discrete_cardinalities=discrete_cardinalities,
            continuous_dims=continuous_dims,
            init_args=init_args,
        )
    return target(**init_args) if init_args else target()


def validate_declared_shape(obj: Any, discrete_cardinalities: list[int], continuous_dims: int, role: str) -> None:
    maybe_discrete_cardinalities = getattr(obj, "discrete_cardinalities", None)
    if maybe_discrete_cardinalities is not None:
        parsed = [int(value) for value in maybe_discrete_cardinalities]
        if parsed != discrete_cardinalities:
            raise ValueError(
                f"{role} discrete_cardinalities mismatch: expected {discrete_cardinalities}, got {maybe_discrete_cardinalities}"
            )
    maybe_discrete_dims = getattr(obj, "discrete_dims", None)
    if maybe_discrete_dims is not None and int(maybe_discrete_dims) != len(discrete_cardinalities):
        raise ValueError(
            f"{role} discrete_dims mismatch: expected {len(discrete_cardinalities)}, got {int(maybe_discrete_dims)}"
        )
    maybe_continuous_dims = getattr(obj, "continuous_dims", None)
    if maybe_continuous_dims is not None and int(maybe_continuous_dims) != continuous_dims:
        raise ValueError(
            f"{role} continuous_dims mismatch: expected {continuous_dims}, got {int(maybe_continuous_dims)}"
        )


def normalize_discrete_subspaces(params: dict[str, Any]) -> list[dict[int, int]]:
    raw_subspaces = params.get("subspaces", [])
    if not isinstance(raw_subspaces, list):
        raise TypeError("subspaces must be an array")
    normalized = []
    for subspace in raw_subspaces:
        if not isinstance(subspace, dict):
            raise TypeError("each subspace must be an object")
        raw = subspace.get("fixed_dims", [])
        if isinstance(raw, dict):
            normalized.append({int(dim): int(value) for dim, value in raw.items()})
        elif isinstance(raw, list):
            normalized.append({int(entry["dim"]): int(entry["value"]) for entry in raw})
        else:
            raise TypeError("subspace.fixed_dims must be an array or object")
    return normalized
