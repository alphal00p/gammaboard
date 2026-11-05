import numpy as np
import pytest

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
