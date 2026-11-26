"""Shared evaluator descriptors, requested channels and unweighted sample output."""

import copy
import math
import struct
from dataclasses import dataclass
from typing import Any

import numpy as np

from .domain import Domain

EVALUATION_PROTOCOL = "gammaboard-evaluation-v1"
BINARY_ENCODING = "gammaboard-evaluation-binary-v1"


@dataclass
class EvaluatorDescriptor:
    domain: Domain
    components: list[str]
    observables: bool = False
    details: bool = False
    diagnostics: bool = False
    scalar_projection: str | None = None
    observable_schema: dict | None = None

    def __post_init__(self):
        if isinstance(self.domain, dict):
            self.domain = Domain.from_dict(self.domain)
        self.domain.validate()
        self.components = list(self.components)
        if (
            not self.components
            or any(not isinstance(v, str) or not v.strip() for v in self.components)
            or len(set(self.components)) != len(self.components)
        ):
            raise ValueError("descriptor components must be nonempty and unique")
        if self.scalar_projection is not None and (
            self.scalar_projection not in ("real", "imag", "abs", "abs_sq")
            or not {"real", "imag"} <= set(self.components)
        ):
            raise ValueError(
                "scalar projection requires real/imag components and a supported projection"
            )

    def to_dict(self):
        result = {
            "domain": self.domain.to_dict(),
            "components": self.components,
            "observables": self.observables,
            "details": self.details,
            "diagnostics": self.diagnostics,
        }
        if self.scalar_projection is not None:
            result["scalar_projection"] = self.scalar_projection
        if self.observable_schema is not None:
            result["observable_schema"] = self.observable_schema
        return result


@dataclass(frozen=True)
class EvaluationRequest:
    components: list[str]
    observables: bool = False
    details: bool = False
    diagnostics: bool = False

    def validate(self, descriptor):
        if (
            not self.components
            or len(set(self.components)) != len(self.components)
            or not set(self.components) <= set(descriptor.components)
        ):
            raise ValueError("unsupported evaluation components")
        for field_name in ("observables", "details", "diagnostics"):
            requested = getattr(self, field_name)
            if type(requested) is not bool or (
                requested and not getattr(descriptor, field_name)
            ):
                raise ValueError(f"unsupported evaluation channel: {field_name}")


@dataclass(slots=True)
class EvaluationBatch:
    """One output per original input, before host weights/masking/reduction.

    Values follow request.components (or are a mapping keyed by component name).
    Optional channels are lists of per-sample dictionaries. None means unavailable,
    not measured zero. Observable fills must be coalesced within each sample.
    """

    values: Any
    observables: list[dict] | None = None
    details: list[dict] | None = None
    diagnostics: list[dict] | None = None


DIAGNOSTIC_COUNTS = (
    "count_total",
    "count_double_precision",
    "count_quad_precision",
    "count_arb_precision",
    "count_nan",
    "count_nan_or_unstable",
    "count_loop_momenta_escalated",
    "total_generated_events",
    "total_accepted_events",
)
DIAGNOSTIC_TIMES = (
    "total_eval_time_ms",
    "total_integrand_eval_time_ms",
    "total_evaluator_eval_time_ms",
    "total_parameterization_time_ms",
    "total_event_processing_time_ms",
)
OPTIONAL_TIMES = (
    "total_canonical_sampling_preparation_time_ms",
    "total_canonical_physical_preparation_time_ms",
)


def normalize_values(values, nr_samples, components):
    if isinstance(values, dict):
        values = np.stack(
            [
                np.asarray(values[name], dtype=np.float64).reshape((nr_samples,))
                for name in components
            ],
            axis=1,
        )
    array = np.asarray(values, dtype=np.float64)
    if len(components) == 1 and array.shape == (nr_samples,):
        array = array.reshape((nr_samples, 1))
    if array.shape != (nr_samples, len(components)):
        raise ValueError(
            f"eval returned shape {array.shape}, expected ({nr_samples},) or ({nr_samples}, {len(components)})"
        )
    finite = np.isfinite(array)
    if not finite.all():
        sample, component = np.argwhere(~finite)[0]
        raise ValueError(
            f"sample {sample}, component {components[component]!r}: eval returned non-finite value"
        )
    return array


def _float(value):
    value = float(value)
    if not math.isfinite(value):
        raise ValueError("non-finite evaluation output")
    return value


def _counter(value, maximum=(1 << 64) - 1):
    if type(value) is not int or not 0 <= value <= maximum:
        raise ValueError("invalid evaluation counter")
    return value


