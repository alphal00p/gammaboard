"""Checker regressions use real subprocesses and intentionally faulty samplers."""

import hashlib
import importlib.util
import json
import math
import os
import subprocess
import sys
import time
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[2] / "check_sampler.py"
FIXTURES = Path(__file__).with_name("sampler_check_fixtures.py")
ROOT = SCRIPT.parent.parent
spec = importlib.util.spec_from_file_location("sampler_check", SCRIPT)
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)


def run_sampler(name, tmp_path, args=None, extra=()):
    return run_checker([f"{FIXTURES}:{name}"], tmp_path, args, extra)


def run_checker(entry_args, tmp_path, args=None, extra=()):
    return subprocess.run(
        [
            sys.executable,
            "-B",
            str(SCRIPT),
            *entry_args,
            "--max-samples",
            "20",
            "--max-calls",
            "16",
            "--timeout",
            "8",
            "--time-limit",
            "90",
            "--init-args",
            json.dumps(args or {}),
            *extra,
        ],
        cwd=tmp_path,
        env={
            **os.environ,
            "PYTHONDONTWRITEBYTECODE": "1",
            "PYTHONPATH": str(ROOT / "process_api/python/src"),
        },
        text=True,
        capture_output=True,
        timeout=110,
        check=False,
    )


def row(output, name):
    return next(line for line in output.splitlines() if line.startswith(name))


def test_full_contract_with_training_artifacts_and_distinct_stage_paths(tmp_path):
    artifacts = tmp_path / "sampler-artifacts"
    result = run_sampler(
        "UniformSampler",
        tmp_path,
        {"training": True, "save_path": str(artifacts)},
        ("--extra-domains",),
    )
    assert result.returncode == 0, result.stdout + result.stderr
    for name in [
        "Snapshot JSON and recovery without notice",
        "Uncommitted B preserves committed A",
        "Recovery with pending feedback",
        "Checkpoint commit notification",
        "Stage handoff and source preservation",
        "Domain: ragged",
        "GenerationStatus.WAITING",
    ]:
        assert row(result.stdout, name).endswith("PASS")
    assert "responsible for cleaning up" in result.stdout
    assert artifacts.is_dir() and list(artifacts.glob("*.json")), (
        "sampler artifacts must not be deleted by the checker"
    )
    assert list(tmp_path.glob("sampler-artifacts.check-stage-*"))
    assert not list(tmp_path.rglob("__pycache__"))


def test_optional_defaults_are_skipped_and_runner_leaves_no_files(tmp_path):
    result = run_sampler("InferenceOnly", tmp_path)
    assert result.returncode == 0, result.stdout + result.stderr
    for name in [
        "Checkpoint commit notification",
        "Stage handoff and source preservation",
        "PDF return contract",
        "Diagnostics JSON",
    ]:
        assert row(result.stdout, name).endswith("SKIP")
    assert list(tmp_path.iterdir()) == []


@pytest.mark.parametrize(
    "name,feature,args",
    [
        ("NoSnapshot", "Snapshot JSON and recovery without notice", {}),
        ("LargeIntegerSnapshot", "Snapshot JSON and recovery without notice", {}),
        ("BrokenRestore", "Snapshot JSON and recovery without notice", {}),
        ("LostFeedback", "Recovery with pending feedback", {"training": True}),
        ("NonIdempotentNotification", "Checkpoint commit notification", {}),
        ("NegativePdf", "PDF return contract", {}),
        ("BadMass", "Discrete mass return contract", {}),
        ("InvalidWeights", "Generation, bounds and shutdown", {}),
        ("OversizedBatch", "Generation, bounds and shutdown", {}),
        ("Deadlock", "Generation, bounds and shutdown", {}),
    ],
)
def test_negative_controls_fail_with_feature_and_summary(tmp_path, name, feature, args):
    result = run_sampler(name, tmp_path, args)
    assert result.returncode == 1, result.stdout + result.stderr
    assert row(result.stdout, feature).endswith("FAIL"), result.stdout
    assert (
        "Summary" in result.stdout
        and "Your sampler may create or modify files" in result.stdout
    )


