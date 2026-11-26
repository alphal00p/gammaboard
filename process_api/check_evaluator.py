#!/usr/bin/env python3
"""Check an external evaluator using explicit physical points and the public protocol.

No database or GammaLoop installation is required in the checker. The executable
may use its own dependencies. This checks consistency, not physical normalization.
Exit: 0 passed, 1 failed, 2 invalid setup/inconclusive, 130 interrupted.
"""

from __future__ import annotations

import argparse
import copy
import json
import math
import struct
import sys
import time
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent / "python/src"))
from check_common import CheckError, RemoteError, RpcWorker, Unexercised
from gammaboard_process import EvaluationRequest, EvaluatorDescriptor
from gammaboard_process.evaluation import (
    BINARY_ENCODING,
    EVALUATION_PROTOCOL,
    EvaluationDecoder,
)
from gammaboard_process.protocol import PROTOCOL


def load_json(value):
    return json.loads(Path(value[1:]).read_text() if value.startswith("@") else value)


def inputs(points):
    discrete = []
    continuous = []
    do = [0]
    co = [0]
    for point in points:
        discrete.extend(point["discrete"])
        continuous.extend(point["continuous"])
        do.append(len(discrete))
        co.append(len(continuous))
    binary = struct.pack(
        "<" + "q" * len(discrete) + "d" * len(continuous), *discrete, *continuous
    )
    return {
        "nr_samples": len(points),
        "xs_discrete_offsets": do,
        "xs_continuous_offsets": co,
    }, binary


def comparable(value):
    if isinstance(value, dict):
        # Only timing diagnostics vary between equivalent evaluations.
        return {
            k: (
                {name: v for name, v in item.items() if not name.endswith("_ms")}
                if k == "diagnostics"
                else comparable(item)
            )
            for k, item in value.items()
        }
    if isinstance(value, list):
        return [comparable(v) for v in value]
    return value


def compare(actual, expected, rtol, atol, path="output"):
    if isinstance(expected, dict):
        if not isinstance(actual, dict):
            raise CheckError(f"{path}: expected an object")
        # Optional Jacobians use either omission or explicit null.
        keys = set(expected) | set(actual)
        for key in keys:
            if key not in expected or key not in actual:
                if (
                    key == "parameterization_jacobian"
                    and actual.get(key) is None
                    and expected.get(key) is None
                ):
                    continue
                raise CheckError(f"{path}: different fields")
            compare(actual[key], expected[key], rtol, atol, f"{path}.{key}")
    elif isinstance(expected, list):
        if not isinstance(actual, list) or len(actual) != len(expected):
            raise CheckError(f"{path}: different lengths")
        for i, (a, b) in enumerate(zip(actual, expected)):
            compare(a, b, rtol, atol, f"{path}[{i}]")
    elif isinstance(expected, (int, float)) and not isinstance(expected, bool):
        counter = (
            path.endswith("count")
            or path.endswith("events")
            or path.endswith("bin_id")
            or ".statistics." in path
            or ".diagnostics." in path
        )
        if counter:
            if type(actual) is not int or actual != expected:
                raise CheckError(f"{path}: counter mismatch {actual} != {expected}")
        elif (
            not isinstance(actual, (int, float))
            or not math.isfinite(actual)
            or not math.isclose(actual, expected, rel_tol=rtol, abs_tol=atol)
        ):
            raise CheckError(
                f"{path}: {actual} != {expected} (rtol={rtol}, atol={atol})"
            )
    elif actual != expected:
        raise CheckError(f"{path}: {actual!r} != {expected!r}")


class EvaluatorWorker(RpcWorker):
    def __init__(self, options):
        super().__init__(
            options,
            options.command,
            "evaluator",
            dict(
                protocol=PROTOCOL,
                role="evaluator",
                args=options.init_args,
                domain=options.domain,
            ),
        )
        try:
            if self.initialized.get("evaluation_protocol") != EVALUATION_PROTOCOL:
                raise Unexercised(
                    "worker has no selected-output contract; legacy workers cannot exercise this checker"
                )
            self.descriptor = EvaluatorDescriptor(**self.initialized["descriptor"])
            self.encodings = self.initialized.get("evaluation_encodings", ["json"])
            self.decoder = EvaluationDecoder(self.descriptor.observable_schema)
        except BaseException:
            self.close()
            raise

    def evaluate(self, points, request, encoding):
        request.validate(self.descriptor)
        params, binary = inputs(points)
        response, output = self.call(
            "evaluate",
            params | {"outputs": vars(request), "encoding": encoding},
            binary,
        )
        if (encoding == "json") != (response.get("encoding") is None):
            raise CheckError("worker ignored requested encoding")
        return self.decoder.decode(response, output, request, len(points))


