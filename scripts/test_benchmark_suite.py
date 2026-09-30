"""Scientific validity and resource-budget checks, without launching a deployment."""
import copy
import contextlib
import io
import json
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import Mock, patch
import subprocess
import benchmark as bench

class SuiteTests(unittest.TestCase):
    def setUp(self):
        self.suite = tomllib.loads((bench.ROOT/'resources/templates/benchmarks/scaling.toml').read_text())

    def test_work_is_independent_of_concurrency(self):
        cases = bench.validate_suite(self.suite)
        self.assertEqual(len(cases),48)
        card = tomllib.loads(bench.run_card(500,256))
        self.assertEqual(card['evaluator']['cpu_iterations_per_sample'],500)
        self.assertNotIn('timing',card['evaluator'])
        self.assertEqual(card['sampler_aggregator_runner_params']['queue']['fixed_batch_size'],256)

    def test_saved_calibration_is_shared_across_integer_and_fractional_suite_labels(self):
        with tempfile.TemporaryDirectory() as tmp:
            saved = Path(tmp)/'calibration.json'
            calibration = dict(cpu_iterations_per_sample=10, measured_seconds_per_sample=.000005)
            bench.write_json(saved, {'5': calibration})
            session = Mock()
            self.assertEqual(bench.calibrate_workloads(session, [5.0], saved)['5.0'], calibration)
            session.cli.assert_not_called()
            with self.assertRaisesRegex(ValueError, 'missing or invalid'):
                bench.calibrate_workloads(session, [.5], saved)
            bench.write_json(saved, {'5': calibration, '5.0': calibration})
            with self.assertRaisesRegex(ValueError, 'duplicate'):
                bench.calibrate_workloads(session, [5], saved)

    def test_invalid_or_over_budget_suite_is_rejected(self):
        for key,value in [('workers',[1,100]),('duration_seconds',float('nan')),('repetitions',0),
                          ('budget_seconds',1801),('batch_sizes',[1]),('eval_us',[1,1]),('duration_seconds',300),
                          ('duration_seconds', True), ('telemetry_interval_ms', '250'),
                          ('duration_seconds', 1), ('warmup_seconds', .1), ('typo', 1),
                          ('bulk_sample_generation', 'true'), ('generation_batch_size', 1),
                          ('repetitions', 10**9)]:
            with self.subTest(key=key,value=value):
                suite = copy.deepcopy(self.suite); suite[key]=value
                with self.assertRaises(ValueError): bench.validate_suite(suite)

    def test_invalid_observations_are_not_plotted_as_zero(self):
        rows = [dict(valid=False,batch_size=256,workers=1,eval_us=10,rate=None),
                dict(valid=True,batch_size=256,workers=1,eval_us=10,rate=12)]
        self.assertEqual(bench.grouped(rows,'rate'),{(256,1,10):[12]})

    def test_all_presets_fit_their_budget(self):
        for path in (bench.ROOT/'resources/templates/benchmarks').glob('*.toml'):
            if path.name == 'frontier.toml':
                continue
            suite = tomllib.loads(path.read_text())
            bench.validate_suite(suite)
            self.assertLessEqual(bench.estimated_seconds(suite), suite['budget_seconds'])

    def test_generation_size_does_not_change_evaluator_work(self):
        suite = dict(self.suite, generation_batch_size=4096)
        bench.validate_suite(suite)
        card = tomllib.loads(bench.run_card(500, 256, generation_batch_size=4096))
        queue = card['sampler_aggregator_runner_params']['queue']
        self.assertEqual(queue['fixed_batch_size'], 256)
        self.assertEqual(queue['max_batch_size'], 256)
        self.assertEqual(card['evaluator']['cpu_iterations_per_sample'], 500)
        self.assertEqual(card['task_queue'][0]['sampler_aggregator']['config']['generation_batch_size'], 4096)


