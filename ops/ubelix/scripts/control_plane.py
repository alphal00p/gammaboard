#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
import signal
import subprocess
import sys
import time
from pathlib import Path


def env_for_apptainer() -> dict[str, str]:
    env = os.environ.copy()
    env.pop("PGTZ", None)
    env.pop("PGOPTIONS", None)
    env.setdefault("APPTAINERENV_LANG", "C.UTF-8")
    env.setdefault("APPTAINERENV_LC_ALL", "C.UTF-8")
    env.setdefault("APPTAINERENV_TZ", "Etc/UTC")
    return env


def run(cmd: list[str], *, env: dict[str, str] | None = None, check: bool = True, capture: bool = False) -> subprocess.CompletedProcess[str]:
    kwargs: dict[str, object] = {"env": env}
    if capture:
        kwargs["stdout"] = subprocess.PIPE
        kwargs["stderr"] = subprocess.STDOUT
        kwargs["text"] = True
    result = subprocess.run(cmd, **kwargs)
    if check and result.returncode != 0:
        raise subprocess.CalledProcessError(result.returncode, cmd, getattr(result, "stdout", None))
    return result  # type: ignore[return-value]


def render_template(path: Path, mapping: dict[str, str]) -> str:
    text = path.read_text()
    for key, value in mapping.items():
        text = text.replace(f"${{{key}}}", value)
    return text


