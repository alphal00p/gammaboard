import pytest

from gammaboard_process.protocol import (
    fixed_domain_shape,
    instantiate_user_object,
    normalize_discrete_subspaces,
    require_homogeneous_offsets,
)


def test_fixed_domain_shapes():
    assert fixed_domain_shape({"rectangular": {"discrete_cardinalities": [], "continuous_dims": 3}}) == ([], 3)
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
                    {"domain": {"rectangular": {"discrete_cardinalities": [], "continuous_dims": 2}}},
                    {"domain": {"rectangular": {"discrete_cardinalities": [], "continuous_dims": 2}}},
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
                        {"domain": {"rectangular": {"discrete_cardinalities": [], "continuous_dims": 1}}},
                        {"domain": {"rectangular": {"discrete_cardinalities": [], "continuous_dims": 2}}},
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


def test_discrete_subspaces_decode_wire_shape():
    assert normalize_discrete_subspaces(
        {
            "subspaces": [
                {"fixed_dims": [{"dim": 0, "value": 2}]},
                {"fixed_dims": [{"dim": 1, "value": 3}]},
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


def test_sampler_snapshot_and_partial_pdf_contract():
    from unittest.mock import Mock
    from gammaboard_process.runners import _SamplerWorker

    worker = _SamplerWorker.__new__(_SamplerWorker)
    worker.sampler = Mock()
    worker.discrete_cardinalities = []
    worker.continuous_dims = 1
    worker.sampler_args = {"save_path": "must-not-be-injected"}
    worker.sampler.snapshot.return_value = None
    with pytest.raises(ValueError, match="must not be null"):
        worker.handle("snapshot", {})
    worker.sampler.snapshot.return_value = {}
    assert worker.handle("snapshot", {}) == {"snapshot": {}}
    worker.sampler.pdf.return_value = [1.0, None]
    assert worker.handle("pdf", {"nr_samples": 2, "xs_discrete_row_major": [],
                                "xs_continuous_row_major": [0.25, 0.75]}) == {"values": [1.0, None]}


def test_gammaloop_weights_feedback_and_owned_arrays(monkeypatch):
    import numpy as np
    from gammaboard_process import GammaLoopBatchResult, GammaLoopEvaluator
    from gammaboard_process import runners
    from gammaboard_process.protocol import PROTOCOL

    retained = []
    class Evaluator(GammaLoopEvaluator):
        def __init__(self, **kwargs): pass
        def eval_gammaloop(self, discrete, continuous, weights, *, require_training_values):
            assert all(a.flags.writeable and a.flags.owndata for a in (discrete, continuous, weights))
            retained.append(weights)
            values = continuous[:, 0] * weights
            return GammaLoopBatchResult({"sum": float(values.sum())},
                                        values if require_training_values else None)

    monkeypatch.setattr(runners, "ProtocolIo", lambda: None)
    worker = runners._EvaluatorWorker(Evaluator)
    worker.handle("initialize", {"protocol": PROTOCOL, "role": "evaluator", "output": {"kind": "gammaloop"},
                                 "domain": {"rectangular": {"discrete_cardinalities": [], "continuous_dims": 1}}, "args": {}})
    binary = np.array([0.25, 0.75, 2., 3.], dtype="<f8").tobytes()
    for required in [True, False]:
        result = worker.handle("eval_batch", {"nr_samples": 2, "require_training_values": required}, binary)
        assert result["gammaloop_state"] == {"sum": 2.75}
        if required:
            np.testing.assert_array_equal(np.frombuffer(result["__binary__"], dtype="<f8"), [0.5, 2.25])
        else:
            assert "__binary__" not in result
        retained[-1][:] = 0
    with pytest.raises(ValueError, match="input length"):
        worker.handle("eval_batch", {"nr_samples": 2, "require_training_values": True}, binary[:-1])
    with pytest.raises(TypeError, match="must implement eval"):
        worker.handle("initialize", {"protocol": PROTOCOL, "role": "evaluator", "output": {"kind":"values", "components":["value"]},
                                     "domain": {"rectangular": {"discrete_cardinalities": [], "continuous_dims": 1}}})


def test_sampler_requires_restoration_at_initialization(monkeypatch):
    from gammaboard_process import runners
    from gammaboard_process.protocol import PROTOCOL
    monkeypatch.setattr(runners, "ProtocolIo", lambda: None)
    class Sampler: pass
    worker = runners._SamplerWorker(Sampler)
    with pytest.raises(TypeError, match="from_snapshot"):
        worker.handle("initialize", {"protocol": PROTOCOL, "role": "sampler", "domain": {"rectangular": {"discrete_cardinalities": [], "continuous_dims": 1}}})


def test_optional_sampler_hooks_have_defaults_and_validate_normalization(monkeypatch):
    from gammaboard_process import runners

    monkeypatch.setattr(runners, "ProtocolIo", lambda: None)
    worker = runners._SamplerWorker(None)
    worker.sampler = type("Sampler", (), {})()
    assert worker.handle("global_pdf_norm", {}) == {"norm": 1.0}
    assert worker.handle("persisted_output", {}) == {"output": None}
    for invalid in (0.0, -1.0, float("nan"), float("inf")):
        worker.sampler.global_pdf_norm = lambda: invalid
        with pytest.raises(ValueError, match="finite and positive"):
            worker.handle("global_pdf_norm", {})


def test_materializer_and_transform_share_batch_type(monkeypatch):
    from gammaboard_process import Batch, Materializer, BatchTransform, runners
    from gammaboard_process.protocol import PROTOCOL

    class Materialize(Materializer):
        def __init__(self, **kwargs): pass
        def materialize_batch(self, latent_batch):
            return Batch([[]], [[latent_batch["x"]]], [2.0])

    class Transform(BatchTransform):
        def __init__(self, **kwargs): pass
        def transform_batch(self, discrete, continuous, weights):
            continuous *= 2
            weights *= 3
            return Batch(discrete, continuous, weights)

    monkeypatch.setattr(runners, "ProtocolIo", lambda: None)
    materializer = runners._MaterializerWorker(Materialize)
    transform = runners._BatchTransformWorker(Transform)
    for worker, role in [(materializer, "materializer"), (transform, "batch_transform")]:
        worker.handle("initialize", {"protocol": PROTOCOL, "role": role,
                                     "domain": {"rectangular": {"discrete_cardinalities": [], "continuous_dims": 1}}})
    batch = materializer.handle("materialize_batch", {"nr_samples": 1, "latent_batch": {"x": 0.25}})
    result = transform.handle("transform_batch", {"nr_samples": 1, **batch})
    assert result["xs_continuous_row_major"] == [0.5]
    assert result["weights"] == [6.0]


@pytest.mark.parametrize("value", [float("nan"), float("inf"), -float("inf")])
def test_invalid_evaluator_component_rejects_the_whole_response(value):
    from gammaboard_process.runners import _normalize_evaluator_values
    with pytest.raises(ValueError, match="sample 1, component 'imag'.*non-finite"):
        _normalize_evaluator_values([[1., 2.], [3., value]], 2, ["real", "imag"])
