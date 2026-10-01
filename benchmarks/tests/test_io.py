"""Measurement validity: starvation, busy denominators and repeat aggregation."""

import unittest
from benchmarks.io import assess, summarize, thread_counts
from benchmarks.common import activity


class IoMeasurementTests(unittest.TestCase):
    def row(self, **changes):
        row = dict(
            role="evaluator",
            batch_size=4096,
            feedback=True,
            io_threads=1,
            repeat=0,
            samples_per_second=40960.0,
            batches_per_second=10.0,
            elapsed_seconds=2.0,
            batches=20,
            measured_io_busy=0.99,
            input_bytes_per_batch=1024,
            feedback_bytes_per_batch=64,
            queue_slots=128,
            insert_bundle=5,
            windows=[dict(empty_claims=0)],
        )
        return assess(dict(row, **changes))

    def test_low_busy_is_not_a_confirmed_ceiling(self):
        row = self.row(measured_io_busy=0.8)
        self.assertTrue(row["valid"])
        self.assertFalse(row["saturated"])
        self.assertIn("not above 90%", row["issues"][0])
        self.assertFalse(self.row(measured_io_busy=0.9)["saturated"])
        self.assertTrue(self.row(measured_io_busy=0.9001)["saturated"])
        self.assertTrue(self.row()["saturated"])

    def test_default_thread_sweep_fits_the_machine_without_shrinking_explicit_requests(self):
        self.assertEqual(thread_counts(None, 32), [1, 2, 4, 8])
        self.assertEqual(thread_counts(None, 8), [1, 2, 4])
        self.assertEqual(thread_counts(None, 3), [1])
        self.assertEqual(thread_counts([4, 1], 8), [1, 4])
        for requested, cores in [([8], 8), ([1, 1], 8), ([0], 8), (None, 2)]:
            with self.subTest(requested=requested, cores=cores), self.assertRaises(ValueError):
                thread_counts(requested, cores)

    def test_starvation_or_bad_counters_invalidates_even_busy_measurements(self):
        for changes in [
            dict(windows=[dict(empty_claims=1)]),
            dict(batches=7),
            dict(measured_io_busy=1.2),
            dict(measured_io_busy=float("nan")),
            dict(samples_per_second=float("inf")),
            dict(elapsed_seconds=0),
        ]:
            with self.subTest(changes=changes):
                row = self.row(**changes)
                self.assertFalse(row["valid"])
                self.assertFalse(row["saturated"])

    def test_invalid_repeats_cannot_bias_summary_or_hide_flags(self):
        a = self.row()
        b = self.row(repeat=1, samples_per_second=0)
        result = summarize([a, b])[0]
        self.assertEqual(result["samples_per_second"], 40960)
        self.assertEqual(result["flagged_trials"], 1)
        self.assertEqual(result["valid_trials"], 1)
        self.assertAlmostEqual(result["input_MiB_per_second"], 10 * 1024 / 1024**2)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            summarize([a, a])

    def test_frontier_busy_weights_worker_windows_and_counts_overlap_once(self):
        def row(name, t, compute, io):
            m = dict(
                epoch="e",
                runner_epoch="e",
                node_uuid=name,
                task_id="1",
                busy=dict(elapsed_seconds=t, compute_seconds=compute, io_seconds=io),
            )
            return dict(worker_id=name, metrics=m, runtime_metrics=m)

        measure = dict(
            snapshots=[
                dict(
                    evaluators=[row("a", 0, 0, 0), row("b", 0, 0, 0)], samplers=[row("s", 0, 0, 0)]
                ),
                dict(
                    evaluators=[row("a", 10, 10, 8), row("b", 5, 0, 4)],
                    samplers=[row("s", 10, 5, 8)],
                ),
            ]
        )
        rates = activity(measure)
        self.assertAlmostEqual(rates["evaluator_compute"], 100 * 10 / 15)
        self.assertEqual(rates["evaluator_io"], 80)
        measure["snapshots"][-1]["samplers"][0]["runtime_metrics"]["busy"]["io_seconds"] = 11
        with self.assertRaises(ValueError):
            activity(measure)


if __name__ == "__main__":
    unittest.main()