def check(options):
    checks = options.checks = []

    def record(name, fn):
        try:
            fn()
        except (CheckError, ValueError, KeyError, TypeError) as error:
            checks.append(dict(check=name, status="fail", reason=str(error)))
            raise CheckError(f"{name}: {error}") from error
        checks.append(dict(check=name, status="pass"))

    compare_rows = lambda a, b: compare(
        comparable(a), comparable(b), options.rtol, options.atol
    )
    baseline = None
    descriptor = None
    with EvaluatorWorker(options) as worker:
        descriptor = worker.descriptor
        request = EvaluationRequest(
            descriptor.components,
            descriptor.observables,
            descriptor.details,
            descriptor.diagnostics,
        )
        encoding = BINARY_ENCODING if BINARY_ENCODING in worker.encodings else "json"
        if encoding not in worker.encodings:
            raise Unexercised("no supported output encoding advertised")
        baseline = worker.evaluate(options.points, request, encoding)
        checks.append(dict(check="descriptor and full output", status="pass"))
        record(
            "empty batch",
            lambda: compare_rows(worker.evaluate([], request, encoding), []),
        )
        for size in sorted(set([1, 2, 3, len(options.points)])):
            rows = []
            for i in range(0, len(options.points), size):
                rows.extend(
                    worker.evaluate(options.points[i : i + size], request, encoding)
                )
            record(f"batch size {size}", lambda: compare_rows(rows, baseline))
        for components in [
            list(reversed(descriptor.components)),
            *[[c] for c in descriptor.components],
        ]:
            selected = EvaluationRequest(components)
            expected = [
                {
                    "values": [
                        row["values"][descriptor.components.index(c)]
                        for c in components
                    ]
                }
                for row in baseline
            ]
            record(
                f"components {components}",
                lambda: compare_rows(
                    worker.evaluate(options.points, selected, encoding), expected
                ),
            )
        if BINARY_ENCODING in worker.encodings and "json" in worker.encodings:
            record(
                "JSON/binary equivalence",
                lambda: compare_rows(
                    worker.evaluate(
                        options.points,
                        request,
                        "json" if encoding != "json" else BINARY_ENCODING,
                    ),
                    baseline,
                ),
            )
        else:
            checks.append(
                dict(
                    check="JSON/binary equivalence",
                    status="skip",
                    reason="both encodings are not advertised",
                )
            )
        if options.reference is not None:
            record(
                "reference outputs", lambda: compare_rows(baseline, options.reference)
            )
        else:
            checks.append(
                dict(
                    check="reference outputs",
                    status="skip",
                    reason="no reference file; physics normalization is not certified",
                )
            )
        base, binary = inputs(options.points)
        base |= {"outputs": vars(request), "encoding": encoding}
        bad = [
            (
                "unknown component",
                base | {"outputs": {"components": ["__unsupported_component__"]}},
                binary,
            ),
            (
                "unknown encoding",
                base | {"encoding": "__unsupported_encoding__"},
                binary,
            ),
            ("truncated coordinates", base, binary[:-1] if binary else b"\0"),
            ("extra coordinate bytes", base, binary + b"\0" * 8),
            (
                "bad offsets",
                base | {"xs_continuous_offsets": [1] * (len(options.points) + 1)},
                binary,
            ),
        ]
        for channel in ("observables", "details", "diagnostics"):
            if not getattr(descriptor, channel):
                bad.append(
                    (
                        f"unsupported {channel}",
                        base
                        | {
                            "outputs": {
                                "components": descriptor.components,
                                channel: True,
                            }
                        },
                        binary,
                    )
                )
        for coordinate in ("discrete", "continuous"):
            index = next(
                (i for i, p in enumerate(options.points) if p[coordinate]), None
            )
            if index is None:
                checks.append(
                    dict(
                        check=f"invalid {coordinate} coordinate",
                        status="skip",
                        reason="fixture has no such coordinates",
                    )
                )
                continue
            for value in (
                [-1, 2**62]
                if coordinate == "discrete"
                else [float("nan"), float("inf")]
            ):
                points = copy.deepcopy(options.points)
                points[index][coordinate][0] = value
                params, blob = inputs(points)
                bad.append((f"invalid {coordinate} {value}", base | params, blob))
        for name, params, blob in bad:
            try:
                worker.call("evaluate", params, blob)
            except RemoteError:
                pass
            else:
                raise CheckError(f"{name}: invalid request was accepted")
            record(
                name + " and recovery",
                lambda: compare_rows(
                    worker.evaluate(options.points, request, encoding), baseline
                ),
            )
    with EvaluatorWorker(options) as worker:
        compare(
            worker.descriptor.to_dict(), descriptor.to_dict(), 0.0, 0.0, "descriptor"
        )
        record(
            "fresh-process replay",
            lambda: compare_rows(
                worker.evaluate(options.points, request, encoding), baseline
            ),
        )
    return checks, descriptor.to_dict()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--command",
        required=True,
        help="JSON executable/argument array; never a shell command",
    )
    parser.add_argument("--cwd")
    parser.add_argument("--init-args", default="{}", help="JSON or @file")
    parser.add_argument(
        "--domain", help="optional legacy/configured domain assertion, JSON or @file"
    )
    parser.add_argument(
        "--points",
        type=Path,
        required=True,
        help="JSON array of {discrete:[],continuous:[]} physical points",
    )
    parser.add_argument(
        "--reference", type=Path, help="expected per-sample selected-output records"
    )
    parser.add_argument("--rtol", type=float, default=1e-10)
    parser.add_argument("--atol", type=float, default=1e-12)
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("--time-limit", type=float, default=120)
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args(argv)
    report = {"format_version": 1, "rtol": args.rtol, "atol": args.atol}
    try:
        args.command = load_json(args.command)
        args.init_args = load_json(args.init_args)
        args.domain = load_json(args.domain) if args.domain else None
        args.points = json.loads(args.points.read_text())
        args.reference = (
            json.loads(args.reference.read_text()) if args.reference else None
        )
        if (
            not isinstance(args.command, list)
            or not args.command
            or any(not isinstance(c, str) or not c for c in args.command)
        ):
            raise ValueError("command must be a nonempty string array")
        if not isinstance(args.points, list) or not args.points:
            raise ValueError("provide at least one physical point")
        for p in args.points:
            if (
                set(p) != {"discrete", "continuous"}
                or any(type(d) is not int or d < 0 for d in p["discrete"])
                or any(not math.isfinite(x) for x in p["continuous"])
            ):
                raise ValueError("invalid fixture point")
        if any(not math.isfinite(v) or v < 0 for v in (args.rtol, args.atol)) or any(
            not math.isfinite(v) or v <= 0 for v in (args.timeout, args.time_limit)
        ):
            raise ValueError("invalid tolerance or deadline")
        args.deadline = time.monotonic() + args.time_limit
        try:
            checks, descriptor = check(args)
        except (
            ValueError,
            KeyError,
            TypeError,
            AttributeError,
            OverflowError,
            struct.error,
        ) as error:
            raise CheckError(f"malformed evaluator response: {error}") from error
        report.update(status="passed", checks=checks, descriptor=descriptor)
        code = 0
    except KeyboardInterrupt:
        report.update(status="interrupted")
        code = 130
    except Unexercised as e:
        report.update(status="inconclusive", error=str(e))
        code = 2
    except (ValueError, OSError, KeyError, TypeError) as e:
        report.update(status="invalid_or_malformed", error=str(e))
        code = 2
    except CheckError as e:
        report.update(status="failed", error=str(e))
        code = 1
    if hasattr(args, "checks"):
        report["checks"] = args.checks
    print(
        json.dumps(report, indent=2, allow_nan=False)
        if args.json
        else f"Evaluator check: {report['status']}\n" + json.dumps(report, indent=2)
    )
    return code


if __name__ == "__main__":
    raise SystemExit(main())
