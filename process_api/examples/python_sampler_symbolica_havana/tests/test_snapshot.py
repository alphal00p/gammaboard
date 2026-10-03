import unittest

from demo_sampler import SymbolicaHavanaSampler
from gammaboard_process import GenerationStatus


class SnapshotTests(unittest.TestCase):
    def test_partial_window_restores_pending_draws_and_final_partial_update(self):
        args = dict(seed=42, bins=4, samples_for_update=16,
                    stop_training_after_n_samples=19)
        shape = dict(discrete_cardinalities=[2], continuous_dims=2)
        sampler = SymbolicaHavanaSampler(**shape, **args)
        draws = [sampler.generate(5) for _ in range(4)]
        self.assertEqual([len(b.weights) for b in draws], [5, 5, 5, 1])
        self.assertEqual(sampler.generate(5), GenerationStatus.WAITING)
        sampler.feedback([w * (i + 1) for i, w in enumerate(draws[0].weights)])
        restored = SymbolicaHavanaSampler.from_snapshot(snapshot=sampler.snapshot(), init_args=args, **shape)
        for draw in draws[1:]:
            values = [w * (i + 1) for i, w in enumerate(draw.weights)]
            sampler.feedback(values)
            restored.feedback(values)
        self.assertEqual(restored.snapshot(), sampler.snapshot())
        last = sampler.generate(5)
        self.assertEqual(last, restored.generate(5))
        self.assertEqual(len(last.weights), 3)
        self.assertEqual(sampler.generate(5), GenerationStatus.FINISHED)
        for worker in [sampler, restored]:
            worker.feedback(last.weights)
        self.assertEqual(restored.snapshot(), sampler.snapshot())
        self.assertEqual(restored.pending_training_sample_count(), 0)
