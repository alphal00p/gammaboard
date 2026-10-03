"""The reported slowdown must compare equivalent accepted work, not raw rates."""

import unittest

from benchmarks import amortization as a


class AmortizationTests(unittest.TestCase):
    def test_joint_sweep_preserves_batch_compute_time_and_bounds_generation(self):
        for workers in [1, 16]:
            for batch in a.BATCH_SIZES:
                cost, generation = a.workload(batch, workers, 0.16384)
                self.assertAlmostEqual(batch * cost, 0.16384)
                self.assertLessEqual(generation * cost / workers, 4 * 0.16384)
                self.assertEqual(generation, min(a.GENERATION_SIZE, 4 * workers * batch))
                self.assertEqual(generation % batch, 0)
        self.assertEqual(a.workload(128, 16), (5e-6, a.GENERATION_SIZE))
        for target in [0, -1, float("nan"), float("inf"), 3]:
            with self.assertRaises(ValueError):
                a.workload(128, 16, target)

    def test_runtime_overhead_direction_and_negative_results(self):
        direct = dict(rate=125, mean_evaluate_batch_seconds=0.002)
        row = a.comparison(direct, dict(rate=100, valid=True, adequate=True))
        self.assertEqual(row["overhead_percent"], 25)
        self.assertEqual(row["evaluator_batch_ms"], 2)
        row = a.comparison(direct, dict(rate=250, valid=True, adequate=True))
        self.assertEqual(row["overhead_percent"], -50)
        with self.assertRaises(ValueError):
            a.comparison(direct, dict(rate=100, valid=False, adequate=True))

    def test_paired_repeats_are_not_pooled_as_independent_batches(self):
        records = [
            dict(
                workers=1,
                mode="materialized",
                batch=128,
                evaluator_batch_ms=1,
                overhead_percent=value,
                direct_samples_per_second=200,
                gammaboard_samples_per_second=100,
            )
            for value in [10, 20, 90]
        ]
        (row,) = a.summarize(records)
        self.assertEqual(row["trials"], 3)
        self.assertEqual(row["overhead_percent"], 20)
        self.assertEqual((row["overhead_low"], row["overhead_high"]), (10, 90))