def validate_batch(result, request, count):
    for name in ("observables", "details", "diagnostics"):
        rows = getattr(result, name)
        if getattr(request, name) and (
            rows is None or len(rows) != count or any(row is None for row in rows)
        ):
            raise ValueError(f"missing or invalid per-sample {name}")
    if request.observables:
        for bundle in result.observables:
            for histogram in bundle["histograms"].values():
                _counter(histogram["sample_count"])
                if histogram["sample_count"] != 1:
                    raise ValueError(
                        "observable output must describe exactly one sample"
                    )
                for count in histogram["statistics"].values():
                    _counter(count)
                for b in [
                    *histogram["bins"],
                    histogram["underflow_bin"],
                    histogram["overflow_bin"],
                ]:
                    w, square = (
                        _float(b["sum_weights"]),
                        _float(b["sum_weights_squared"]),
                    )
                    if not math.isclose(square, w * w, rel_tol=1e-12, abs_tol=0):
                        raise ValueError(
                            "observable second moment must square the complete sample contribution"
                        )
    if request.details:
        for d in result.details:
            _float(d["integrand_value_re"])
            _float(d["integrand_value_im"])
            if d.get("parameterization_jacobian") is not None:
                _float(d["parameterization_jacobian"])
    if request.diagnostics:
        for d in result.diagnostics:
            if d["count_total"] != 1:
                raise ValueError("diagnostics must describe one sample")
            for key in DIAGNOSTIC_COUNTS:
                _counter(d[key], (1 << 63) - 1)
            for key in DIAGNOSTIC_TIMES:
                if _float(d[key]) < 0:
                    raise ValueError("negative diagnostic timing")
            for key in OPTIONAL_TIMES:
                if d.get(key) is not None and _float(d[key]) < 0:
                    raise ValueError("negative diagnostic timing")


def clear_schema(bundle):
    schema = copy.deepcopy(bundle)
    for h in schema["histograms"].values():
        h["sample_count"] = 0
        h["statistics"] = {key: 0 for key in h["statistics"]}
        for b in [*h["bins"], h["underflow_bin"], h["overflow_bin"]]:
            b.update(
                entry_count=0,
                sum_weights=0.0,
                sum_weights_squared=0.0,
                mitigated_fill_count=0,
            )
    return schema


class EvaluationEncoder:
    def __init__(self):
        self.schema = None
        self.sent_schema = False

    def encode(self, result, request, values):
        count = len(values)
        validate_batch(result, request, count)
        if not (request.observables or request.details or request.diagnostics):
            return {
                "encoding": BINARY_ENCODING,
                "__binary__": np.ascontiguousarray(values, dtype="<f8").tobytes(),
            }
        data = bytearray()

        def optional(value):
            data.append(int(value is not None))
            if value is not None:
                data.extend(struct.pack("<d", _float(value)))

        for i, row in enumerate(values):
            data.extend(np.asarray(row, dtype="<f8").tobytes())
            if request.observables:
                bundle = result.observables[i]
                schema = clear_schema(bundle)
                if self.schema is None:
                    self.schema = schema
                elif schema != self.schema:
                    raise ValueError("observable schema changed; use JSON transport")
                for name in sorted(bundle["histograms"]):
                    h = bundle["histograms"][name]
                    bins = [*h["bins"], h["underflow_bin"], h["overflow_bin"]]
                    occupied = [
                        (j, b)
                        for j, b in enumerate(bins)
                        if b["entry_count"]
                        or b["mitigated_fill_count"]
                        or struct.pack(
                            "<dd", b["sum_weights"], b["sum_weights_squared"]
                        )
                        != bytes(16)
                    ]
                    data.extend(
                        struct.pack(
                            "<QQQI",
                            *[
                                _counter(h["statistics"][key])
                                for key in (
                                    "in_range_entry_count",
                                    "nan_value_count",
                                    "mitigated_pair_count",
                                )
                            ],
                            len(occupied),
                        )
                    )
                    for j, b in occupied:
                        data.extend(
                            struct.pack(
                                "<IQddQ",
                                j,
                                _counter(b["entry_count"]),
                                _float(b["sum_weights"]),
                                _float(b["sum_weights_squared"]),
                                _counter(b["mitigated_fill_count"]),
                            )
                        )
            if request.details:
                d = result.details[i]
                data.extend(
                    struct.pack("<dd", d["integrand_value_re"], d["integrand_value_im"])
                )
                optional(d.get("parameterization_jacobian"))
            if request.diagnostics:
                d = result.diagnostics[i]
                data.extend(
                    struct.pack(
                        "<9q5d",
                        *[d[key] for key in DIAGNOSTIC_COUNTS],
                        *[d[key] for key in DIAGNOSTIC_TIMES],
                    )
                )
                for key in OPTIONAL_TIMES:
                    optional(d.get(key))
        response = {"encoding": BINARY_ENCODING, "__binary__": bytes(data)}
        if self.schema is not None and not self.sent_schema:
            response["observable_schema"] = self.schema
            self.sent_schema = True
        return response


