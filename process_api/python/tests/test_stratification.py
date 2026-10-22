import numpy as np
import pytest

from gammaboard_process import GammaLoopBatchResult, StratificationContext
from gammaboard_process import runners
from gammaboard_process.protocol import PROTOCOL


def context(labels):
    return {"partition": {"kind": "voronoi", "centers": [[0., 0.], [2., 10.]], "scales": [1., 10.]},
            "labels": labels}


def test_scaled_mask_uses_raw_components_and_stable_ties():
    partition = StratificationContext(context([0, 0, 1]), 3)
    values = np.array([[0., 8.], [1., 5.], [2., 10.]])
    np.testing.assert_array_equal(partition.mask(values), [True, True, True])
    with pytest.raises(ValueError, match="finite unweighted"):
        partition.mask(np.array([[0., np.nan], [1., 5.], [2., 10.]]))
    with pytest.raises(ValueError, match="finite unweighted"):
        partition.mask(values[:, :1])
    with pytest.raises(ValueError, match="stratum labels"):
        StratificationContext(context([0, 2, 1]), 3)
    bad = context([0])
    bad["partition"]["scales"] = [1., 0.]
    with pytest.raises(ValueError, match="partition"):
        StratificationContext(bad, 1)


def test_hybrid_wire_keeps_feedback_full_weight_and_returns_raw_values(monkeypatch):
    class Evaluator:
        def __init__(self, **kwargs):
            pass

        def eval_gammaloop(self, discrete, continuous, weights, *, require_training_values, stratification):
            raw = np.column_stack([continuous[:, 0], np.zeros(len(weights))])
            masked = raw[:, 0] * weights * stratification.mask(raw)
            return GammaLoopBatchResult(
                state={"sum": float(masked.sum()), "count": len(weights)},
                training_values=masked if require_training_values else None, values=raw)

    monkeypatch.setattr(runners, "ProtocolIo", lambda: None)
    worker = runners._EvaluatorWorker(Evaluator)
    worker.handle("initialize", {"protocol": PROTOCOL, "role": "evaluator", "output": {"kind": "gammaloop"},
                                 "domain": {"rectangular": {"discrete_cardinalities": [], "continuous_dims": 1}}, "args": {}})
    partition = {"partition": {"kind": "voronoi", "centers": [[0., 0.], [1., 0.]]}, "labels": [0, 0, 1]}
    binary = np.array([.25, .75, .75, 4., 4., 4./3.], dtype="<f8").tobytes()
    for required in (False, True):
        result = worker.handle("eval_batch", {"nr_samples": 3, "require_training_values": required,
                                             "stratification": partition}, binary)
        assert result["gammaloop_state"] == {"sum": 2., "count": 3}
        expected = ([1., 0., 1.] if required else []) + [.25, 0., .75, 0., .75, 0.]
        np.testing.assert_array_equal(np.frombuffer(result["__binary__"], dtype="<f8"), expected)

    original = worker.evaluator.eval_gammaloop
    def bad_feedback(*args, **kwargs):
        result = original(*args, **kwargs)
        result.training_values[1] = 1.
        return result
    worker.evaluator.eval_gammaloop = bad_feedback
    with pytest.raises(ValueError, match="misses must return zero"):
        worker.handle("eval_batch", {"nr_samples": 3, "require_training_values": True,
                                     "stratification": partition}, binary)
