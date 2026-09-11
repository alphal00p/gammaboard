"""Benchmark workload, measurement and cleanup tests; no deployment required."""
import contextlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import Mock

import benchmark_queue as bench


class BenchmarkTests(unittest.TestCase):
    def test_suite_preserves_compute_capacities_and_bursty_training(self):
        cases = bench.cases()
        self.assertEqual(len(cases), 16)
        self.assertEqual(len(set(cases)), 16)
        self.assertLess(len(cases) * (4 + bench.WARMUP_SECONDS) + 30, bench.MAX_SECONDS)
        for case in cases:
            evaluators, rate, regime = case
            card = tomllib.loads(bench.run_card(case))
            cost = card['evaluator']['timing']['per_sample_seconds']
            self.assertAlmostEqual(evaluators / cost, rate)
            if evaluators == 64 and rate == 1000:
                self.assertEqual(cost, .064)
            sampler = card['task_queue'][0]['sampler_aggregator']['config']
            self.assertNotIn('generation_timing', sampler)  # Fast between updates.
            if regime == 'training_burst':
                self.assertLessEqual(sampler['training_window_samples'], 100000)
                self.assertEqual(sampler['update_timing']['overhead_seconds'], .5)
            else:
                self.assertNotIn('training_window_samples', sampler)

    def test_measurements_use_counter_deltas_and_actual_elapsed_time(self):
        rows = [dict(time=1, samples=100, updates=1, update_seconds=.5, busy=.5),
                dict(time=3, samples=500, updates=3, update_seconds=1.5, busy=.75)]
        result = bench.summarize(rows)
        self.assertEqual(result, dict(rate=200, busy=.625, updates=2, stall_ms=500))

    def test_invalid_or_oversized_requests_fail_before_startup(self):
        for args in [('--duration', '300'), ('--duration', 'nan'), ('--rates', '0'),
                     ('--evaluators', '1', '1')]:
            result = subprocess.run([sys.executable, str(Path(bench.__file__)), *args],
                                    capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            self.assertIn('error:', result.stderr)

    def test_cleanup_uses_supervisor_and_preserves_data_on_failure(self):
        for exit_code in [0, 1]:
            with tempfile.TemporaryDirectory() as temp:
                deployment = bench.Deployment(Path(sys.executable), 30)
                deployment.directory = Path(temp) / 'deployment'
                deployment.directory.mkdir()
                deployment.process = Mock()
                deployment.process.poll.return_value = None
                deployment.process.wait.return_value = exit_code
                if exit_code:
                    with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(RuntimeError):
                        deployment.__exit__(None)
                    self.assertTrue(deployment.directory.exists())
                else:
                    deployment.__exit__(None)
                    self.assertFalse(deployment.directory.exists())
                deployment.process.terminate.assert_called_once()
                deployment.process.wait.assert_called_once_with(timeout=30)


if __name__ == '__main__':
    unittest.main()
