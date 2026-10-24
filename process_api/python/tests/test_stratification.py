import numpy as np
import pytest

from gammaboard_process import GammaLoopBatchResult, StratificationContext
from gammaboard_process import runners
from gammaboard_process.protocol import PROTOCOL


def test_ragged_domain_feedback_and_partial_masses_share_one_sampler_contract(
    monkeypatch,
):
    from gammaboard_process import (
        Domain,
        RectangularDomain,
        DiscreteDomain,
        DomainBranch,
        DiscreteAxisRole,
        SampleBatch,
    )
    from gammaboard_process.batch_codec import BatchCodec

    leaf = RectangularDomain((), 1)
    domain = DiscreteDomain(
        (
            DomainBranch(0, leaf),
            DomainBranch(
                1,
                DiscreteDomain(
                    tuple(DomainBranch(i, leaf) for i in (0, 1, 3)),
                    DiscreteAxisRole.STRATUM,
                ),
            ),
        ),
        DiscreteAxisRole.STRATUM,
    )
    prefixes = ((0,), (1, 0), (1, 1), (1, 3))
    assert Domain.from_dict(domain.to_dict()) == domain
    assert domain.stratum_prefixes() == prefixes
    with pytest.raises(ValueError, match="inhomogeneous|contiguous"):
        domain.require_rectangular_shape()
    with pytest.raises(ValueError, match="left-bound"):
        DiscreteDomain((DomainBranch(0, domain),)).validate()

    class Aware:
        def __init__(self, *, domain, evaluator_metadata):
            assert domain.stratum_prefixes() == prefixes

        @classmethod
        def from_snapshot(cls, **kwargs):
            raise AssertionError("fresh initialization expected")

        def generate(self, max_samples):
            return SampleBatch(
                tuple(np.array(p) for p in prefixes),
                np.full((4, 1), 0.5),
                np.array([2.0, 6.0, 6.0, 6.0]),
                4,
            )

        def feedback(self, feedback):
            np.testing.assert_array_equal(feedback.values, [0.0, 6.0, 0.0, 6.0])
            np.testing.assert_array_equal(
                feedback.unmasked_values, [2.0, 6.0, 6.0, 6.0]
            )
            assert feedback.strata.requested == list(prefixes)
            assert feedback.strata.actual == [prefixes[i] for i in (1, 1, 0, 3)]

        def discrete_mass(self, subspaces):
            known = {(0,): 0.5, (1, 0): 1 / 6, (1, 1): 1 / 6, (1, 3): 1 / 6}
            return [
                known.get(tuple(s[i] for i in range(len(s))))
                if set(s) == set(range(len(s)))
                else None
                for s in subspaces
            ]

    monkeypatch.setattr(runners, "ProtocolIo", lambda: None)
    worker = runners._SamplerWorker(Aware)
    worker.handle(
        "initialize",
        {
            "protocol": PROTOCOL,
            "role": "sampler",
            "domain": domain.to_dict(),
            "initialization": {"kind": "fresh"},
        },
    )
    draw = worker.handle("generate", {"max_samples": 4})
    assert draw["xs_discrete_offsets"] == [0, 1, 3, 5, 7]
    decoded = BatchCodec(domain).decode(draw, draw["__binary__"], weighted=True)
    assert [tuple(row) for row in decoded.xs_discrete] == list(prefixes)
    wire = (
        np.array([2.0, 6.0, 6.0, 6.0], dtype="<f8").tobytes()
        + np.array([0, 1, 2, 3, 1, 1, 0, 3], dtype="<u8").tobytes()
    )
    worker.handle("feedback", {"nr_values": 4, "stratum_prefixes": prefixes}, wire)
    queries = [
        {"fixed_dims": [{"dim": i, "value": v} for i, v in enumerate(p)]}
        for p in prefixes
    ]
    queries.append({"fixed_dims": [{"dim": 3, "value": 0}]})
    assert worker.handle("discrete_mass", {"subspaces": queries}) == {
        "values": [0.5, 1 / 6, 1 / 6, 1 / 6, None]
    }
    worker.sampler.discrete_mass = lambda _: [float("nan")] * len(queries)
    with pytest.raises(ValueError, match="probability"):
        worker.handle("discrete_mass", {"subspaces": queries})


