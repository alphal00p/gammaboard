"""One coordinate codec for all roles, supporting rectangular and ragged domains."""

import numpy as np

from .batches import Batch
from .domain import Domain, RectangularDomain


class BatchCodec:
    def __init__(self, domain: Domain):
        domain.validate()
        self.domain = domain
        self.widths = domain.fixed_widths()

    def offsets(self, params, key, n, width):
        offsets = params.get(key)
        if offsets is None:
            if width is None:
                raise ValueError(f"{key} is required for a ragged domain")
            return np.arange(n + 1, dtype=np.int64) * width
        if (
            len(offsets) != n + 1
            or any(type(v) is not int or v < 0 for v in offsets)
            or offsets[0] != 0
            or any(a > b for a, b in zip(offsets, offsets[1:]))
        ):
            raise ValueError(f"invalid {key}")
        if width is not None and offsets != [i * width for i in range(n + 1)]:
            raise ValueError(f"{key} must match fixed width {width}")
        return np.asarray(offsets, dtype=np.int64)

    @staticmethod
    def rows(flat, offsets, width):
        if len(flat) != offsets[-1]:
            raise ValueError("coordinate length does not match offsets")
        if width is not None:
            return np.array(flat.reshape(len(offsets) - 1, width), copy=True)
        return tuple(
            np.array(flat[a:b], copy=True) for a, b in zip(offsets, offsets[1:])
        )

    def decode(self, params, binary=None, *, weighted=False):
        n = params["nr_samples"]
        if type(n) is not int or n < 0:
            raise ValueError("invalid sample count")
        do = self.offsets(params, "xs_discrete_offsets", n, self.widths[0])
        co = self.offsets(params, "xs_continuous_offsets", n, self.widths[1])
        if binary is not None:
            size = (int(do[-1]) + int(co[-1]) + (n if weighted else 0)) * 8
            if len(binary) != size:
                raise ValueError("invalid evaluator input length")
            d = np.frombuffer(binary, dtype="<i8", count=int(do[-1]))
            c = np.frombuffer(
                binary, dtype="<f8", count=int(co[-1]), offset=int(do[-1]) * 8
            )
            weights = (
                np.frombuffer(
                    binary, dtype="<f8", count=n, offset=(int(do[-1]) + int(co[-1])) * 8
                ).copy()
                if weighted
                else np.ones(n)
            )
        else:
            d = np.asarray(params["xs_discrete_row_major"], dtype=np.int64)
            c = np.asarray(params["xs_continuous_row_major"], dtype=np.float64)
            weights = (
                np.asarray(params["weights"], dtype=np.float64).copy()
                if weighted
                else np.ones(n)
            )
        batch = Batch(
            self.rows(d, do, self.widths[0]), self.rows(c, co, self.widths[1]), weights
        )
        self.validate(batch)
        return batch

    def validate(self, batch):
        weights = batch.weights
        if weights.ndim != 1 or not np.isfinite(weights).all() or (weights <= 0).any():
            raise ValueError("sampling weights must be finite and positive")
        n = len(weights)
        if len(batch.xs_discrete) != n or len(batch.xs_continuous) != n:
            raise ValueError("batch sample count mismatch")
        if isinstance(self.domain, RectangularDomain):
            if not np.isfinite(batch.xs_continuous).all():
                raise ValueError("continuous inputs must be finite")
            for axis, cardinality in enumerate(self.domain.discrete_cardinalities):
                values = batch.xs_discrete[:, axis]
                if ((values < 0) | (values >= cardinality)).any():
                    raise ValueError("discrete coordinate out of bounds")
        else:
            for d, c in zip(batch.xs_discrete, batch.xs_continuous):
                if not np.isfinite(c).all():
                    raise ValueError("continuous inputs must be finite")
                self.domain.validate_point(d, c)

    def encode(self, batch, n, *, binary=False):
        if not isinstance(batch, Batch):
            raise TypeError("callback must return Batch")
        weights = np.asarray(batch.weights, dtype=np.float64)
        if weights.shape != (n,):
            raise ValueError("batch sample count mismatch")
        normalized, flats, response = [], [], {}
        for values, width, dtype, key in zip(
            (batch.xs_discrete, batch.xs_continuous),
            self.widths,
            ("<i8", "<f8"),
            ("xs_discrete", "xs_continuous"),
        ):
            if width is not None:
                rows = np.asarray(values, dtype=dtype).reshape(n, width)
                flat = np.ascontiguousarray(rows.reshape(-1), dtype=dtype)
            else:
                if len(values) != n:
                    raise ValueError("batch row count mismatch")
                rows = tuple(np.asarray(row, dtype=dtype) for row in values)
                if any(row.ndim != 1 for row in rows):
                    raise ValueError("ragged coordinates must be one-dimensional rows")
                offsets = [0]
                for row in rows:
                    offsets.append(offsets[-1] + len(row))
                response[key + "_offsets"] = offsets
                flat = np.concatenate(rows) if rows else np.empty(0, dtype=dtype)
            normalized.append(rows)
            flats.append(flat)
            if not binary:
                response[key + "_row_major"] = flat.tolist()
        self.validate(Batch(*normalized, weights))
        if binary:
            response["__binary__"] = b"".join(
                (
                    flats[0].data,
                    flats[1].data,
                    np.ascontiguousarray(weights, dtype="<f8").data,
                )
            )
        else:
            response["weights"] = weights.tolist()
        return response
