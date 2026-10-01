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


@pytest.mark.parametrize("offsets", [None, [0, 2, 4]])
def test_homogeneous_offsets_may_be_omitted(offsets):
    params = {} if offsets is None else {"offsets": offsets}
    require_homogeneous_offsets(params, "offsets", 2, 2, "evaluator")


@pytest.mark.parametrize("offsets", [[], [0, 2], [0, 2, 4, 6], [0, 3, 4], [1, 2, 4]])
def test_explicit_homogeneous_offsets_keep_shape_validation(offsets):
    with pytest.raises(ValueError, match="fixed width"):
        require_homogeneous_offsets({"offsets": offsets}, "offsets", 2, 2, "evaluator")


def test_sampler_binary_frame_handles_strided_endian_and_empty_arrays():
    import struct
    import numpy as np
    from gammaboard_process.runners import _encode_batch_binary

    discrete = np.array([0, 9, 1, 9, 0, 9], dtype=">i8")[::2, None]
    continuous = np.array([[0.1, 0.3, 0.5], [0.2, 0.4, 0.6]], dtype=">f8").T
    weights = np.array([3.0, 2.0, 1.0])[::-1]
    binary = _encode_batch_binary(discrete, continuous, weights)
    assert isinstance(binary, bytes)
    assert binary == struct.pack("<3q9d", 0, 1, 0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 1.0, 2.0, 3.0)
    # The returned frame owns its bytes; later callback mutations cannot alter it.
    discrete[:] = 1
    continuous[:] = 42
    weights[:] = 99
    assert binary == struct.pack("<3q9d", 0, 1, 0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 1.0, 2.0, 3.0)
    assert _encode_batch_binary(
        np.empty((3, 0), dtype=np.int64), np.empty((3, 0)), np.ones(3)
    ) == struct.pack("<3d", 1.0, 1.0, 1.0)


def test_evaluator_and_feedback_callback_arrays_are_independent_and_writable():
    import struct
    import numpy as np
    from unittest.mock import Mock
    from gammaboard_process.runners import _decode_eval_inputs, _SamplerWorker

    binary = struct.pack("<2q2d", 0, 1, 0.25, 0.75)
    first = _decode_eval_inputs(binary, 2, 1, 1)
    second = _decode_eval_inputs(binary, 2, 1, 1)
    for array in first:
        assert array.flags.writeable and array.flags.owndata
        array[:] = 0
    np.testing.assert_array_equal(second[0], [[0], [1]])
    np.testing.assert_array_equal(second[1], [[0.25], [0.75]])
    worker = _SamplerWorker.__new__(_SamplerWorker)
    worker.sampler = Mock()
    payload = struct.pack("<2d", 0.25, 0.75)
    worker.handle("feedback", {"nr_values": 2}, payload)
    retained = worker.sampler.feedback.call_args.args[0]
    retained[:] = -1
    worker.handle("feedback", {"nr_values": 2}, payload)
    current = worker.sampler.feedback.call_args.args[0]
    assert retained.flags.writeable and retained.flags.owndata
    np.testing.assert_array_equal(current, [0.25, 0.75])
