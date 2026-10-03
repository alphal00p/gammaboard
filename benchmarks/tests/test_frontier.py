from pathlib import Path
import tomllib
import unittest
from unittest.mock import Mock, patch
import tempfile
import time

from benchmarks import frontier as f


class FrontierTests(unittest.TestCase):
    def setUp(self):
        self.suite = f.validate(
            tomllib.loads((f.bench.ROOT / "benchmarks/frontier.toml").read_text())
        )

    def test_process_evaluator_measurement_does_not_require_mock_sleep_counters(self):
        evaluator = dict(worker_id="e", rss_bytes=0, metrics=dict(engine_diagnostics={}))
        snapshot = dict(evaluators=[evaluator], samplers=[])
        measured = dict(
            valid=True,
            issues=[],
            snapshots=[snapshot, snapshot],
            evaluator_deltas=[dict(batches_completed=8, samples_evaluated=128)],
        )
        counters = dict(
            produced_samples_total=128,
            completed_samples_total=128,
            ingested_samples_total=0,
            queue=dict(rolling=dict(insert_bundle_payload_bytes_per_batch=dict(mean=896))),
        )
        with patch.object(f, "sampler_progress", return_value=(128, 0, 1)), patch.object(
            f, "runtime", return_value=counters
        ), patch.object(f, "active_evaluators", return_value=[evaluator]), patch.object(
            f.bench, "activity", return_value={}
        ):
            row = f.measurement_row(measured, f.Point("materialized", 0, 1, 16))
        self.assertTrue(row["valid"] and row["adequate"])
        self.assertEqual(row["rate"], 128)
        self.assertIsNone(row["sleep_requested_seconds"])
        self.assertIsNone(row["sleep_actual_seconds"])

    def test_modes_use_distinct_payloads_and_fixed_training_window(self):
        for mode in f.MODES:
            card = tomllib.loads(f.card(mode, 321, self.suite))
            task = card["task_queue"][-1]
            config = task["sampler_aggregator"]["config"]
            self.assertEqual(card["evaluator"]["cpu_iterations_per_sample"], 0)
            self.assertEqual(card["evaluator"]["value_coordinate"], 0)
            self.assertEqual(card["evaluator"]["timing"]["per_sample_seconds"], 0.000321)
            self.assertAlmostEqual(
                card["evaluator"]["timing"]["sigma_per_sample_seconds"], 0.0000321
            )
            self.assertEqual(card["evaluator"]["timing"]["seed"], 1234)
            self.assertEqual(
                set(card["evaluator"]["timing"]),
                {"per_sample_seconds", "sigma_per_sample_seconds", "seed"},
            )
            self.assertEqual(
                config["kind"], "havana_inference" if mode == "rng" else "naive_monte_carlo"
            )
            self.assertEqual(
                config.get("training_window_samples"), 10**12 if mode == "training" else None
            )
            self.assertGreaterEqual(card["sampler_aggregator_runner_params"]["queue"]["max_generation_size"], 16)
            self.assertNotIn("max_generation_size", config)
            if mode == "rng":
                self.assertEqual(len(card["task_queue"]), 2)

    def test_pool_size_defaults_to_current_capacity(self):
        self.assertEqual(
            tomllib.loads(f.card("training", 0, self.suite))["sampler_aggregator_runner_params"][
                "db_pool_size"
            ],
            6,
        )
        legacy = {k: v for k, v in self.suite.items() if k != "sampler_db_pool_size"}
        f.validate(legacy)
        self.assertEqual(
            tomllib.loads(f.card("training", 0, legacy))["sampler_aggregator_runner_params"][
                "db_pool_size"
            ],
            6,
        )
        for value in [0, 7, True, 2.5]:
            with self.assertRaisesRegex(ValueError, "sampler_db_pool_size"):
                f.validate(dict(self.suite, sampler_db_pool_size=value))

    def test_sampler_io_threads_are_recorded_and_validated(self):
        legacy = {k: v for k, v in self.suite.items() if k != "sampler_io_threads"}
        for suite, expected in [(legacy, 1), (dict(self.suite, sampler_io_threads=3), 3)]:
            f.validate(suite)
            self.assertEqual(
                tomllib.loads(f.card("training", 0, suite))["sampler_aggregator_runner_params"][
                    "io_threads"
                ],
                expected,
            )
        for value in [0, -1, True, 2.5]:
            with self.assertRaisesRegex(ValueError, "sampler_io_threads"):
                f.validate(dict(self.suite, sampler_io_threads=value))

    def test_default_sparse_curves_cover_both_ends_and_selected_knees(self):
        points = f.validate_points(f.sparse_points(self.suite), self.suite)
        self.assertEqual(len(points), 58)
        self.assertEqual(len(set(points)), len(points))
        for mode in self.suite["modes"]:
            for cost in self.suite["eval_us"]:
                counts = [p.workers for p in points if p.mode == mode and p.eval_us == cost]
                self.assertEqual((min(counts), max(counts)), (1, 512))
                self.assertEqual(
                    counts,
                    (
                        self.suite["workers"]
                        if cost == 0
                        else sorted(
                            {1, 4, 16, 64, 256, 512}
                            | ({128 if mode == "rng" else 8} if cost == 5 else set())
                        )
                    ),
                )
        self.assertIn(f.Point("training", 0, 512, 32768), points)

    def test_sparse_plan_respects_reduced_fleet_and_memory_limits(self):
        suite = dict(self.suite, modes=["rng"], workers=[4, 8, 16], sample_memory_budget=65536)
        for point in f.validate_points(f.sparse_points(suite), suite):
            self.assertIn(point.workers, suite["workers"])
            self.assertLessEqual(
                point.batch * (4 * point.workers + 2), suite["sample_memory_budget"]
            )
        with self.assertRaisesRegex(ValueError, "--points"):
            f.sparse_points(dict(self.suite, eval_us=[50]))

    def test_invalid_point_is_rejected_before_starting_any_processes(self):
        from types import SimpleNamespace

        with tempfile.TemporaryDirectory() as tmp:
            points = Path(tmp) / "points.json"
            f.bench.write_json(points, [f.asdict(f.Point("rng", 0, 1024, 1024))])
            args = SimpleNamespace(
                suite=f.bench.ROOT / "benchmarks/frontier.toml",
                budget=None,
                points=points,
                search=False,
            )
            with patch.object(f.bench, "Session") as session, self.assertRaisesRegex(
                ValueError, "invalid validation point"
            ):
                f.execute(args)
            session.assert_not_called()

    def test_batch_caps_bound_slow_work_and_sample_residency(self):
        self.assertEqual(f.batch_limit(self.suite, 0.05, 128), 512)
        self.assertLessEqual(f.batch_limit(self.suite, 0.05, 128) * 0.05, 30)
        self.assertLessEqual(
            f.batch_limit(self.suite, 0.0000005, 128) * 514, self.suite["sample_memory_budget"]
        )
        self.assertEqual(f.batch_limit(self.suite, 1, 1), 16)  # runner minimum

    def test_long_batches_get_enough_measurement_and_budget_headroom(self):
        point = f.Point("training", 50000, 1, 512)
        duration = f.measurement_seconds(self.suite, point)
        self.assertGreaterEqual(duration / (point.batch * 0.05), 8)
        with tempfile.TemporaryDirectory() as tmp:
            search = f.Measurements(
                Mock(deadline=time.monotonic() + 150), Path(tmp), self.suite, [1]
            )
            self.assertFalse(search.has_time(point))
            search.session.deadline = time.monotonic() + 600
            self.assertTrue(search.has_time(point))

    def test_warmup_passes_all_pre_tuning_work_including_buffered_draw(self):
        point = f.Point("training", 5, 4, 1024)
        settings = f.queue_settings(point, self.suite)

        def snap(produced, completed, buffered=0):
            return dict(
                samplers=[
                    dict(
                        runtime_metrics=dict(
                            produced_samples_total=produced, completed_samples_total=completed
                        ),
                        engine_diagnostics=dict(
                            runner=dict(queue_config=settings, buffered_generated_samples=buffered)
                        ),
                    )
                ]
            )

        before = snap(10000, 5000, 20000)
        self.assertTrue(f.applied(before, settings))
        self.assertFalse(f.warmed(snap(100000, 38191), before, point))
        self.assertTrue(f.warmed(snap(100000, 38192), before, point))
        self.assertFalse(f.warmed(snap(100000, 38192), before, point, 16))
        self.assertTrue(f.warmed(snap(100000, 38208), before, point, 16))

    def test_training_interval_covers_generation_feedback_with_small_eval_batches(self):
        for delay, batch in [(5, 65536), (200, 4096), (5000, 256)]:
            with self.subTest(delay=delay):
                config = tomllib.loads(f.card("training", delay, self.suite))[
                    "sampler_aggregator_runner_params"
                ]["queue"]
                generation = config["max_generation_size"]
                self.assertGreater(generation, batch)
                for workers in [1, 4, 512]:
                    point = f.Point("training", delay, workers, batch)
                    duration = f.measurement_seconds(self.suite, point)
                    self.assertGreaterEqual(duration * workers / (delay / 1e6), 2 * generation)
                materialized = f.Point("materialized", delay, 1, batch)
                self.assertGreaterEqual(
                    f.measurement_seconds(self.suite, materialized),
                    2 * generation * delay / 1e6,
                )

    def test_summary_keeps_distinct_queue_settings(self):
        row = dict(
            point=f.asdict(f.Point("rng", 0.5, 1, 1024)),
            valid=True,
            adequate=True,
            rate=100,
            settings={"max_concurrent_insert_tasks": 1},
            observed_batch_size=1024,
            batches_per_second=0.1,
            rss_bytes=100,
        )
        fresh = dict(row, rate=80, settings={"max_concurrent_insert_tasks": 2})
        result = f.summarize([row, fresh])
        self.assertEqual(len(result["configurations"]), 2)
        self.assertEqual(result["frontier"][0]["rate"], 100)

    def test_summary_combines_repeats_and_excludes_inadequate_intervals(self):
        row = dict(
            point=f.asdict(f.Point("materialized", 5, 1, 1024)),
            valid=True,
            adequate=True,
            rate=100,
            settings={},
            observed_batch_size=1024,
            batches_per_second=0.1,
            rss_bytes=100,
        )
        result = f.summarize([row, dict(row, rate=80), dict(row, rate=999, adequate=False)])[
            "frontier"
        ][0]
        self.assertEqual(result["rate"], 90)
        self.assertEqual(result["trials"], 2)
        self.assertEqual(f.summarize([dict(row, adequate=False)])["frontier"], [])

    def test_transport_cap_preserves_bulk_generation(self):
        self.assertEqual(f.batch_limit(self.suite, 0, 1), 32768)
        self.assertEqual(f.max_generation_size(self.suite, 0), 262144)
        self.assertEqual(
            f.max_generation_size(dict(self.suite, max_batch_size=65536), 0), 262144
        )

    def test_prefer_shorter_batch_only_when_it_retains_peak_throughput(self):
        row = dict(
            point=f.asdict(f.Point("rng", 0.5, 1, 1024)),
            valid=True,
            adequate=True,
            rate=100,
            settings={},
            observed_batch_size=1024,
            batches_per_second=0.1,
            rss_bytes=100,
        )
        large = dict(row, point=dict(row["point"], batch=4096), rate=104)
        self.assertEqual(f.summarize([row, large])["frontier"][0]["point"]["batch"], 1024)
        self.assertEqual(
            f.summarize([row, dict(large, rate=110)])["frontier"][0]["point"]["batch"], 4096
        )

    def test_warmup_uses_completed_new_work_for_the_whole_fleet(self):
        def snapshot(completed, produced=16000):
            return dict(
                samplers=[
                    dict(
                        runtime_metrics=dict(
                            completed_samples_total=completed, produced_samples_total=produced
                        ),
                        engine_diagnostics=dict(runner=dict(buffered_generated_samples=0)),
                    )
                ]
            )

        point = f.Point("rng", 0.5, 128, 1024)
        baseline = snapshot(16000)  # completed small-batch initialization
        self.assertFalse(f.warmed(snapshot(16000), baseline, point))
        self.assertFalse(f.warmed(snapshot(16000 + 2 * 1024), baseline, point))
        self.assertTrue(f.warmed(snapshot(16000 + 2 * 128 * 1024), baseline, point))

    def test_evaluator_work_without_accepted_progress_is_not_adequate(self):
        row = dict(
            point=f.asdict(f.Point("training", 50000, 128, 16)),
            adequate=True,
            completed_batches=100,
            rate=0.0,
            elapsed_seconds=12.0,
        )
        self.assertFalse(f.adequate(row))
        self.assertFalse(f.adequate(dict(row, rate=127 / 12)))
        self.assertTrue(f.adequate(dict(row, rate=128 / 12)))

    def test_sampler_progress_uses_matching_counters_and_clock(self):
        def snapshot(n, ingested, t):
            return dict(
                samplers=[
                    dict(
                        runtime_metrics=dict(
                            completed_samples_total=n,
                            ingested_samples_total=ingested,
                            busy=dict(elapsed_seconds=t),
                            runner_epoch="e",
                            node_uuid="n",
                            task_id="t",
                        )
                    )
                ]
            )

        measurement = dict(
            completed_samples=999,
            elapsed_seconds=3,
            snapshots=[snapshot(100, 80, 4), snapshot(500, 480, 9)],
        )
        self.assertEqual(f.sampler_progress(measurement), (400, 400, 5))
        # Feedback already present at the first publication is outside the interval.
        last = measurement["snapshots"][-1]["samplers"][0]["runtime_metrics"]
        last["ingested_samples_total"] = 80
        self.assertEqual(f.sampler_progress(measurement), (400, 0, 5))
        last["ingested_samples_total"] = 79
        with self.assertRaisesRegex(RuntimeError, "invalid sampler progress"):
            f.sampler_progress(measurement)
        last["ingested_samples_total"] = 480
        measurement["snapshots"][-1]["samplers"][0]["runtime_metrics"]["runner_epoch"] = "changed"
        with self.assertRaisesRegex(RuntimeError, "identity changed"):
            f.sampler_progress(measurement)

    def test_zero_delay_has_no_cpu_work_and_still_bounds_memory(self):
        evaluator = tomllib.loads(f.card("rng", 0, self.suite))["evaluator"]
        self.assertEqual(evaluator["timing"]["per_sample_seconds"], 0)
        self.assertEqual(evaluator["timing"]["sigma_per_sample_seconds"], 0)
        self.assertEqual(evaluator["cpu_iterations_per_sample"], 0)
        self.assertLessEqual(
            f.batch_limit(self.suite, 0, 128) * 514, self.suite["sample_memory_budget"]
        )

    def test_sleeping_process_count_is_independent_of_physical_core_count(self):
        infrastructure, cpus = f.worker_cpus(
            list(range(8)), dict.fromkeys(range(8), 0), [1, 64, 512], 4
        )
        self.assertEqual(len(cpus), 512)
        self.assertEqual(set(cpus), {4, 5, 6, 7})
        self.assertFalse(set(infrastructure) & set(cpus))
        with self.assertRaises(ValueError):
            f.worker_cpus([0, 1], {0: 0, 1: 0}, [128], 2)

    def test_failed_explicit_confirmation_is_not_reported_as_completed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            points = root / "points.json"
            point = f.asdict(f.Point("materialized", 0, 1, 1024))
            f.bench.write_json(points, [point])
            binary = root / "gammaboard"
            binary.touch()
            args = Mock(
                suite=f.bench.ROOT / "benchmarks/frontier.toml",
                budget=None,
                points=points,
                binary=binary,
                output=root / "results",
                port_offset=170,
                effective_suite=None,
                cpu_limit=None,
                database_directory=Path("/tmp"),
            )
            failed = dict(point=point, valid=False, adequate=False, rate=None)
            with patch.object(f.bench, "physical_cpus", return_value=list(range(32))), patch.object(
                f.bench, "core_loads", return_value=dict.fromkeys(range(32), 0)
            ), patch.object(f.os, "sched_setaffinity"), patch.dict(f.os.environ), patch.object(
                f.bench, "preserve_inputs", return_value=(binary, {})
            ), patch.object(
                f.bench, "Session"
            ) as session, patch.object(
                f, "Measurements"
            ) as search:
                session.return_value.__enter__.return_value.deadline = time.monotonic() + 1000
                search.return_value.confirm.return_value = failed
                search.return_value.records = [failed]
                f.execute(args)
            manifest = f.json.loads((args.output / "manifest.json").read_text())
            self.assertEqual(manifest["status"], "incomplete")
            self.assertEqual(manifest["unmeasured_contexts"], [point])
            self.assertEqual(manifest["timing_jitter"], dict(relative_sigma=0.1, seed=1234))
            self.assertEqual(manifest["value_coordinate"], 0)

    def test_rng_initializes_the_grid_before_attaching_the_full_fleet(self):
        with tempfile.TemporaryDirectory() as tmp:
            session = Mock()

            def cli(*args):
                if args[:2] == ("run", "create"):
                    return dict(run_id=7)
                if args[:3] == ("run", "task", "list"):
                    return [
                        dict(id=1, name="initialize-grid", nr_completed_samples=16),
                        dict(id=2, name="measure", nr_completed_samples=0),
                    ]

            session.cli.side_effect = cli
            point = f.Point("rng", 5000, 512, 256)
            live = f.LiveRun(session, Path(tmp) / "run", self.suite, "rng", 5000, point)

            def wait(predicate):
                resumes = [
                    c.args[-1]
                    for c in session.cli.call_args_list
                    if c.args[:2] == ("run", "resume")
                ]
                self.assertEqual(resumes, [1])

            live.wait = wait
            live.__enter__()
            resumes = [
                c.args[-1] for c in session.cli.call_args_list if c.args[:2] == ("run", "resume")
            ]
            self.assertEqual(resumes, [1, 511])
            self.assertEqual(live.completed_offset, 16)

    def test_single_point_validation_leaves_cleanup_to_its_private_deployment(self):
        session = Mock(single_run=True)
        live = f.LiveRun(session, Path("/unused"), self.suite, "rng", 0)
        live.run = 7
        live.__exit__(None)
        session.cli.assert_not_called()

    def test_completed_prefix_bursts_cannot_masquerade_as_steady_progress(self):
        point = f.Point("materialized", 5000, 1, 256)
        self.assertFalse(f.progress_agrees(point, 14 * 256, 10 * 256))
        self.assertTrue(f.progress_agrees(point, 32 * 256, 31 * 256))
        self.assertFalse(f.progress_agrees(point, 100 * 256, 80 * 256))

    def test_generation_window_removes_partial_cycles_and_recomputes_deltas(self):
        suite = dict(self.suite, max_generation_size=4096)
        point = f.Point("training", 5000, 1, 256)

        def snapshot(t, accepted, evaluated):
            return dict(
                run_id=1,
                nodes=[dict(name="e", live=True, active_run_id=1, active_role="evaluator")],
                samplers=[
                    dict(
                        runtime_metrics=dict(
                            runner_epoch="s",
                            node_uuid="s",
                            task_id="1",
                            completed_samples_total=accepted + 16,
                            ingested_samples_total=accepted,
                            busy=dict(elapsed_seconds=t),
                        )
                    )
                ],
                evaluators=[
                    dict(
                        worker_id="e",
                        created_at=str(t),
                        metrics=dict(
                            epoch="e",
                            node_uuid="e",
                            task_id="1",
                            samples_evaluated=evaluated,
                            batches_completed=evaluated // 256,
                            cumulative=dict(evaluate_seconds=t),
                        ),
                    )
                ],
            )

        snapshots = [
            snapshot(0, 2048, 3000),
            snapshot(10, 4096, 4096),
            snapshot(20, 6000, 6100),
            snapshot(30, 8192, 8192),
            snapshot(39, 9000, 12288),
        ]
        raw = dict(
            valid=True,
            issues=[],
            snapshots=snapshots,
            evaluator_deltas=[dict(samples_evaluated=9288)],
            elapsed_seconds=39,
        )
        result = f.generation_window(raw, point, suite, completed_offset=16)
        self.assertEqual(result["window_selection"]["first_snapshot"], 1)
        self.assertEqual(result["window_selection"]["last_snapshot"], 3)
        self.assertEqual(f.sampler_progress(result), (4096, 4096, 20))
        self.assertEqual(result["evaluator_deltas"][0]["samples_evaluated"], 4096)
        self.assertEqual(result["evaluator_deltas"][0]["batches_completed"], 16)
        self.assertEqual(result["evaluator_deltas"][0]["evaluate_seconds"], 20)
        self.assertEqual(len(raw["snapshots"]), 5)
        # A single boundary, or a very short cycle, keeps the original interval.
        short = dict(raw, snapshots=snapshots[:3])
        self.assertIs(f.generation_window(short, point, suite, 16), short)
        snapshots[-1] = snapshot(100, 9000, 10240)
        self.assertIs(f.generation_window(raw, point, suite, 16), raw)

    def test_invalid_config_rejected(self):
        for key, value in [
            ("eval_us", [float("nan")]),
            ("eval_us", [-1]),
            ("workers", [1, 1]),
            ("workers", [1024]),
            ("modes", ["inference"]),
            ("budget_seconds", 0),
            ("unknown", 4),
            ("insert_concurrency", 0),
            ("insert_concurrency", 1.5),
            ("max_batch_seconds", 30.1),
        ]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                f.validate(dict(self.suite, **{key: value}))


if __name__ == "__main__":
    unittest.main()
