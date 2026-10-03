"""The adapter must honor GammaBoard's weighted feedback for complete generation draws."""
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

import numpy as np
import torch

from madnis_sampler import MadnisSampler
from gammaboard_process import GenerationStatus


class TrainingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        torch.set_num_threads(1)

    def test_multiple_draws_wait_for_completed_training_window(self):
        sampler = MadnisSampler(continuous_dims=2, discrete_cardinalities=[], training_steps=2,
                               training_batch_size=16, use_gpu=False,
                               flow_config=dict(layers=2, units=8, bins=4))
        for step in range(2):
            batches = [sampler.generate(n) for n in [5, 11]]
            weights = np.concatenate([batch.weights for batch in batches])
            self.assertEqual(sampler._training_samples_remaining(), 0)
            self.assertEqual(sampler.generate(16), GenerationStatus.WAITING)
            sampler.feedback(weights[:5])
            self.assertEqual(sampler.step, step)
            self.assertEqual(sampler._training_samples_remaining(), 0)
            sampler.feedback(weights[5:])
            self.assertEqual(sampler.get_diagnostics()['training_updates'], step + 1)
        self.assertIsNone(sampler._training_samples_remaining())
        batch = sampler.generate(8)
        self.assertIsNone(batch.training_remaining)
        self.assertEqual(sampler.step, 2)
        self.assertEqual(sampler.pending_weights, [])
        self.assertEqual(sampler.pending_training_samples, [])

    def test_proposal_weight_is_removed_once_even_with_different_chunk_partitions(self):
        sampler = MadnisSampler.__new__(MadnisSampler)
        sampler.device = torch.device('cpu')
        sampler.step = 0
        sampler.trained_samples = 3
        sampler.pending_weights = [np.array([2.]), np.array([1.5, 1.25])]
        sampler.pending_training_samples = [torch.zeros(2, 2), torch.zeros(1, 2)]
        sampler.pending_training_probs = [torch.tensor([.5, 2., 4.], dtype=torch.float64)]
        sampler.madnis = SimpleNamespace(_optimization_step=Mock(return_value=(0.,)), scheduler=None, step=0)
        sampler._train_step()
        samples = sampler.madnis._optimization_step.call_args.args[0]
        torch.testing.assert_close(samples.func_vals, torch.tensor([1., 3., 5.], dtype=torch.float64))
        self.assertEqual(sampler.step, 1)
        self.assertEqual(sampler.pending_weights, [])

    def test_excess_feedback_is_rejected_without_mutating_the_pending_window(self):
        sampler = MadnisSampler(continuous_dims=2, discrete_cardinalities=[], training_steps=1,
                               training_batch_size=16, use_gpu=False,
                               flow_config=dict(layers=2, units=8, bins=4))
        sampler.generate(5)
        with self.assertRaisesRegex(ValueError, 'more training feedback'):
            sampler.feedback(np.ones(6))
        self.assertEqual(sampler.pending_weights, [])


if __name__ == '__main__':
    unittest.main()