def json_response(result, request, values):
    validate_batch(result, request, len(values))
    rows = []
    for i, value in enumerate(values):
        row = {"values": value.tolist()}
        for name in ("observables", "details", "diagnostics"):
            if getattr(request, name):
                row[name] = getattr(result, name)[i]
        rows.append(row)
    return {"samples": rows}


class EvaluationDecoder:
    """Decode either selected-output encoding; retain the histogram schema across calls."""

    def __init__(self, schema=None):
        self.schema = schema
        self.received_schema = False

    def decode(self, response, binary, request, count):
        if response.get("encoding") is None:
            if binary or response.get("observable_schema") is not None:
                raise ValueError("unexpected binary/schema in JSON evaluation")
            rows = response.get("samples")
            if not isinstance(rows, list) or len(rows) != count:
                raise ValueError("evaluation result count mismatch")
        else:
            if (
                response["encoding"] != BINARY_ENCODING
                or response.get("samples") is not None
            ):
                raise ValueError("unsupported evaluation encoding")
            schema = response.get("observable_schema")
            if schema is not None:
                if self.received_schema or (
                    self.schema is not None and self.schema != schema
                ):
                    raise ValueError("observable schema changed or repeated")
                if clear_schema(schema) != schema:
                    raise ValueError("observable schema is not cleared")
                self.schema = schema
                self.received_schema = True
            position = 0

            def read(fmt):
                nonlocal position
                size = struct.calcsize(fmt)
                if position + size > len(binary):
                    raise ValueError("truncated evaluation binary")
                value = struct.unpack_from(fmt, binary, position)
                position += size
                return value

            def optional():
                (tag,) = read("<B")
                if tag not in (0, 1):
                    raise ValueError("invalid optional tag")
                return read("<d")[0] if tag else None

            rows = []
            for _ in range(count):
                row = {"values": list(read("<" + "d" * len(request.components)))}
                if request.observables:
                    if self.schema is None:
                        raise ValueError("missing observable schema")
                    bundle = copy.deepcopy(self.schema)
                    for name in sorted(bundle["histograms"]):
                        h = bundle["histograms"][name]
                        h["sample_count"] = 1
                        counts = read("<QQQI")
                        h["statistics"] = dict(
                            zip(
                                (
                                    "in_range_entry_count",
                                    "nan_value_count",
                                    "mitigated_pair_count",
                                ),
                                counts[:3],
                            )
                        )
                        bins = [*h["bins"], h["underflow_bin"], h["overflow_bin"]]
                        previous = -1
                        if counts[3] > len(bins):
                            raise ValueError("too many populated bins")
                        for _ in range(counts[3]):
                            index, entries, weight, square, mitigated = read("<IQddQ")
                            if not previous < index < len(bins):
                                raise ValueError("invalid sparse bin index")
                            previous = index
                            bins[index].update(
                                entry_count=entries,
                                sum_weights=weight,
                                sum_weights_squared=square,
                                mitigated_fill_count=mitigated,
                            )
                    row["observables"] = bundle
                if request.details:
                    re, im = read("<dd")
                    row["details"] = dict(
                        integrand_value_re=re,
                        integrand_value_im=im,
                        parameterization_jacobian=optional(),
                    )
                if request.diagnostics:
                    d = dict(zip(DIAGNOSTIC_COUNTS, read("<9q")))
                    d.update(zip(DIAGNOSTIC_TIMES, read("<5d")))
                    d.update((key, optional()) for key in OPTIONAL_TIMES)
                    row["diagnostics"] = d
                rows.append(row)
            if position != len(binary):
                raise ValueError("trailing evaluation binary")
        result = EvaluationBatch(
            values=np.asarray([r["values"] for r in rows], dtype=float).reshape(
                count, len(request.components)
            ),
            **{
                name: [r.get(name) for r in rows] if getattr(request, name) else None
                for name in ("observables", "details", "diagnostics")
            },
        )
        normalize_values(result.values, count, request.components)
        validate_batch(result, request, count)
        return rows
