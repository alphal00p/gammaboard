"""Exercise generation/status/feedback framing without a subprocess."""
import unittest
from unittest.mock import Mock

import numpy as np
from gammaboard_process import GenerationStatus, SampleBatch
from gammaboard_process.runners import _SamplerWorker


class GenerationTests(unittest.TestCase):
    def setUp(self):
        self.worker = _SamplerWorker.__new__(_SamplerWorker)
        self.worker.sampler = Mock()
        self.worker.discrete_cardinalities = []
        self.worker.continuous_dims = 1

    def batch(self, n=3, remaining=9):
        return SampleBatch(np.empty((n, 0), dtype=np.int64), np.arange(n)[:, None],
                           np.ones(n), training_remaining=remaining)

    def test_draw_respects_current_limit_and_preserves_training_window(self):
        self.worker.sampler.generate.return_value = self.batch()
        for budget in [100, 3]:
            response = self.worker.handle('generate', {'max_samples': budget})
            self.assertEqual((response['kind'], response['nr_samples'], response['training_remaining']),
                             ('batch', 3, 9))
            self.assertEqual(np.frombuffer(response['__binary__'], dtype='<f8').tolist(),
                             [0., 1., 2., 1., 1., 1.])
            self.worker.sampler.generate.assert_called_with(budget)
        with self.assertRaisesRegex(ValueError, 'max_samples'):
            self.worker.handle('generate', {'max_samples': 2})

    def test_waiting_and_finished_have_no_binary_payload(self):
        for status in GenerationStatus:
            self.worker.sampler.generate.return_value = status
            self.assertEqual(self.worker.handle('generate', {'max_samples': 100}), {'kind': status.value})

    def test_rejects_invalid_sizes_and_training_window(self):
        for budget in [None, 0, -1, True, 1.5]:
            with self.assertRaisesRegex(ValueError, 'max_samples'):
                self.worker.handle('generate', {'max_samples': budget})
        with self.assertRaisesRegex(ValueError, 'max_samples'):
            self.worker.handle('generate', {})
        for batch in [self.batch(0), self.batch(3, 2), self.batch(3, True)]:
            self.worker.sampler.generate.return_value = batch
            with self.assertRaises(ValueError):
                self.worker.handle('generate', {'max_samples': 100})

    def test_feedback_preserves_signed_weighted_values_and_rejects_truncation(self):
        values = np.array([1.25, -3.5, 0.], dtype='<f8')
        self.assertEqual(self.worker.handle('feedback', {'nr_values': 3}, values.tobytes()), {'ok': True})
        np.testing.assert_array_equal(self.worker.sampler.feedback.call_args.args[0], values)
        with self.assertRaisesRegex(ValueError, 'feedback length'):
            self.worker.handle('feedback', {'nr_values': 4}, values.tobytes())
