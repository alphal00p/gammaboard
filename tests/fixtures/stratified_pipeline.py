"""Nontrivial coordinate/Jacobian pipeline with an original discrete branch.

Materializer: z=2u, dz/du=2; transform: x=3z+1, dx/dz=3;
evaluator: f(d,x)=(d+1)*x. The exact sum over d=0,1 and x in [1,7] is 72.
"""
import sys
import numpy as np
from gammaboard_process import Batch, run_materializer, run_batch_transform, run_evaluator


class Materializer:
    def __init__(self, *, discrete_cardinalities, continuous_dims):
        assert discrete_cardinalities == [2, 2] and continuous_dims == 1

    def materialize_batch(self, latent):
        payload = latent["payload"]
        assert payload["kind"] == "indexed_batch"
        discrete = np.asarray(payload["discrete_signatures"], dtype=np.int64)[payload["discrete_map"]]
        u = np.asarray(payload["continuous_values"]).reshape(-1, 1)
        assert np.all((u >= 0) & (u <= 1))
        return Batch(discrete, 2 * u, 2 * np.asarray(payload["weights"]))


class Transform:
    def __init__(self, *, discrete_cardinalities, continuous_dims):
        # Only the original physical branch remains after the materializer.
        assert discrete_cardinalities == [2] and continuous_dims == 1

    def transform_batch(self, xs_discrete, xs_continuous, weights):
        assert xs_discrete.shape[1] == 1
        assert np.all((xs_continuous >= 0) & (xs_continuous <= 2))
        assert np.allclose(weights, 8.0)  # 1/(p_stratum*p_branch) * 2
        return Batch(xs_discrete, 3 * xs_continuous + 1, 3 * weights)


class Evaluator:
    def __init__(self, *, discrete_cardinalities, continuous_dims):
        assert discrete_cardinalities == [2] and continuous_dims == 1

    def eval(self, xs_discrete, xs_continuous):
        assert xs_discrete.shape[1] == 1
        assert np.all((xs_continuous >= 1) & (xs_continuous <= 7))
        return (xs_discrete[:, 0] + 1) * xs_continuous[:, 0]


if __name__ == "__main__":
    runner, adapter = {"materializer": (run_materializer, Materializer),
                       "transform": (run_batch_transform, Transform),
                       "evaluator": (run_evaluator, Evaluator)}[sys.argv[1]]
    runner(adapter)
