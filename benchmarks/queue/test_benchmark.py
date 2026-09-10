"""Fast tests of benchmark generation and methodology, without deployments."""
import importlib.util
from pathlib import Path
import tomllib
import unittest
import tempfile
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('benchmark_queue',ROOT/'scripts/benchmark_queue.py')
bench=importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)

class BenchmarkTests(unittest.TestCase):
    def test_cli_single_and_multi_value_responses(self):
        self.assertEqual(bench.parse_cli_json('{"run_id":1}'),{'run_id':1})
        self.assertEqual(bench.parse_cli_json('{"node":"a"}\n{"node":"b"}\n'),[{'node':'a'},{'node':'b'}])

    def test_matrix_and_generated_tomls(self):
        suite=tomllib.loads((ROOT/'benchmarks/queue/suite.toml').read_text())
        cases=bench.cases(suite)
        self.assertEqual(len(cases),16)
        self.assertEqual(len({bench.case_id(c) for c in cases}),16)
        for case in cases:
            card=tomllib.loads(bench.run_card(case,12,'test',10000))
            timing=card['evaluator']['timing']
            self.assertAlmostEqual(case['evaluators']/timing['per_sample_seconds'],case['rate'])
            params=card['task_queue'][0]['sampler_aggregator']['config']
            self.assertEqual('training_window_samples' in params,case['regime'].startswith('training_'))
            self.assertEqual(card['task_queue'][0]['stop_condition']['max_samples'],10000)
    def test_default_budget_and_burst_stalls(self):
        suite=tomllib.loads((ROOT/'benchmarks/queue/suite.toml').read_text())
        self.assertEqual(suite['evaluators'],[1,4,16,64])
        self.assertEqual(suite['rates'],[1000,2000000])
        # Paired measurements leave substantial time for launch/drain/cleanup.
        self.assertLess(len(bench.cases(suite))*2*(suite['warmup_seconds']+suite['measurement_seconds']),180)
        for case in bench.cases(suite):
            card=tomllib.loads(bench.run_card(case,42,'test',10000000))
            params=card['task_queue'][0]['sampler_aggregator']['config']
            self.assertEqual(params['generation_timing']['per_sample_seconds'],0)
            if case['regime']=='training_burst':
                self.assertLessEqual(params['training_window_samples'],100000)
                self.assertEqual(params['update_timing']['overhead_seconds'],.5)
                self.assertEqual(params['ingest_timing']['per_sample_seconds'],0)

    def test_oversized_run_is_rejected_before_deployment(self):
        with tempfile.TemporaryDirectory() as temp:
            output=Path(temp)/'never-created'
            result=subprocess.run([sys.executable,str(ROOT/'scripts/benchmark_queue.py'),
                                   'run','--binary',sys.executable,'--duration','300',
                                   '--output',str(output)],capture_output=True,text=True,timeout=5)
            self.assertEqual(result.returncode,2)
            self.assertIn('exceed the wall-time budget',result.stderr)
            self.assertFalse(output.exists())

    def test_expensive_64_worker_case_and_seed_pairing(self):
        case=dict(evaluators=64,rate=1000,regime='training_small',continuous_dims=6,noise_fraction=.1)
        a=tomllib.loads(bench.run_card(case,42,'A',10000))
        b=tomllib.loads(bench.run_card(case,42,'B',10000))
        self.assertEqual(a['evaluator'],b['evaluator'])
        self.assertEqual(a['evaluator']['timing']['per_sample_seconds'],.064)
    def test_binary_snapshot_is_independent_of_later_builds(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); source=root/'source'; source.write_bytes(b'first build')
            variants,metadata=bench.snapshot_binaries([('A',source)],root)
            source.write_bytes(b'second build')
            self.assertEqual(variants[0][1].read_bytes(),b'first build')
            self.assertEqual(metadata['A']['sha256'],bench.identity(variants[0][1])['sha256'])

    def test_completed_record_journal_round_trip(self):
        with tempfile.TemporaryDirectory() as temp:
            output=Path(temp)
            self.assertEqual(bench.load_records(output),[])
            a=dict(variant='A',repetition=0,samples_per_second=100)
            b=dict(variant='B',repetition=0,samples_per_second=110)
            bench.append_record(output,a)
            bench.append_record(output,b)
            self.assertEqual(bench.load_records(output),[a,b])

    def test_summary_retains_repetition_variation(self):
        case=dict(evaluators=1,rate=1000,regime='inference')
        rows=[dict(case=case,variant='A',samples_per_second=r) for r in [100,120,140]]
        summary=bench.summary(rows)[0]
        self.assertEqual(summary['mean'],120)
        self.assertEqual(summary['stdev'],20)

if __name__=='__main__':unittest.main()
