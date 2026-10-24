from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import numpy as np
from .domain import Domain


class StratificationContext:
    """Mask for native pooled observables; never changes their full weights.

    Classify the unweighted real/imaginary integrand, then apply the returned
    mask to all observable contributions from the same original draw. Count
    every draw, including misses. Return the raw values in GammaLoopBatchResult.
    """

    def __init__(self, payload: dict, nr_samples: int):
        partition = payload["partition"]
        if partition["kind"] != "voronoi":
            raise ValueError("unsupported stratification partition")
        prefixes = Domain.from_dict(partition["domain"]).stratum_prefixes()
        centers = {
            tuple(int(x) for x in key.split(",")): value
            for key, value in partition["centers"].items()
        }
        if set(centers) != set(prefixes):
            raise ValueError("Voronoi centers must cover the complete stratum prefixes")
        self.prefixes = prefixes
        self.centers = np.asarray([centers[p] for p in prefixes], dtype=np.float64)
        self.scales = np.asarray(partition.get("scales", [1.0, 1.0]), dtype=np.float64)
        self.labels = np.asarray(payload["labels"])
        if (
            self.centers.ndim != 2
            or self.centers.shape[1] != 2
            or len(self.centers) == 0
            or not np.isfinite(self.centers).all()
            or self.scales.shape != (2,)
            or not np.isfinite(self.scales).all()
            or (self.scales <= 0).any()
        ):
            raise ValueError("invalid real/imaginary Voronoi partition")
        if (
            self.labels.shape != (nr_samples,)
            or self.labels.dtype.kind not in "iu"
            or (self.labels < 0).any()
            or (self.labels >= len(self.centers)).any()
        ):
            raise ValueError("invalid stratum labels")

    def mask(self, values: np.ndarray) -> np.ndarray:
        values = np.asarray(values, dtype=np.float64)
        if values.shape != (len(self.labels), 2) or not np.isfinite(values).all():
            raise ValueError(
                "stratification needs finite unweighted (N,2) real/imag values"
            )
        with np.errstate(over="ignore", invalid="ignore"):
            delta = (values[:, None, :] - self.centers[None, :, :]) / self.scales
            distances = np.hypot.reduce(delta, axis=2)
        if not np.isfinite(distances).all():
            raise ValueError("Voronoi distance overflow")
        return np.argmin(distances, axis=1) == self.labels


class HistogramBinSnapshot(dict):
    """Dict shape for a single histogram bin.

    Fields (all optional at the dict level — absent optional fields mean null):

    - ``x_min`` / ``x_max``: bin edges for continuous histograms
    - ``bin_id``: integer bin ID for discrete histograms
    - ``label``: human-readable bin label
    - ``entry_count``: number of entries in this bin
    - ``sum_weights``: accumulated weight sum
    - ``sum_weights_squared``: accumulated squared weight sum (for error estimation)
    - ``mitigated_fill_count``: count of misbinning-mitigated fills
    """


class HistogramStatisticsSnapshot(dict):
    """Dict shape for histogram-level statistics.

    Fields:

    - ``in_range_entry_count``: entries that fell inside the histogram range
    - ``nan_value_count``: entries with NaN observable value
    - ``mitigated_pair_count``: pairs whose weights were mitigated
    """


class HistogramSnapshot(dict):
    """Dict shape for a single named histogram.

    Required fields:

    - ``kind``: ``"continuous"`` or ``"discrete"``
    - ``title``: observable title
    - ``type_description``: format tag (e.g. ``"HwU"``)
    - ``phase``: ``"real"`` or ``"imag"``
    - ``value_transform``: ``"identity"`` or ``"log10"``
    - ``supports_misbinning_mitigation``: bool
    - ``sample_count``: total samples accumulated
    - ``log_x_axis`` / ``log_y_axis``: display hints
    - ``bins``: list of :class:`HistogramBinSnapshot`
    - ``underflow_bin`` / ``overflow_bin``: :class:`HistogramBinSnapshot`
    - ``statistics``: :class:`HistogramStatisticsSnapshot`

    Optional fields (set to ``null`` when absent):

    - ``x_min`` / ``x_max``: overall range for continuous histograms
    - ``discrete_min_bin_id``: lowest bin ID for discrete histograms
    - ``discrete_ordering``: ``"ascending_bin_id"``, ``"value_descending"``,
      or ``"abs_value_descending"``
    """


class ObservableSnapshotBundle(dict):
    """Dict shape for a bundle of named histograms.

    Single field:

    - ``histograms``: mapping from observable name to :class:`HistogramSnapshot`

    Example::

        bundle: ObservableSnapshotBundle = {
            "histograms": {
                "pt": { "kind": "continuous", "title": "pT", ... },
                "eta": { "kind": "continuous", "title": "η", ... },
            }
        }
    """


@dataclass(slots=True)
class GammaLoopBatchResult:
    """Return value from :meth:`~gammaboard_process.GammaLoopEvaluator.eval_gammaloop`.

    ``state`` must be a JSON-serializable dict matching the
    ``GammaLoopAccumulatorState`` schema expected by GammaBoard.  It must
    contain at minimum a ``bundle`` key with an
    :class:`ObservableSnapshotBundle`.  It may also contain an ``estimate``
    key (a ``VectorAccumulatorState`` JSON object with ``"real"`` and
    ``"imag"`` components) and a ``diagnostics`` key.

    When wrapping ``pygloop``, you can obtain a ready-to-use dict by calling
    the GammaLoop accumulator state's serialization method and passing its
    output directly as ``state``.

    ``training_values`` contains one unmasked weighted scalar per sample when
    ``require_training_values`` is true, and must be None otherwise.
    Apply the supplied weights once to both the state and feedback. Every component
    and the projection must count every sample, including finite zeros. Final values,
    required moments, and feedback must be finite; an invalid result rejects the batch.

    ``values`` must be the finite unmasked ``(N,2)`` real/imag array when a
    ``StratificationContext`` was supplied, and None otherwise. Histograms and
    the native estimate use the context's mask with full weights, counting every
    original draw. The host applies its frozen stratum probabilities only to
    the separate primary integral estimator. Never apply them to ``state``.

    See examples/python_gammaloop_observable for a complete producer.
    """

    state: dict[str, Any]
    training_values: list[float] | None = None
    # Raw unweighted (N,2) real/imag values, required exactly when stratifying.
    values: Any | None = None
