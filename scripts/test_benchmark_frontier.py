from pathlib import Path
import tomllib
import unittest
from unittest.mock import Mock, patch
import tempfile
import time

import benchmark_frontier as f


class FrontierTests(unittest.TestCase):
    def setUp(self):
        self.suite=f.validate(tomllib.loads((f.bench.ROOT/'resources/templates/benchmarks/frontier.toml').read_text()))

    def test_modes_use_distinct_payloads_and_fixed_training_window(self):
        for mode in f.MODES:
            card=tomllib.loads(f.card(mode,321,self.suite))
            task=card['task_queue'][-1]
            config=task['sampler_aggregator']['config']
            self.assertEqual(card['evaluator']['cpu_iterations_per_sample'],0)
            self.assertEqual(card['evaluator']['value_coordinate'],0)
            self.assertEqual(card['evaluator']['timing']['per_sample_seconds'],.000321)
            self.assertAlmostEqual(card['evaluator']['timing']['sigma_per_sample_seconds'],.0000321)
            self.assertEqual(card['evaluator']['timing']['seed'],1234)
            self.assertEqual(set(card['evaluator']['timing']),
                             {'per_sample_seconds','sigma_per_sample_seconds','seed'})
            self.assertEqual(config['kind'],'havana_inference' if mode=='rng' else 'naive_monte_carlo')
            self.assertEqual(config.get('training_window_samples'),10**12 if mode=='training' else None)
            self.assertGreaterEqual(config['generation_batch_size'],16)
            if mode=='rng':self.assertEqual(len(card['task_queue']),2)

    def test_default_sparse_curves_cover_both_ends_and_selected_knees(self):
        points=f.validate_points(f.sparse_points(self.suite),self.suite)
        self.assertEqual(len(points),87)
        self.assertEqual(len(set(points)),len(points))
        for mode in f.MODES:
            for cost in self.suite['eval_us']:
                counts=[p.workers for p in points if p.mode==mode and p.eval_us==cost]
                self.assertEqual((min(counts),max(counts)),(1,512))
                self.assertEqual(counts,self.suite['workers'] if cost==0 else
                    sorted({1,4,16,64,256,512} | ({128 if mode=='rng' else 8} if cost==5 else set())))
        self.assertIn(f.Point('rng',0,512,524288),points)
        self.assertIn(f.Point('training',0,512,131072),points)

    def test_sparse_plan_respects_reduced_fleet_and_memory_limits(self):
        suite=dict(self.suite,modes=['rng'],workers=[4,8,16],sample_memory_budget=65536)
        for point in f.validate_points(f.sparse_points(suite),suite):
            self.assertIn(point.workers,suite['workers'])
            self.assertLessEqual(point.batch*(4*point.workers+2),suite['sample_memory_budget'])
        with self.assertRaisesRegex(ValueError,'--search or --points'):
            f.sparse_points(dict(self.suite,eval_us=[50]))

    def test_invalid_point_is_rejected_before_starting_any_processes(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as tmp:
            points=Path(tmp)/'points.json'
            f.bench.write_json(points,[f.asdict(f.Point('rng',0,1024,1024))])
            args=SimpleNamespace(suite=f.bench.ROOT/'resources/templates/benchmarks/frontier.toml',
                                 budget=None,points=points,search=False)
            with patch.object(f.bench,'Session') as session, self.assertRaisesRegex(ValueError,'invalid validation point'):
                f.execute(args)
            session.assert_not_called()

    def test_batch_caps_bound_slow_work_and_sample_residency(self):
        self.assertEqual(f.batch_limit(self.suite,.05,128),512)
        self.assertLessEqual(f.batch_limit(self.suite,.05,128)*.05,30)
        self.assertLessEqual(f.batch_limit(self.suite,.0000005,128)*514,self.suite['sample_memory_budget'])
        self.assertEqual(f.batch_limit(self.suite,1,1),16) # runner minimum

    def test_long_batches_get_enough_measurement_and_budget_headroom(self):
        point=f.Point('training',50000,1,512)
        duration=f.measurement_seconds(self.suite,point,'confirm')
        self.assertGreaterEqual(duration/(point.batch*.05),8)
        with tempfile.TemporaryDirectory() as tmp:
            search=f.Search(Mock(deadline=time.monotonic()+150),Path(tmp),self.suite,[1])
            self.assertFalse(search.has_time(point,'confirm'))
            search.session.deadline=time.monotonic()+600
            self.assertTrue(search.has_time(point,'confirm'))

    def test_warmup_passes_all_pre_tuning_work_including_buffered_draw(self):
        point=f.Point('training',5,4,1024)
        settings=f.queue_settings(point,self.suite)
        def snap(produced,completed,buffered=0):
            return dict(samplers=[dict(runtime_metrics=dict(produced_samples_total=produced,
                completed_samples_total=completed),engine_diagnostics=dict(runner=dict(
                queue_config=settings,buffered_generated_samples=buffered)))])
        before=snap(10000,5000,20000)
        self.assertTrue(f.applied(before,settings))
        self.assertFalse(f.warmed(snap(100000,38191),before,point))
        self.assertTrue(f.warmed(snap(100000,38192),before,point))
        self.assertFalse(f.warmed(snap(100000,38192),before,point,16))
        self.assertTrue(f.warmed(snap(100000,38208),before,point,16))

    def test_training_interval_covers_generation_feedback_with_small_eval_batches(self):
        for delay, batch in [(5, 65536), (200, 4096), (5000, 256)]:
            with self.subTest(delay=delay):
                config=tomllib.loads(f.card('training',delay,self.suite))['task_queue'][-1]['sampler_aggregator']['config']
                generation=config['generation_batch_size']
                self.assertGreater(generation,batch)
                for workers in [1,4,512]:
                    point=f.Point('training',delay,workers,batch)
                    duration=f.measurement_seconds(self.suite,point,'confirm')
                    self.assertGreaterEqual(duration*workers/(delay/1e6),2*generation)
                materialized=f.Point('materialized',delay,1,batch)
                self.assertLess(f.measurement_seconds(self.suite,materialized,'confirm'),
                                generation*delay/1e6)

    def test_fresh_confirmation_keeps_distinct_queue_settings(self):
        row=dict(point=f.asdict(f.Point('rng',.5,1,1024)),valid=True,adequate=True,
                 phase='confirm',rate=100,settings={'max_concurrent_insert_tasks':1},
                 observed_batch_size=1024,batches_per_second=.1,rss_bytes=100)
        fresh=dict(row,fresh_run=True,rate=80,settings={'max_concurrent_insert_tasks':2})
        result=f.summarize([row,fresh])
        self.assertEqual(len(result['configurations']),2)
        self.assertEqual(result['frontier'][0]['rate'],80)
        self.assertTrue(result['frontier'][0]['fresh_run'])

    def test_held_out_data_replaces_selection_data(self):
        row=dict(point=f.asdict(f.Point('rng',.5,1,1024)),valid=True,adequate=True,phase='search',rate=100,
                 settings={},observed_batch_size=1024,batches_per_second=.1,rss_bytes=100)
        result=f.summarize([row,dict(row,phase='confirm',rate=80)])['frontier'][0]
        self.assertTrue(result['confirmed']);self.assertEqual(result['rate'],80)
        self.assertEqual(f.summarize([dict(row,adequate=False)])['frontier'],[])

    def test_prefer_shorter_batch_only_when_it_retains_peak_throughput(self):
        row=dict(point=f.asdict(f.Point('rng',.5,1,1024)),valid=True,adequate=True,phase='search',rate=100,
                 settings={},observed_batch_size=1024,batches_per_second=.1,rss_bytes=100)
        large=dict(row,point=dict(row['point'],batch=4096),rate=104)
        self.assertEqual(f.summarize([row,large])['frontier'][0]['point']['batch'],1024)
        self.assertEqual(f.summarize([row,dict(large,rate=110)])['frontier'][0]['point']['batch'],4096)

    def test_worker_plateau_keeps_far_side_probe_and_confirms_selection(self):
        with tempfile.TemporaryDirectory() as tmp:
            session=Mock(deadline=time.monotonic()+1000)
            search=f.Search(session,Path(tmp),self.suite,[1,4,16,32,64,128])
            calls=[]
            def measure(live,point,phase='search'):
                calls.append((point,phase))
                return dict(point=f.asdict(point),valid=True,adequate=True,rate=100)
            search.measure=measure
            with patch.object(f,'LiveRun') as live:
                live.return_value.__enter__.return_value.settings = None
                search.run('training',5,time.monotonic()+100)
            self.assertEqual({p.workers for p,phase in calls}, {1,16,64,128})
            self.assertEqual(calls[-1][1],'confirm')
            self.assertEqual(calls[-1][0].workers,1)
            self.assertTrue(any(d['reason'].startswith('worker plateau') for d in search.decisions))

    def test_warmup_uses_completed_new_work_for_the_whole_fleet(self):
        def snapshot(completed,produced=16000):
            return dict(samplers=[dict(runtime_metrics=dict(completed_samples_total=completed,produced_samples_total=produced),engine_diagnostics=dict(runner=dict(buffered_generated_samples=0)))])
        point=f.Point('rng',.5,128,1024)
        baseline=snapshot(16000) # completed small-batch initialization
        self.assertFalse(f.warmed(snapshot(16000),baseline,point))
        self.assertFalse(f.warmed(snapshot(16000+2*1024),baseline,point))
        self.assertTrue(f.warmed(snapshot(16000+2*128*1024),baseline,point))

    def test_evaluator_work_without_accepted_progress_is_not_adequate(self):
        row=dict(point=f.asdict(f.Point('training',50000,128,16)),adequate=True,
                 completed_batches=100,rate=0.,elapsed_seconds=12.)
        self.assertFalse(f.adequate(row))
        self.assertFalse(f.adequate(dict(row,rate=127/12)))
        self.assertTrue(f.adequate(dict(row,rate=128/12)))

    def test_sampler_progress_uses_matching_counters_and_clock(self):
        def snapshot(n,ingested,t):
            return dict(samplers=[dict(runtime_metrics=dict(completed_samples_total=n,
                ingested_samples_total=ingested,
                busy=dict(elapsed_seconds=t),runner_epoch='e',node_uuid='n',task_id='t'))])
        measurement=dict(completed_samples=999,elapsed_seconds=3,
                         snapshots=[snapshot(100,80,4),snapshot(500,480,9)])
        self.assertEqual(f.sampler_progress(measurement),(400,400,5))
        # Feedback already present at the first publication is outside the interval.
        last=measurement['snapshots'][-1]['samplers'][0]['runtime_metrics']
        last['ingested_samples_total']=80
        self.assertEqual(f.sampler_progress(measurement),(400,0,5))
        last['ingested_samples_total']=79
        with self.assertRaisesRegex(RuntimeError,'invalid sampler progress'):f.sampler_progress(measurement)
        last['ingested_samples_total']=480
        measurement['snapshots'][-1]['samplers'][0]['runtime_metrics']['runner_epoch']='changed'
        with self.assertRaisesRegex(RuntimeError,'identity changed'):f.sampler_progress(measurement)

    def test_zero_delay_has_no_cpu_work_and_still_bounds_memory(self):
        evaluator=tomllib.loads(f.card('rng',0,self.suite))['evaluator']
        self.assertEqual(evaluator['timing']['per_sample_seconds'],0)
        self.assertEqual(evaluator['timing']['sigma_per_sample_seconds'],0)
        self.assertEqual(evaluator['cpu_iterations_per_sample'],0)
        self.assertLessEqual(f.batch_limit(self.suite,0,128)*514,self.suite['sample_memory_budget'])

    def test_batch_tuning_does_not_mistake_small_batch_saturation_for_worker_plateau(self):
        with tempfile.TemporaryDirectory() as tmp:
            search=f.Search(Mock(deadline=time.monotonic()+1000),Path(tmp),self.suite,[16])
            calls=[]
            def measure(live,point,phase='search'):
                calls.append(point)
                return dict(point=f.asdict(point),valid=True,adequate=True,
                            rate=min(point.batch/1024,64)*100)
            search.measure=measure
            best=search.tune_count(Mock(settings=None),f.Point('rng',.5,16,1024),time.monotonic()+100)
            self.assertEqual(best['point']['batch'],65536)
            self.assertIn(262144,[p.batch for p in calls])

    def test_short_context_still_checks_batch_and_queue_before_pruning(self):
        with tempfile.TemporaryDirectory() as tmp:
            search=f.Search(Mock(deadline=time.monotonic()+1000),Path(tmp),self.suite,[16])
            calls=[]
            def measure(live,point,phase='search'):
                calls.append(point)
                return dict(point=f.asdict(point),valid=True,adequate=True,rate=100)
            search.measure=measure
            search.tune_count(Mock(settings=None),f.Point('rng',.5,16,1024),time.monotonic()-1)
            self.assertIn(4096,[p.batch for p in calls])
            calls.clear()
            search.measure=lambda live,point: calls.append(point) or dict(
                point=f.asdict(point),valid=True,adequate=True,rate=2560)
            search.tune_count(Mock(settings=None),f.Point('training',50000,128,16),time.monotonic()-1)
            self.assertEqual(len(calls),1) # Already at the service-rate bound.

    def test_failed_peak_confirmation_checks_competitive_alternative(self):
        with tempfile.TemporaryDirectory() as tmp:
            search=f.Search(Mock(deadline=time.monotonic()+1000),Path(tmp),self.suite,[1,4,16])
            def row(point,rate=100):
                return dict(point=f.asdict(point),valid=True,adequate=True,rate=rate)
            search.tune_count=lambda live,point,limit,frontier_rate: row(point)
            search.confirm=Mock(side_effect=lambda point: row(point,50))
            with patch.object(f,'LiveRun'):
                search.run('rng',5,time.monotonic()+500)
            self.assertEqual([call.args[0].workers for call in search.confirm.call_args_list],[1,1,16])
            self.assertGreater(search.confirm.call_args_list[1].args[0].batch,search.confirm.call_args_list[0].args[0].batch)

    def test_worker_range_keeps_linear_anchors_and_refines_around_the_knee(self):
        workers=[1,2,4,8,16,32,64,128,256,512]
        records=[dict(point=f.asdict(f.Point('rng',0,4,131072)),phase='confirm',
                      valid=True,adequate=True,rate=18e6)]
        self.assertEqual(f.worker_range(workers,5,records,'rng'),[1,4,16,64,128,256,512])
        self.assertEqual(f.worker_range(workers,5000,records,'rng'),[1,4,16,64,256,512])
        self.assertEqual(f.worker_range(workers,.5,records,'rng'),[1,4,8,16,64,256,512])
        self.assertEqual(f.worker_range(workers,0,records,'rng'),workers)

    def test_zero_delay_keeps_all_counts_after_a_plateau(self):
        with tempfile.TemporaryDirectory() as tmp:
            search=f.Search(Mock(deadline=time.monotonic()+1000),Path(tmp),self.suite,[1,2,4,8,512])
            seen=[]
            def tune(live,point,limit,frontier_rate):
                seen.append(point.workers)
                return dict(point=f.asdict(point),valid=True,adequate=True,rate=100)
            search.tune_count=tune
            search.confirm=Mock(return_value=dict(valid=True,adequate=True,rate=100))
            with patch.object(f,'LiveRun'):
                search.run('rng',0,time.monotonic()-1)
            self.assertEqual(seen,[1,2,4,8,512])

    def test_clearly_dominated_fleet_skips_queue_matrix_after_batch_probe(self):
        with tempfile.TemporaryDirectory() as tmp:
            search=f.Search(Mock(deadline=time.monotonic()+1000),Path(tmp),self.suite,[128])
            calls=[]
            def measure(live,point,phase='search'):
                calls.append(point)
                return dict(point=f.asdict(point),valid=True,adequate=True,rate=100)
            search.measure=measure
            search.tune_count(Mock(settings=None),f.Point('rng',5,128,1024),time.monotonic()+100,1000)
            self.assertEqual([p.batch for p in calls],[1024,4096])

    def test_sleeping_process_count_is_independent_of_physical_core_count(self):
        infrastructure,cpus=f.worker_cpus(list(range(8)),dict.fromkeys(range(8),0),[1,64,512],4)
        self.assertEqual(len(cpus),512)
        self.assertEqual(set(cpus),{4,5,6,7})
        self.assertFalse(set(infrastructure)&set(cpus))
        with self.assertRaises(ValueError):f.worker_cpus([0,1],{0:0,1:0},[128],2)

    def test_comparison_rejects_different_cpu_and_telemetry_budgets(self):
        with tempfile.TemporaryDirectory() as tmp:
            before,after=Path(tmp)/'before',Path(tmp)/'after'
            before.mkdir();after.mkdir()
            manifest=dict(schema_version=5,suite=self.suite,workload='batch_sleep',
                          rate_source='sampler',evaluator_cpus=[0,1],niceness=0)
            f.bench.write_json(before/'manifest.json',manifest)
            f.bench.write_json(after/'manifest.json',dict(manifest,evaluator_cpus=[0,1,2,3]))
            with self.assertRaisesRegex(ValueError,'CPU budget'):f.compare(before,after)
            f.bench.write_json(after/'manifest.json',dict(manifest,suite=dict(self.suite,telemetry_interval_ms=1000)))
            with self.assertRaisesRegex(ValueError,'telemetry_interval'):f.compare(before,after)
            f.bench.write_json(after/'manifest.json',dict(manifest,value_coordinate=0))
            with self.assertRaisesRegex(ValueError,'value_coordinate'):f.compare(before,after)
            f.bench.write_json(after/'manifest.json',dict(manifest,suite=dict(self.suite,max_batch_size=262144)))
            with self.assertRaisesRegex(ValueError,'max_batch_size'):f.compare(before,after)

    def test_failed_explicit_confirmation_is_not_reported_as_completed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            points=root/'points.json'
            point=f.asdict(f.Point('rng',0,1,1024))
            f.bench.write_json(points,[point])
            binary=root/'gammaboard';binary.touch()
            args=Mock(suite=f.bench.ROOT/'resources/templates/benchmarks/frontier.toml',
                      budget=None,points=points,binary=binary,output=root/'results',port_offset=170)
            failed=dict(point=point,phase='confirm',valid=False,adequate=False,rate=None)
            with patch.object(f.bench,'physical_cpus',return_value=list(range(32))), \
                 patch.object(f,'core_loads',return_value=dict.fromkeys(range(32),0)), \
                 patch.object(f.os,'sched_setaffinity'), patch.dict(f.os.environ), \
                 patch.object(f.bench,'preserve_inputs',return_value=(binary,{})), \
                 patch.object(f.bench,'Session') as session, patch.object(f,'Search') as search:
                session.return_value.__enter__.return_value.deadline=time.monotonic()+1000
                search.return_value.confirm.return_value=failed
                search.return_value.records=[failed]
                f.execute(args)
            manifest=f.json.loads((args.output/'manifest.json').read_text())
            self.assertEqual(manifest['status'],'incomplete')
            self.assertEqual(manifest['unmeasured_contexts'],[point])
            self.assertEqual(manifest['timing_jitter'],dict(relative_sigma=.1,seed=1234))
            self.assertEqual(manifest['value_coordinate'],0)

    def test_rng_initializes_the_grid_before_attaching_the_full_fleet(self):
        with tempfile.TemporaryDirectory() as tmp:
            session=Mock()
            def cli(*args):
                if args[:2]==('run','create'):return dict(run_id=7)
                if args[:3]==('run','task','list'):
                    return [dict(id=1,name='initialize-grid',nr_completed_samples=16),
                            dict(id=2,name='measure',nr_completed_samples=0)]
            session.cli.side_effect=cli
            point=f.Point('rng',5000,512,256)
            live=f.LiveRun(session,Path(tmp)/'run',self.suite,'rng',5000,point)
            def wait(predicate):
                resumes=[c.args[-1] for c in session.cli.call_args_list if c.args[:2]==('run','resume')]
                self.assertEqual(resumes,[1])
            live.wait=wait
            live.__enter__()
            resumes=[c.args[-1] for c in session.cli.call_args_list if c.args[:2]==('run','resume')]
            self.assertEqual(resumes,[1,511])
            self.assertEqual(live.completed_offset,16)

    def test_single_point_validation_leaves_cleanup_to_its_private_deployment(self):
        session=Mock(single_run=True)
        live=f.LiveRun(session,Path('/unused'),self.suite,'rng',0)
        live.run=7
        live.__exit__(None)
        session.cli.assert_not_called()

    def test_comparison_rejects_fixed_delays_or_a_different_jitter_seed(self):
        with tempfile.TemporaryDirectory() as tmp:
            before,after=Path(tmp)/'before',Path(tmp)/'after'
            before.mkdir();after.mkdir()
            manifest=dict(schema_version=5,suite=self.suite,workload='batch_sleep',
                          rate_source='sampler',evaluator_cpus=[0,1],niceness=0)
            f.bench.write_json(after/'manifest.json',dict(manifest,timing_jitter=f.TIMING_JITTER))
            for change in [{},dict(timing_jitter=dict(relative_sigma=.1,seed=5678))]:
                f.bench.write_json(before/'manifest.json',dict(manifest,**change))
                with self.assertRaisesRegex(ValueError,'timing jitter'):f.compare(before,after)

    def test_invalid_config_rejected(self):
        for key,value in [('eval_us',[float('nan')]),('eval_us',[-1]),('workers',[1,1]),('workers',[1024]),('modes',['inference']),
                          ('budget_seconds',0),('unknown',4),('insert_concurrency',0),('insert_concurrency',1.5),
                          ('max_batch_seconds',30.1)]:
            with self.subTest(key=key),self.assertRaises(ValueError):f.validate(dict(self.suite,**{key:value}))


if __name__=='__main__':unittest.main()
