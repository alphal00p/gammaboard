from gammaboard_process.domain import RectangularDomain

"""Negative controls for the E2E oracle: plausible totals must not hide corruption."""
import unittest
import numpy as np
from gammaboard_process import Feedback
from recovery_sampler import GenerationStatus, RecoverySampler


class RecoveryOracleTests(unittest.TestCase):
    def sampler(self):
        return RecoverySampler(domain=RectangularDomain(tuple([]), 1), training=True)

    def test_feedback_rejects_loss_duplication_reordering_and_wrong_weights(self):
        correct = np.arange(1, 5) / 32768.0
        for bad in [correct[:-1], correct[[0, 0, 2, 3]], correct[::-1], correct / 2]:
            with self.subTest(values=bad):
                sampler = self.sampler()
                sampler.generate(4)
                with self.assertRaises(AssertionError):
                    sampler.feedback(Feedback(bad))
                # A rejected callback must not consume its draw.
                sampler.feedback(Feedback(correct))
                with self.assertRaises(AssertionError):
                    sampler.feedback(Feedback(correct))

    def test_restore_keeps_pending_draws_and_rejects_missing_ranges(self):
        sampler = self.sampler()
        sampler.generate(4)
        sampler.generate(3)
        sampler.feedback(Feedback(np.arange(1, 5) / 32768.0))
        snapshot = sampler.snapshot()
        restored = RecoverySampler.from_snapshot(
            snapshot=snapshot,
            init_args={"training": True},
            domain=RectangularDomain(tuple([]), 1),
        )
        restored.feedback(Feedback(np.arange(5, 8) / 32768.0))
        self.assertEqual(restored.accepted, 7)
        snapshot["pending"] = []
        with self.assertRaises(AssertionError):
            RecoverySampler.from_snapshot(
                snapshot=snapshot,
                init_args={"training": True},
                domain=RectangularDomain(tuple([]), 1),
            )

    def test_training_window_requires_all_feedback(self):
        sampler = self.sampler()
        sampler.window = 4
        sampler.generate(3)
        sampler.generate(3)  # Only the remaining single sample is generated.
        sampler.feedback(Feedback(np.arange(1, 4) / 32768.0))
        self.assertEqual(sampler.generate(3), GenerationStatus.WAITING)
        sampler.feedback(Feedback(np.array([4]) / 32768.0))
        self.assertEqual(sampler.generate(3).xs_continuous.shape, (3, 1))
