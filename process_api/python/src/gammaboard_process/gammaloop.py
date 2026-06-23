from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Optional


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
    """Return value from :meth:`~gammaboard_process.Evaluator.eval_gammaloop`.

    ``state`` must be a JSON-serializable dict matching the
    ``GammaLoopAccumulatorState`` schema expected by GammaBoard.  It must
    contain at minimum a ``bundle`` key with an
    :class:`ObservableSnapshotBundle`.  It may also contain an ``estimate``
    key (a ``VectorAccumulatorState`` JSON object with ``"real"`` and
    ``"imag"`` components) and a ``diagnostics`` key.

    When wrapping ``pygloop``, you can obtain a ready-to-use dict by calling
    the GammaLoop accumulator state's serialization method and passing its
    output directly as ``state``.

    ``training_values`` is an optional list of one scalar per sample.  These
    are forwarded to the sampler as importance-sampling feedback.  When
    omitted, no training update is sent for this batch.

    Example::

        from gammaboard_process import GammaLoopBatchResult

        def eval_gammaloop(self, xs_discrete, xs_continuous):
            state_dict = my_gammaloop_run.eval_batch(xs_continuous)
            training = state_dict.pop("training_values", None)
            return GammaLoopBatchResult(state=state_dict, training_values=training)
    """

    state: dict[str, Any]
    training_values: list[float] | None = None
