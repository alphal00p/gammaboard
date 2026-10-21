"""Independent wire peer for materializer, transform, sampler, and metadata tests."""

import json, struct, sys, time

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
    return json.loads(sys.stdin.buffer.read(content_length))

def send_result(req_id, result, binary=b""):
    body = json.dumps(
        {"jsonrpc": "2.0", "id": req_id, "result": result},
        separators=(",", ":"),
    ).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\nBinary-Length: {len(binary)}\r\n\r\n".encode("ascii"))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.write(binary)
    sys.stdout.buffer.flush()


mode = sys.argv[1]
state = {}
while True:
    req = read_frame()
    if req is None:
        break
    method = req.get("method")
    params = req.get("params") or {}
    if mode == "materializer":
        if method == "initialize":
            send_result(req.get("id"), {"ok": True})
        elif method == "materialize_batch":
            batch = params["latent_batch"]["payload"]
            offsets = [0]
            for width in batch["continuous_layouts"]:
                offsets.append(offsets[-1] + width)
            send_result(req.get("id"), {
                "xs_discrete_row_major": [],
                "xs_discrete_offsets": [0, 0, 0],
                "xs_continuous_row_major": [value * 2.0 for value in batch["continuous_values"]],
                "xs_continuous_offsets": offsets,
                "weights": batch["weights"],
            })
        else:
            raise ValueError(f"unknown method: {method}")
    elif mode == "transform":
        if method == "initialize":
            send_result(req.get("id"), {"ok": True})
        elif method == "transform_batch":
            send_result(req.get("id"), {
                "xs_discrete_row_major": params["xs_discrete_row_major"],
                "xs_discrete_offsets": params["xs_discrete_offsets"],
                "xs_continuous_row_major": [value * 2.0 for value in params["xs_continuous_row_major"]],
                "xs_continuous_offsets": params["xs_continuous_offsets"],
                "weights": [value * 3.0 for value in params["weights"]],
            })
        else:
            raise ValueError(f"unknown method: {method}")
    elif mode == "sampler":
        if method == "initialize":
            time.sleep(float((params.get("args") or {}).get("initialize_delay_seconds", 0)))
            state["evaluator_metadata"] = params.get("evaluator_metadata")
            send_result(req.get("id"), {"ok": True})
        elif method == "generate":
            count = params["max_samples"]
            binary = struct.pack("<" + "d" * (2 * count), *([0.5] * count + [1.0] * count))
            send_result(req.get("id"), {"kind": "batch", "nr_samples": count, "training_remaining": None}, binary)
        elif method == "snapshot":
            send_result(req.get("id"), {"snapshot": state})
        elif method == "persisted_output":
            send_result(req.get("id"), {"output": None})
        elif method == "global_pdf_norm":
            send_result(req.get("id"), {"norm": 1.0})
        elif method == "diagnostics":
            send_result(req.get("id"), {"diagnostics": {}})
        elif method in ("pdf", "discrete_pdf"):
            send_result(req.get("id"), {"values": None})
        else:
            send_result(req.get("id"), {"ok": True})
