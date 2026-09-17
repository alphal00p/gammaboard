"""Scientific validity and resource-budget checks, without launching a deployment."""
import copy
from pathlib import Path
import tomllib
import unittest
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

    def test_invalid_or_over_budget_suite_is_rejected(self):
        for key,value in [('workers',[1,100]),('duration_seconds',float('nan')),('repetitions',0),
                          ('budget_seconds',1801),('batch_sizes',[1]),('eval_us',[1,1]),('duration_seconds',300)]:
            with self.subTest(key=key,value=value):
                suite = copy.deepcopy(self.suite); suite[key]=value
                with self.assertRaises(ValueError): bench.validate_suite(suite)

    def test_invalid_observations_are_not_plotted_as_zero(self):
        rows = [dict(valid=False,batch_size=256,workers=1,eval_us=10,rate=None),
                dict(valid=True,batch_size=256,workers=1,eval_us=10,rate=12)]
        self.assertEqual(bench.grouped(rows,'rate'),{(256,1,10):[12]})


class CleanupTests(unittest.TestCase):
    def test_unsuccessful_shutdown_cannot_report_success(self):
        import contextlib
        import io
        import tempfile
        from unittest.mock import Mock
        with tempfile.TemporaryDirectory() as tmp:
            session = bench.Session.__new__(bench.Session)
            session.directory = Path(tmp)
            session.process = Mock(returncode=1)
            session.process.poll.return_value = 1
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(RuntimeError):
                session.__exit__(None)
            self.assertTrue(session.directory.exists())

if __name__=='__main__': unittest.main()