class MeasurementTests(unittest.TestCase):
    def record(self, rate=100, direct_rate=200, valid=True, issues=None):
        return bench.measurement_record(
            dict(valid=valid, issues=issues or [], samples_per_second=rate),
            {1: dict(samples_per_second=direct_rate)}, eval_us=10, workers=1,
            batch_size=64, repeat=0, cpu_iterations_per_sample=10)

    def test_invalid_baselines_and_intervals_never_produce_ratios(self):
        for rate in (0, -1, None, float('inf'), float('nan')):
            for row in (self.record(rate=rate), self.record(direct_rate=rate)):
                self.assertFalse(row['valid'])
                self.assertIsNone(row['retained_efficiency'])
                json.dumps(row, allow_nan=False)
        self.assertFalse(self.record(valid=False)['valid'])
        self.assertFalse(self.record(issues=['stale evaluator telemetry'])['valid'])
        self.assertEqual(self.record()['retained_efficiency'], .5)

    def test_partial_coverage_distinguishes_invalid_and_missing_trials(self):
        suite = dict(batch_sizes=[64], workers=[1, 4], eval_us=[10], repetitions=3)
        result = bench.summarize([self.record(), dict(self.record(valid=False), repeat=1)], suite)
        self.assertFalse(result['complete'])
        self.assertEqual(result['planned_cases'], 6)
        first, missing = result['rows']
        self.assertEqual((first['valid_trials'], first['invalid_trials'], first['missing_trials']), (1, 1, 1))
        self.assertEqual(first['rate']['median'], 100)
        self.assertEqual(missing['missing_trials'], 3)
        self.assertIsNone(missing['rate'])

    def test_duplicate_trials_cannot_masquerade_as_repetitions(self):
        suite = dict(batch_sizes=[64], workers=[1], eval_us=[10], repetitions=2)
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            bench.summarize([self.record(), self.record()], suite)

    def test_startup_failure_can_be_summarized_without_results(self):
        with tempfile.TemporaryDirectory() as tmp:
            suite = dict(batch_sizes=[64], workers=[1], eval_us=[10], repetitions=3)
            bench.write_json(Path(tmp)/'manifest.json', dict(suite=suite, status='incomplete'))
            with contextlib.redirect_stdout(io.StringIO()):
                result = bench.summary(Path(tmp))
            self.assertEqual(result['completed_cases'], 0)
            self.assertEqual(result['rows'][0]['missing_trials'], 3)

    def test_revision_comparison_rejects_changed_work_and_disjoint_cases(self):
        with tempfile.TemporaryDirectory() as tmp:
            before, after = Path(tmp)/'before', Path(tmp)/'after'
            before.mkdir()
            after.mkdir()
            first = self.record()
            (before/'results.jsonl').write_text(json.dumps(first)+'\n')
            for changed, message in [
                    (dict(first, cpu_iterations_per_sample=20), 'calibrated work differs'),
                    (dict(first, eval_us=100), 'no overlapping valid cases')]:
                (after/'results.jsonl').write_text(json.dumps(changed)+'\n')
                with self.assertRaisesRegex(ValueError, message):
                    bench.compare(before, after)
            (before/'results.jsonl').write_text(
                json.dumps(first)+'\n'+json.dumps(dict(first, repeat=1, cpu_iterations_per_sample=20))+'\n')
            with self.assertRaisesRegex(ValueError, 'work changes between repetitions'):
                bench.compare(before, after)

    def test_case_rotates_baselines_and_pauses_before_direct_measurement(self):
        for repeat, expected_order in enumerate([
                ['gammaboard', 'direct-1', 'direct-4'],
                ['direct-1', 'gammaboard', 'direct-4'],
                ['direct-1', 'direct-4', 'gammaboard']]):
            active = False
            def cli(*args, **kwargs):
                nonlocal active
                if args[:2] == ('benchmark', 'evaluator'):
                    self.assertFalse(active, 'direct baseline ran alongside active pipeline work')
                    return dict(samples_per_second=200)
                if args[:2] == ('run', 'create'):
                    return dict(run_id=1)
                if args[:2] == ('run', 'resume'):
                    active = True
                if args[:2] == ('run', 'pause'):
                    active = False
                if args[:2] == ('run', 'performance'):
                    self.assertIn('--interval', args)
                    self.assertIn('--max-age', args)
                    return dict(valid=True, issues=[], samples_per_second=100)
                return {}
            with tempfile.TemporaryDirectory() as tmp:
                session = Mock()
                session.cli.side_effect = cli
                record = bench.measure_case(session, Path(tmp),
                    dict(warmup_seconds=1, duration_seconds=4), 10, 4, 64, repeat, 500)
                self.assertEqual(record['measurement_order'], expected_order)
                self.assertTrue((Path(tmp)/'warmup.json').exists())
                self.assertEqual(record['cpu_iterations_per_sample'], 500)


