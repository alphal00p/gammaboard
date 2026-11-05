from dataclasses import dataclass
from typing import Any


@dataclass(slots=True)
class ObservableBatch:
    """Unweighted outputs, one per original input sample.

    ``values`` is a finite (N, 2) real/imag array. ``observables`` contains N
    GammaLoop-compatible ObservableSnapshotBundles, each with sample_count=1.
    Sum correlated fills within each bin before squaring. Include empty samples.
    GammaBoard alone applies outer weights, masks, training projections, and
    per-stratum reduction. No aggregated state or sampling metadata is accepted.
    """

    values: Any
    observables: list[dict]