class ControlPlane:
    def __init__(self, mode: str):
        self.mode = mode
        self.env = env_for_apptainer()
        self.workspace_root = Path(os.environ["GAMMABOARD_WORKSPACE_ROOT"])
        self.image_path = Path(os.environ["GAMMABOARD_IMAGE"])
        self.deploy_name = os.environ.get("DEPLOY_NAME", "default")
        self.run_name = os.environ.get("RUN_NAME", "ubelix-hello")
        self.node_prefix = os.environ.get("NODE_PREFIX", "gb-hello")
        self.poll_timeout_seconds = int(os.environ.get("POLL_TIMEOUT_SECONDS", "300"))
        self.db_port = int(os.environ.get("DB_PORT", "5433"))
        self.api_port = int(os.environ.get("API_PORT", "4000"))
        self.frontend_port = int(os.environ.get("FRONTEND_PORT", "8080"))
        self.frontend_build_dir = Path(
            os.environ.get("FRONTEND_BUILD_DIR", str(self.workspace_root / "dashboard" / "build"))
        )
        self.hostname = os.environ.get("HOSTNAME", "unknown")
        self.user = os.environ.get("USER", "user")
        self.job_id = os.environ.get("SLURM_JOB_ID", str(os.getpid()))
        self.local_tmp = os.environ.get("SLURM_TMPDIR") or os.environ.get("TMPDIR") or "/tmp"

        self.control_log_dir = self.workspace_root / "logs" / "control"
        self.worker_log_dir = self.workspace_root / "logs" / "workers"
        self.runtime_dir = self.workspace_root / "runtime"
        self.instance_name = f"gb-{mode}-{self.job_id}"
        self.runtime_config = self.runtime_dir / f"runtime-{self.job_id}-{mode}.toml"
        self.run_toml = self.runtime_dir / f"run-{self.job_id}-{mode}.toml"
        self.nginx_conf = self.runtime_dir / f"nginx-{self.job_id}-{mode}.conf"

        self.server_proc: subprocess.Popen[bytes] | None = None
        self.nginx_proc: subprocess.Popen[bytes] | None = None
        self.sampler_proc: subprocess.Popen[bytes] | None = None
        self.eval_proc: subprocess.Popen[bytes] | None = None

    def instance_exec_cmd(self, inner_cmd: list[str]) -> list[str]:
        return ["apptainer", "exec", f"instance://{self.instance_name}", *inner_cmd]

    def gb_cmd(self, args: list[str]) -> list[str]:
        return self.instance_exec_cmd(["gammaboard", "--runtime-config", str(self.runtime_config), *args])

    def start_instance(self) -> None:
        run(["apptainer", "instance", "stop", self.instance_name], env=self.env, check=False)
        run(
            ["apptainer", "instance", "start", "-B", str(self.workspace_root), str(self.image_path), self.instance_name],
            env=self.env,
        )

    def stop_instance(self) -> None:
        run(["apptainer", "instance", "stop", self.instance_name], env=self.env, check=False)

    def setup_files(self) -> None:
        self.control_log_dir.mkdir(parents=True, exist_ok=True)
        self.worker_log_dir.mkdir(parents=True, exist_ok=True)
        self.runtime_dir.mkdir(parents=True, exist_ok=True)

        template = self.workspace_root / "ops" / "config" / "runtime_local_postgres.template.toml"
        if not template.exists():
            raise RuntimeError(f"missing runtime template: {template}")
        rendered = render_template(
            template,
            {
                "WORKSPACE_ROOT": str(self.workspace_root),
                "LOCAL_TMP": str(self.local_tmp),
                "DEPLOY_NAME": self.deploy_name,
                "DB_PORT": str(self.db_port),
            },
        )
        self.runtime_config.write_text(rendered)

    def cleanup(self) -> None:
        for proc in [self.server_proc, self.nginx_proc, self.sampler_proc, self.eval_proc]:
            if proc is not None and proc.poll() is None:
                proc.terminate()
        for proc in [self.server_proc, self.nginx_proc, self.sampler_proc, self.eval_proc]:
            if proc is not None:
                try:
                    proc.wait(timeout=5)
                except Exception:
                    proc.kill()
        try:
            run(self.gb_cmd(["node", "stop", "--all"]), env=self.env, check=False)
            run(self.gb_cmd(["run", "pause", self.run_name]), env=self.env, check=False)
            run(self.gb_cmd(["db", "stop"]), env=self.env, check=False)
        finally:
            self.stop_instance()
        for p in [self.runtime_config, self.run_toml, self.nginx_conf]:
            try:
                p.unlink()
            except FileNotFoundError:
                pass

    def run_hello_single(self) -> None:
        sampler_node = f"{self.node_prefix}-sampler"
        evaluator_node = f"{self.node_prefix}-eval-1"
        self.run_toml.write_text(
            f"""name = "{self.run_name}"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = {{ max_samples = 5_000 }}
accumulator = {{ config = "scalar" }}
sampler_aggregator = {{ config = {{ kind = "naive_monte_carlo" }} }}
"""
        )

        print(f"node: {self.hostname}")
        print(f"workspace_root: {self.workspace_root}")
        print(f"runtime_config: {self.runtime_config}")
        print(f"database_host_port: 127.0.0.1:{self.db_port}")
        print(f"sampler_node: {sampler_node}")
        print(f"evaluator_node: {evaluator_node}")
        print("starting local postgres")
        run(self.gb_cmd(["db", "start"]), env=self.env)

        sampler_out = open(self.worker_log_dir / f"{self.job_id}-{sampler_node}.out", "wb")
        sampler_err = open(self.worker_log_dir / f"{self.job_id}-{sampler_node}.err", "wb")
        eval_out = open(self.worker_log_dir / f"{self.job_id}-{evaluator_node}.out", "wb")
        eval_err = open(self.worker_log_dir / f"{self.job_id}-{evaluator_node}.err", "wb")
        self.sampler_proc = subprocess.Popen(self.gb_cmd(["node", "run", "--name", sampler_node]), env=self.env, stdout=sampler_out, stderr=sampler_err)
        self.eval_proc = subprocess.Popen(self.gb_cmd(["node", "run", "--name", evaluator_node]), env=self.env, stdout=eval_out, stderr=eval_err)

        print("waiting for workers to announce")
        time.sleep(10)
        run(self.gb_cmd(["node", "list"]), env=self.env, check=False)

        print(f"creating run {self.run_name}")
        run(self.gb_cmd(["run", "add", str(self.run_toml)]), env=self.env)

        print("auto-assigning workers")
        run(self.gb_cmd(["auto-assign", self.run_name, "1"]), env=self.env)

        deadline = time.time() + self.poll_timeout_seconds
        while time.time() < deadline:
            print(f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} poll: node list")
            run(self.gb_cmd(["node", "list"]), env=self.env, check=False)
            print(f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} poll: task list")
            task = run(self.gb_cmd(["run", "task", "list", self.run_name]), env=self.env, capture=True, check=False)
            out = task.stdout or ""
            print(out, end="")
            if "state=completed" in out:
                print("task completed")
                break
            if "state=failed" in out:
                raise RuntimeError("task failed")
            time.sleep(5)
        else:
            raise RuntimeError(f"timed out waiting for run completion ({self.poll_timeout_seconds}s)")

        run(self.gb_cmd(["run", "list", self.run_name]), env=self.env, check=False)
        print("hello-single finished")

    def write_nginx_conf(self) -> None:
        conf = f"""daemon off;
pid {self.runtime_dir}/nginx-{self.job_id}.pid;
error_log {self.control_log_dir}/{self.job_id}-nginx.err info;
events {{
  worker_connections 1024;
}}
http {{
  access_log {self.control_log_dir}/{self.job_id}-nginx-access.log;
  server {{
    listen 0.0.0.0:{self.frontend_port};
    server_name _;
    root {self.frontend_build_dir};
    index index.html;

    location /api/ {{
      proxy_pass http://127.0.0.1:{self.api_port};
      proxy_http_version 1.1;
      proxy_set_header Host $host;
      proxy_set_header X-Real-IP $remote_addr;
      proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
      proxy_set_header X-Forwarded-Proto $scheme;
    }}

    location / {{
      try_files $uri /index.html;
    }}
  }}
}}
"""
        self.nginx_conf.write_text(conf)

    def run_ui_single(self) -> None:
        if not self.frontend_build_dir.exists():
            raise RuntimeError(f"missing frontend build dir: {self.frontend_build_dir}")

        print(f"node: {self.hostname}")
        print(f"workspace_root: {self.workspace_root}")
        print(f"runtime_config: {self.runtime_config}")
        print(f"frontend_build_dir: {self.frontend_build_dir}")
        print(f"api_url: http://{self.hostname}:{self.api_port}")
        print(f"frontend_url: http://{self.hostname}:{self.frontend_port}")
        print(
            f"tunnel_cmd: ssh -N -L {self.frontend_port}:{self.hostname}:{self.frontend_port} {self.user}@submit03.unibe.ch"
        )

        print("starting local postgres")
        run(self.gb_cmd(["db", "start"]), env=self.env)

        server_out = open(self.control_log_dir / f"{self.job_id}-server.out", "wb")
        server_err = open(self.control_log_dir / f"{self.job_id}-server.err", "wb")
        self.server_proc = subprocess.Popen(
            self.gb_cmd(["server", "--server-config", str(self.workspace_root / "ops" / "config" / "server.toml")]),
            env=self.env,
            stdout=server_out,
            stderr=server_err,
        )

        self.write_nginx_conf()
        nginx_out = open(self.control_log_dir / f"{self.job_id}-frontend.out", "wb")
        self.nginx_proc = subprocess.Popen(
            self.instance_exec_cmd(["/usr/sbin/nginx", "-c", str(self.nginx_conf)]),
            env=self.env,
            stdout=nginx_out,
            stderr=subprocess.STDOUT,
        )

        time.sleep(2)
        if self.server_proc.poll() is not None:
            raise RuntimeError(f"server process exited early; see {self.control_log_dir}/{self.job_id}-server.err")
        if self.nginx_proc.poll() is not None:
            raise RuntimeError(f"nginx process exited early; see {self.control_log_dir}/{self.job_id}-nginx.err")

        self.server_proc.wait()
        self.nginx_proc.wait()

    def execute(self) -> None:
        self.setup_files()
        self.start_instance()
        signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
        signal.signal(signal.SIGINT, lambda *_: sys.exit(0))
        try:
            if self.mode == "hello-single":
                self.run_hello_single()
            elif self.mode == "ui-single":
                self.run_ui_single()
            else:
                raise RuntimeError(f"unsupported mode: {self.mode}")
        finally:
            self.cleanup()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", required=True, choices=["hello-single", "ui-single"])
    args = parser.parse_args()
    cp = ControlPlane(args.mode)
    cp.execute()


if __name__ == "__main__":
    main()