class CleanupTests(unittest.TestCase):
    def test_cleanup_failure_is_fatal_not_a_recoverable_trial_error(self):
        from benchmark_common import TrialError
        def cli(*args, **kwargs):
            if args[:2] == ('run', 'pause'):
                raise RuntimeError('cleanup failed')
            return dict(run_id=7)
        session = Mock()
        session.cli.side_effect = cli
        with tempfile.TemporaryDirectory() as tmp, self.assertRaisesRegex(RuntimeError, 'cleanup failed') as raised:
            with bench.active_run(session, Path(tmp)/'run.toml', 4, '5s'):
                raise RuntimeError('measurement failed')
        self.assertNotIsInstance(raised.exception, TrialError)

    def test_failed_measurement_drains_and_removes_its_run(self):
        session = Mock()
        session.cli.return_value = {'run_id': 7}
        with tempfile.TemporaryDirectory() as tmp, self.assertRaisesRegex(RuntimeError, 'measurement failed'):
            with bench.active_run(session, Path(tmp)/'run.toml', 4, '5s'):
                raise RuntimeError('measurement failed')
        self.assertEqual([call.args for call in session.cli.call_args_list][-3:], [
            ('run', 'pause', 7), ('run', 'wait', 7, '--until', 'idle'), ('run', 'remove', '--yes', 7)])

    def test_unsuccessful_shutdown_cannot_report_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            session = bench.Session.__new__(bench.Session)
            session.directory = Path(tmp)
            session.output = Path(tmp)
            session.workers = []
            session.offset = 191
            session.process = Mock(returncode=1)
            session.process.poll.return_value = 1
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(RuntimeError), \
                    patch('benchmark_common.os.killpg'), patch('benchmark_common.time.sleep'):
                session.__exit__(None)
            self.assertTrue(session.directory.exists())

    def test_clean_shutdown_removes_only_the_private_deployment(self):
        with tempfile.TemporaryDirectory() as tmp:
            session = bench.Session.__new__(bench.Session)
            session.directory = Path(tmp)/'private'
            session.directory.mkdir()
            session.output = Path(tmp)
            session.workers = []
            session.offset = 191
            session.process = Mock(returncode=0)
            session.process.poll.return_value = 0
            session.__exit__(None)
            self.assertFalse(session.directory.exists())
            self.assertTrue(Path(tmp).exists())

    def test_deployment_cleanup_does_not_mask_the_measurement_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            session = bench.Session.__new__(bench.Session)
            session.directory = Path(tmp)/'private'
            session.directory.mkdir()
            session.output = Path(tmp)
            session.workers = []
            session.offset = 191
            session.process = Mock(returncode=None)
            session.process.poll.return_value = None
            session.process.wait.side_effect = [subprocess.TimeoutExpired('deploy',90),0]
            with contextlib.redirect_stderr(io.StringIO()), patch('benchmark_common.os.killpg'), \
                    patch('benchmark_common.time.sleep'):
                session.__exit__(RuntimeError, RuntimeError('original measurement failure'), None)
            self.assertFalse(json.loads((Path(tmp)/'cleanup.json').read_text())['clean'])
            self.assertTrue(session.directory.exists())

if __name__=='__main__': unittest.main()