def context(labels):
    return {
        "partition": {
            "kind": "voronoi",
            "domain": {
                "rectangular": {
                    "strata_dims": 1,
                    "discrete_cardinalities": [2],
                    "continuous_dims": 0,
                }
            },
            "centers": {"0": [0.0, 0.0], "1": [2.0, 10.0]},
            "scales": [1.0, 10.0],
        },
        "labels": labels,
    }


def test_scaled_mask_uses_raw_components_and_stable_ties():
    partition = StratificationContext(context([0, 0, 1]), 3)
    values = np.array([[0.0, 8.0], [1.0, 5.0], [2.0, 10.0]])
    np.testing.assert_array_equal(partition.mask(values), [True, True, True])
    with pytest.raises(ValueError, match="finite unweighted"):
        partition.mask(np.array([[0.0, np.nan], [1.0, 5.0], [2.0, 10.0]]))
    with pytest.raises(ValueError, match="finite unweighted"):
        partition.mask(values[:, :1])
    with pytest.raises(ValueError, match="stratum labels"):
        StratificationContext(context([0, 2, 1]), 3)
    bad = context([0])
    bad["partition"]["scales"] = [1.0, 0.0]
    with pytest.raises(ValueError, match="partition"):
        StratificationContext(bad, 1)


def test_hybrid_wire_keeps_feedback_full_weight_and_returns_raw_values(monkeypatch):
    class Evaluator:
        def __init__(self, **kwargs):
            pass

        def eval_gammaloop(
            self,
            discrete,
            continuous,
            weights,
            *,
            require_training_values,
            stratification,
        ):
            raw = np.column_stack([continuous[:, 0], np.zeros(len(weights))])
            masked = raw[:, 0] * weights * stratification.mask(raw)
            return GammaLoopBatchResult(
                state={"sum": float(masked.sum()), "count": len(weights)},
                training_values=raw[:, 0] * weights
                if require_training_values
                else None,
                values=raw,
            )

    monkeypatch.setattr(runners, "ProtocolIo", lambda: None)
    worker = runners._EvaluatorWorker(Evaluator)
    worker.handle(
        "initialize",
        {
            "protocol": PROTOCOL,
            "role": "evaluator",
            "output": {"kind": "gammaloop"},
            "domain": {
                "rectangular": {"discrete_cardinalities": [], "continuous_dims": 1}
            },
            "args": {},
        },
    )
    partition = {
        "partition": {
            "kind": "voronoi",
            "domain": {
                "rectangular": {
                    "strata_dims": 1,
                    "discrete_cardinalities": [2],
                    "continuous_dims": 0,
                }
            },
            "centers": {"0": [0.0, 0.0], "1": [1.0, 0.0]},
        },
        "labels": [0, 0, 1],
    }
    binary = np.array([0.25, 0.75, 0.75, 4.0, 4.0, 4.0 / 3.0], dtype="<f8").tobytes()
    for required in (False, True):
        result = worker.handle(
            "eval_batch",
            {
                "nr_samples": 3,
                "require_training_values": required,
                "stratification": partition,
            },
            binary,
        )
        assert result["gammaloop_state"] == {"sum": 2.0, "count": 3}
        expected = ([1.0, 3.0, 1.0] if required else []) + [
            0.25,
            0.0,
            0.75,
            0.0,
            0.75,
            0.0,
        ]
        np.testing.assert_array_equal(
            np.frombuffer(result["__binary__"], dtype="<f8"), expected
        )

    original = worker.evaluator.eval_gammaloop

    def bad_feedback(*args, **kwargs):
        result = original(*args, **kwargs)
        result.training_values[1] = float("nan")
        return result

    worker.evaluator.eval_gammaloop = bad_feedback
    with pytest.raises(ValueError, match="non-finite"):
        worker.handle(
            "eval_batch",
            {
                "nr_samples": 3,
                "require_training_values": True,
                "stratification": partition,
            },
            binary,
        )