def test_checkpoint_overwrite_is_detected_without_removing_sampler_artifacts(tmp_path):
    artifacts = tmp_path / "bad-artifacts"
    result = run_sampler("OverwriteSnapshot", tmp_path, {"save_path": str(artifacts)})
    assert result.returncode == 1, result.stdout + result.stderr
    assert row(result.stdout, "Uncommitted B preserves committed A").endswith("FAIL")
    assert (artifacts / "overwritten.json").is_file()


def test_finite_sampler_may_finish_before_the_budget(tmp_path):
    result = run_sampler("UniformSampler", tmp_path, {"limit": 5, "training": True})
    assert result.returncode == 0, result.stdout + result.stderr
    assert row(result.stdout, "GenerationStatus.FINISHED").endswith("PASS")
    assert row(result.stdout, "Uncommitted B preserves committed A").endswith("PASS")


def test_finished_without_samples_is_inconclusive(tmp_path):
    result = run_sampler("UniformSampler", tmp_path, {"limit": 0})
    assert result.returncode == 2, result.stdout + result.stderr
    assert row(result.stdout, "GenerationStatus.FINISHED").endswith("PASS")
    assert row(result.stdout, "Generation, bounds and shutdown").endswith(
        "INCONCLUSIVE"
    )


def test_partial_pdf_availability_is_allowed(tmp_path):
    result = run_sampler("PartialPdf", tmp_path)
    assert result.returncode == 0, result.stdout + result.stderr
    assert row(result.stdout, "PDF return contract").endswith("PASS")


def test_missing_prefix_masses_fail_only_when_required(tmp_path):
    ordinary = run_sampler("NoMass", tmp_path)
    assert ordinary.returncode == 0, ordinary.stdout + ordinary.stderr
    assert row(ordinary.stdout, "Discrete mass return contract").endswith("SKIP")
    required = run_sampler("NoMass", tmp_path, extra=("--require-stratified",))
    assert required.returncode == 1, required.stdout + required.stderr
    assert row(required.stdout, "Discrete mass return contract").endswith("FAIL")


