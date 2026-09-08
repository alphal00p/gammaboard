import pytest

from gammaboard_process.protocol import (
    fixed_domain_shape,
    instantiate_user_object,
    normalize_discrete_subspaces,
    require_homogeneous_offsets,
)


def test_fixed_domain_shapes():
    assert fixed_domain_shape({"continuous": {"dims": 3}}) == ([], 3)
    assert fixed_domain_shape(
        {
            "rectangular": {
                "discrete_cardinalities": [2, 4],
                "continuous_dims": 5,
            }
        }
    ) == ([2, 4], 5)
    assert fixed_domain_shape(
        {
            "discrete": {
                "branches": [
                    {"domain": {"continuous": {"dims": 2}}},
                    {"domain": {"continuous": {"dims": 2}}},
                ]
            }
        }
    ) == ([2], 2)


def test_inhomogeneous_domain_and_offsets_are_rejected():
    with pytest.raises(ValueError, match="inhomogeneous"):
        fixed_domain_shape(
            {
                "discrete": {
                    "branches": [
                        {"domain": {"continuous": {"dims": 1}}},
                        {"domain": {"continuous": {"dims": 2}}},
                    ]
                }
            }
        )
    with pytest.raises(ValueError, match="fixed width 2"):
        require_homogeneous_offsets({"offsets": [0, 1, 4]}, "offsets", 2, 2, "evaluator")


def test_user_object_initialization_and_restore():
    class Worker:
        def __init__(self, *, discrete_cardinalities, continuous_dims, evaluator_metadata, scale):
            self.values = (discrete_cardinalities, continuous_dims, evaluator_metadata, scale)

        @classmethod
        def from_snapshot(
            cls,
            *,
            snapshot,
            discrete_cardinalities,
            continuous_dims,
            init_args,
            evaluator_metadata,
        ):
            return (snapshot, discrete_cardinalities, continuous_dims, init_args, evaluator_metadata)

    fresh = instantiate_user_object(
        Worker,
        discrete_cardinalities=[3],
        continuous_dims=2,
        init_args={"scale": 4},
        evaluator_metadata={"process": "demo"},
    )
    assert fresh.values == ([3], 2, {"process": "demo"}, 4)

    restored = instantiate_user_object(
        Worker,
        discrete_cardinalities=[3],
        continuous_dims=2,
        init_args={"scale": 4},
        snapshot={"step": 7},
        evaluator_metadata={"process": "demo"},
    )
    assert restored == ({"step": 7}, [3], 2, {"scale": 4}, {"process": "demo"})

    with pytest.raises(TypeError, match="reserved keys"):
        instantiate_user_object(
            Worker,
            discrete_cardinalities=[],
            continuous_dims=1,
            init_args={"continuous_dims": 9},
        )


def test_discrete_subspaces_accept_wire_and_mapping_shapes():
    assert normalize_discrete_subspaces(
        {
            "subspaces": [
                {"fixed_dims": [{"dim": 0, "value": 2}]},
                {"fixed_dims": {"1": 3}},
            ]
        }
    ) == [{0: 2}, {1: 3}]
