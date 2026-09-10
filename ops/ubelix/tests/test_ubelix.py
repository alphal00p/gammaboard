import sys
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import ubelix  # noqa: E402


def test_build_revision_defaults_and_override():
    args = ubelix.parser().parse_args(["build", "gammaboard"])
    assert args.revision == "HEAD"
    args = ubelix.parser().parse_args(
        ["build", "gammaloop", "--revision", "0123456789abcdef"]
    )
    assert args.revision == "0123456789abcdef"


def test_worker_database_url_is_passwordless():
    assert ubelix.database_url("compute-01", port_offset=2) == (
        "postgresql://postgres@compute-01:5402/gammaboard_db"
    )


def test_slurm_gpu_config_is_normalized():
    assert ubelix.derive_capabilities_from_config({"cores": 8, "gpu": True}) == {
        "cpus": 8,
        "gpu": 1,
    }
    assert ubelix.sbatch_args_from_config({"cores": 8, "gpu": True}) == [
        "--partition=gpu",
        "--gres=gpu:rtx4090:1",
        "--cpus-per-task=8",
    ]


def test_group_launches_have_stable_unique_names():
    groups = ubelix.launch_groups_for_request(
        {
            "id": 12,
            "args": {
                "groups": [
                    {"count": 2, "name_prefix": "cpu", "config": {"cores": 4}},
                    {"count": 1, "name_prefix": "cpu", "config": {"cores": 8}},
                ]
            },
        }
    )
    assert [group["node_name"] for group in groups] == ["cpu-12-1", "cpu-12-2", "cpu-12-3"]
    assert [group["capabilities"]["cpus"] for group in groups] == [4, 4, 8]


def test_resumed_request_preserves_reserved_name_and_scheduler_config():
    request = {"id": 99, "args": {"groups": [{"count": 1, "node_names": ["gpu-4"], "max_start_failures": 7, "config": {"partition": "gpu", "gres": "gpu:a100:1", "cpus": 8}}]}}
    worker = ubelix.launch_groups_for_request(request)[0]
    assert worker["node_name"] == "gpu-4"
    assert worker["config"]["gres"] == "gpu:a100:1"
    assert worker["max_start_failures"] == 7
    assert ubelix.parser().parse_args(["up", "--resume-workers"]).resume_workers