def test_stratified_fixture_carries_feedback_labels_and_exact_masses(tmp_path):
    result = run_sampler(
        "UniformSampler", tmp_path, {"training": True}, ("--require-stratified",)
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert row(result.stdout, "Discrete mass return contract").endswith("PASS")


def test_hung_sampler_is_killed_and_summary_is_printed(tmp_path):
    pid_path = tmp_path / "sampler.pid"
    start = time.monotonic()
    result = run_sampler(
        "HungSampler", tmp_path, {"save_path": str(pid_path)}, ("--timeout", "1.5")
    )
    assert result.returncode == 1, result.stdout + result.stderr
    assert "timed out" in result.stdout and "Summary" in result.stdout
    assert time.monotonic() - start < 15
    pid = int(pid_path.read_text())
    with pytest.raises(ProcessLookupError):
        os.kill(pid, 0)


def test_overall_deadline_is_inconclusive_and_stops_worker(tmp_path):
    result = run_sampler("HungSampler", tmp_path, extra=("--time-limit", "1.5"))
    assert result.returncode == 2, result.stdout + result.stderr
    assert "overall time budget exhausted" in result.stdout
    assert "Summary" in result.stdout


def test_module_entrypoint_is_resolved_without_copying_it():
    from gammaboard_process import Sampler

    assert checker.load_entry("gammaboard_process.abc:Sampler") is Sampler


def test_source_files_and_init_args_file_are_not_modified(tmp_path):
    watched = [
        SCRIPT,
        FIXTURES,
        ROOT / "process_api/python/src/gammaboard_process/abc.py",
    ]
    args = tmp_path / "args.json"
    args.write_text('{"seed": 123}')
    watched.append(args)
    before = {p: hashlib.sha256(p.read_bytes()).digest() for p in watched}
    result = run_sampler(
        "InferenceOnly", tmp_path, extra=("--init-args", "@" + str(args))
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert '"seed": 123' in result.stdout
    assert before == {p: hashlib.sha256(p.read_bytes()).digest() for p in watched}
    assert list(tmp_path.iterdir()) == [args]


def test_invalid_setup_and_reserved_arguments_are_reported(tmp_path):
    result = run_sampler("UniformSampler", tmp_path, {"domain": {}})
    assert result.returncode == 2
    assert "reserved" in result.stderr


def test_extra_domain_rejection_is_inconclusive_not_a_false_pass(tmp_path):
    result = run_sampler("OnlyContinuous", tmp_path, extra=("--extra-domains",))
    assert result.returncode == 0, result.stdout + result.stderr
    assert row(result.stdout, "Domain: ragged").endswith("INCONCLUSIVE")


def test_low_budget_statistics_are_inconclusive(tmp_path):
    result = run_sampler("UniformSampler", tmp_path, extra=("--statistics",))
    assert result.returncode == 0, result.stdout + result.stderr
    assert row(result.stdout, "Toy-integral statistics").endswith("INCONCLUSIVE")
    assert row(result.stdout, "PDF / sampling-weight consistency").endswith("PASS")


def test_unit_integral_statistics(tmp_path):
    result = run_sampler(
        "UniformSampler", tmp_path, extra=("--statistics", "--max-samples", "40")
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert row(result.stdout, "Toy-integral statistics").endswith("PASS")


@pytest.mark.parametrize("training", [False, True])
def test_numpy_sampler_tutorial_satisfies_the_process_contract(tmp_path, training):
    entry = ROOT / "process_api/examples/python_sampler/src/demo_sampler.py"
    args = {"training_samples": 128, "feedback_window": 16} if training else {}
    extra = [
        "--require-stratified", "--max-samples", "512", "--max-calls", "128",
        "--stage-init-args", '{"training_samples":0}',
    ]
    if not training:
        extra.append("--statistics")
    result = run_checker([f"{entry}:UniformSampler"], tmp_path, args, extra)
    assert result.returncode == 0, result.stdout + result.stderr
    for name in [
        "Snapshot JSON and recovery without notice", "Discrete mass return contract",
        "Stage handoff and source preservation",
    ]:
        assert row(result.stdout, name).endswith("PASS"), result.stdout
    if training:
        assert row(result.stdout, "Recovery with pending feedback").endswith("PASS")
        assert row(result.stdout, "GenerationStatus.FINISHED").endswith("PASS")
    else:
        assert row(result.stdout, "PDF / sampling-weight consistency").endswith("PASS")
        assert row(result.stdout, "Toy-integral statistics").endswith("PASS")


def rectangular(cards=(), continuous=2, strata=0):
    return {
        "rectangular": {
            "discrete_cardinalities": list(cards),
            "continuous_dims": continuous,
            "strata_dims": strata,
        }
    }


def stratum_tree(branches):
    return {
        "discrete": {
            "axis_role": "stratum",
            "branches": [
                {"index": index, "domain": domain} for index, domain in branches
            ],
        }
    }


def toy_statistics(name="UniformSampler", *, domain=None, args=None, samples=512):
    options = checker.parse_args(
        [
            f"{FIXTURES}:{name}",
            "--statistics",
            "--require-stratified",
            "--max-samples", str(samples),
            "--max-calls", "128",
            "--batch-sizes", "1,7,31",
            "--init-args", json.dumps(args or {}),
            "--domain", json.dumps(domain or rectangular([2], strata=1)),
        ]
    )
    return checker.Suite(options, checker.Report()).statistics()


def test_stratified_statistics_run_through_cli(tmp_path):
    result = run_sampler(
        "UniformSampler",
        tmp_path,
        extra=(
            "--statistics", "--require-stratified",
            "--max-samples", "512", "--max-calls", "64",
        ),
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert row(result.stdout, "Toy-integral statistics").endswith("PASS")
    assert "stratum (0,) integral of x[0]-1/2:" in result.stdout
    assert "expected -0.125" in result.stdout and "expected 0.125" in result.stdout
    assert "combined integral of x[0]-1/2:" in result.stdout


@pytest.mark.parametrize(
    "domain,volume",
    [
        (rectangular([2], strata=1), 1),
        (rectangular([2, 2, 3], strata=2), 3),
        (
            stratum_tree([
                (0, rectangular([3])),
                (1, stratum_tree([(k, rectangular([3])) for k in (0, 1, 3)])),
            ]),
            3,
        ),
    ],
)
def test_stratified_toys_with_nonuniform_density_and_integrand_dimensions(
    domain, volume
):
    from gammaboard_process import Domain

    detail = toy_statistics("NonuniformSampler", domain=domain)
    for prefix in Domain.from_dict(domain).stratum_prefixes():
        assert f"stratum {prefix} integral of 1:" in detail
    assert f"expected {volume}; combined integral of x[0]-1/2:" in detail
    assert "smoke check only" in detail


def test_stratified_toy_exact_accounting_retains_zeros_and_sums_variances():
    detail = toy_statistics("ScriptedStrataSampler", samples=128)
    assert "stratum (0,): N=32, matches=16, p=0.25" in detail
    assert "stratum (1,): N=96, matches=48, p=0.75" in detail
    # Each stratum has half zeros and half +/-1/4 after p*W; the different
    # attempted counts (not matches or total N) determine its standard error.
    for prefix, n, mean in [(0, 32, -0.125), (1, 96, 0.125)]:
        error = 0.125 / math.sqrt(n - 1)
        assert (
            f"stratum ({prefix},) integral of x[0]-1/2: {mean} ± {error:.3g}"
            in detail
        )
    combined_error = 0.125 * math.sqrt(1 / 31 + 1 / 95)
    assert (
        f"combined integral of x[0]-1/2: 0 ± {combined_error:.3g}; expected 0"
        in detail
    )
    assert "combined integral of 1: 1 ±" in detail


def test_stratum_checks_detect_wrong_weights_despite_signed_cancellation():
    with pytest.raises(checker.Unexercised) as exc:
        toy_statistics("ScriptedStrataSampler", args={"weight_scale": 10}, samples=512)
    detail = str(exc.value)
    assert "combined integral of x[0]-1/2: 0 ±" in detail
    assert "stratum (0,) integral of x[0]-1/2: -1.25" in detail
    assert "disagreement needs investigation" in detail


def test_stratified_toy_detects_incorrect_normalized_prefix_masses():
    with pytest.raises(checker.Unexercised, match="disagreement needs investigation"):
        toy_statistics("ScriptedStrataSampler", args={"reported_p": 0.5})


@pytest.mark.parametrize(
    "name,kwargs,message",
    [
        ("NoMass", {}, "exact stratum-prefix masses"),
        ("UniformSampler", {"samples": 20}, "32 attempts"),
        ("UniformSampler", {"args": {"training": True}}, "frozen proposal"),
        ("ScriptedStrataSampler", {"args": {"miss_only": True}}, "matches=0"),
        (
            "UniformSampler",
            {"domain": rectangular([2], continuous=0, strata=1)},
            "continuous coordinate",
        ),
        (
            "UniformSampler",
            {"domain": stratum_tree([
                (0, rectangular(continuous=1)), (1, rectangular(continuous=2)),
            ])},
            "same rectangular integrand domain",
        ),
    ],
)
def test_stratified_toy_unexercised_cases(name, kwargs, message):
    with pytest.raises(checker.Unexercised, match=message):
        toy_statistics(name, **kwargs)


def test_stratified_toy_rejects_zero_mass():
    with pytest.raises(checker.CheckError, match="positive prefix masses"):
        toy_statistics("ScriptedStrataSampler", args={"reported_p": 0})


def test_portable_snapshot_values():
    for state in [{}, [], 0, False, {"optional": None}, {"rng": str(2**128)}]:
        checker.validate_snapshot(state)
    for state in [None, {"bad": float("nan")}, {"rng": 2**128}]:
        with pytest.raises((checker.CheckError, ValueError)):
            checker.validate_snapshot(state)


def test_real_console_entrypoint_uses_public_protocol_and_its_shebang(tmp_path):
    executable = tmp_path / "sampler env" / "bin" / "sampler"
    executable.parent.mkdir(parents=True)
    executable.write_text(
        f"#!{sys.executable}\n"
        "import runpy, sys\n"
        f"assert sys.executable == {sys.executable!r}\n"
        f"sys.argv = [{str(FIXTURES)!r}, 'UniformSampler']\n"
        f"runpy.run_path({str(FIXTURES)!r}, run_name='__main__')\n"
    )
    executable.chmod(0o755)
    before = set(tmp_path.rglob("*"))
    result = run_checker(
        ["./" + str(executable.relative_to(tmp_path))],
        tmp_path,
        {"training": True},
    )
    assert result.returncode == 0, result.stdout + result.stderr
    for feature in [
        "Entry point and initialization",
        "Recovery with pending feedback",
        "Checkpoint commit notification",
        "Stage handoff and source preservation",
        "PDF return contract",
    ]:
        assert row(result.stdout, feature).endswith("PASS"), result.stdout
    assert "cannot distinguish a no-op hook" in result.stdout
    assert set(tmp_path.rglob("*")) == before


def test_command_array_preserves_arguments_cwd_and_legacy_defaults(tmp_path):
    workdir = tmp_path / "worker directory"
    workdir.mkdir()
    script = workdir / "worker with spaces.py"
    literal_args = ["spaces in argument", "$(touch unwanted); `touch unwanted2`"]
    script.write_text(
        "import os, runpy, sys\n"
        f"assert os.getcwd() == {str(workdir)!r}\n"
        f"assert sys.argv[1:] == {literal_args!r}\n"
        f"sys.argv = [{str(FIXTURES)!r}, 'InferenceOnly', '--legacy']\n"
        f"runpy.run_path({str(FIXTURES)!r}, run_name='__main__')\n"
    )
    command = [sys.executable, "-B", script.name, *literal_args]
    before = set(tmp_path.rglob("*"))
    result = run_checker(
        ["--command", json.dumps(command), "--cwd", str(workdir)], tmp_path
    )
    assert result.returncode == 0, result.stdout + result.stderr
    for feature in [
        "Checkpoint commit notification",
        "Stage handoff and source preservation",
        "PDF return contract",
        "Discrete mass return contract",
        "Diagnostics JSON",
        "Persisted output JSON",
    ]:
        assert row(result.stdout, feature).endswith("SKIP"), result.stdout
    assert set(tmp_path.rglob("*")) == before


@pytest.mark.parametrize(
    "name,feature,extra",
    [
        ("NegativePdf", "PDF return contract", ()),
        ("BrokenRestore", "Snapshot JSON and recovery without notice", ()),
        ("NonIdempotentNotification", "Checkpoint commit notification", ()),
        ("BrokenStage", "Stage handoff and source preservation", ()),
        ("NoMass", "Discrete mass return contract", ("--require-stratified",)),
    ],
)
def test_executable_failures_are_not_reported_as_optional_absence(
    tmp_path, name, feature, extra
):
    command = [sys.executable, "-B", str(FIXTURES), name]
    result = run_checker(["--command", json.dumps(command)], tmp_path, extra=extra)
    assert result.returncode == 1, result.stdout + result.stderr
    assert row(result.stdout, feature).endswith("FAIL"), result.stdout


def test_command_validation():
    import argparse

    for value in [
        "[]",
        '"sampler --arg"',
        "[null]",
        '[""]',
        '["worker", 3]',
        '["worker", "\\u0000"]',
    ]:
        with pytest.raises(argparse.ArgumentTypeError):
            checker.json_command(value)
    assert checker.json_command('["worker", "", "--arg with spaces"]') == [
        "worker",
        "",
        "--arg with spaces",
    ]
    for argv in [[], ["worker", "--command", '["worker"]']]:
        with pytest.raises(SystemExit) as exc:
            checker.parse_args(argv)
        assert exc.value.code == 2
