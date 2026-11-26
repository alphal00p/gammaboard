//! Local process lifecycle tests. Opt in because these own disposable PostgreSQL processes.
use serde_json::Value;
use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, Instant},
};

struct Instance {
    root: tempfile::TempDir,
    config: PathBuf,
}
impl Instance {
    fn new() -> Self {
        Self::configured(true)
    }
    fn configured(bind_ports: bool) -> Self {
        let root = tempfile::Builder::new()
            .prefix("gb-cli-")
            .tempdir()
            .unwrap();
        let config = root.path().join("instance.toml");
        let ports = if bind_ports {
            (0..3)
                .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let port = |i: usize| {
            ports
                .get(i)
                .map(|listener| listener.local_addr().unwrap().port())
                .unwrap_or(25000 + i as u16)
        };
        fs::create_dir(root.path().join("dashboard")).unwrap();
        fs::write(
            root.path().join("dashboard/index.html"),
            "local instance test",
        )
        .unwrap();
        fs::write(
            &config,
            format!(
                r#"
name = "test"
[paths]
state_dir = "control"
log_dir = "logs"
temp_dir = "tmp"
template_dir = "{}/resources/templates"
resource_roots = ["{}/resources"]
[database]
mode = "managed"
host = "127.0.0.1"
port = {}
name = "gammaboard"
user = "postgres"
data_dir = "postgres"
socket_dir = "socket"
[server]
api_port = {}
dashboard_port = {}
dashboard_dir = "dashboard"
[shutdown]
sampler_drain_timeout_seconds = 20
node_stop_timeout_seconds = 10
"#,
                env!("CARGO_MANIFEST_DIR"),
                env!("CARGO_MANIFEST_DIR"),
                port(0),
                port(1),
                port(2)
            ),
        )
        .unwrap();
        Self { root, config }
    }
    fn cmd(&self) -> Command {
        let mut cmd =
            Command::new(Path::new(env!("CARGO_BIN_EXE_gammaboard")).with_file_name("gb"));
        // Different cwd proves instance-relative paths and embedded migrations work.
        cmd.current_dir("/tmp")
            .arg("--instance")
            .arg(&self.config)
            .arg("--json");
        cmd
    }
    fn output(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> Value {
        let output = self.output(args);
        assert!(
            output.status.success(),
            "{args:?}: {}\n{}\nSUPERVISOR:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            fs::read_to_string(self.root.path().join("logs/service.log")).unwrap_or_default()
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
            panic!(
                "{args:?}: invalid JSON {e}: {}",
                String::from_utf8_lossy(&output.stdout)
            )
        })
    }
    fn card(&self, name: &str, samples: u64) -> PathBuf {
        let path = self.root.path().join(format!("{name}.toml"));
        let raw = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("resources/templates/runs/installation-smoke.toml"),
        )
        .unwrap();
        fs::write(
            &path,
            raw.replace("installation-smoke", name)
                .replace("10_000", &samples.to_string()),
        )
        .unwrap();
        path
    }
}
impl Instance {
    fn receipt(&self) -> Value {
        fs::read(self.root.path().join("control/status.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null)
    }
    fn wait_stopped(&self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.receipt()["state"] != "stopped" {
            assert!(
                Instant::now() < deadline,
                "service exit not confirmed: {}",
                self.receipt()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        if self.receipt().is_null() {
            return;
        }
        if !matches!(
            self.receipt()["state"].as_str(),
            Some("stopped" | "closing")
        ) {
            let _ = self.output(&["stop", "--timeout", "3s"]);
            if let Ok(output) = self.cmd().arg("status").output()
                && let Ok(status) = serde_json::from_slice::<Value>(&output.stdout)
                && status["cleanup_required"] == true
                && let Some(id) = status["stop_id"].as_str()
            {
                // Only synthetic external jobs are used in these tests.
                let _ = self.output(&["control", "acknowledge-cleanup", id]);
                let _ = self.output(&["stop", "--timeout", "10s"]);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        while self.receipt()["state"] != "stopped" && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        if std::thread::panicking() {
            eprintln!(
                "SERVICE LOG: {}",
                fs::read_to_string(self.root.path().join("logs/service.log")).unwrap_or_default()
            );
        }
    }
}

#[test]
#[ignore = "starts a combined service and exercises the separately built lightweight gb binary"]
fn lightweight_client_remote_workflow() {
    let instance = Instance::new();
    instance.ok(&["start", "--workers", "2", "--wait"]);
    let definition: toml::Value =
        toml::from_str(&fs::read_to_string(&instance.config).unwrap()).unwrap();
    let port = definition["server"]["api_port"].as_integer().unwrap();
    let endpoint = format!("http://127.0.0.1:{port}");
    let binary = Path::new(env!("CARGO_BIN_EXE_gammaboard")).with_file_name("gb");
    assert!(
        binary.exists(),
        "build the workspace before running this test"
    );
    let call = |args: &[&str]| {
        let output = Command::new(&binary)
            .current_dir("/tmp")
            .args(["--endpoint", &endpoint, "--json"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if output.stdout.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice::<Value>(&output.stdout).unwrap()
        }
    };
    let card = instance.card("remote-client", 100);
    let result = call(&["run", "create", card.to_str().unwrap(), "--start"]);
    let id = result["created"]["run_id"].as_i64().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let result = call(&["run", "results", &id]);
        if result["run"]["lifecycle_state"] == "completed" {
            break;
        }
        assert!(Instant::now() < deadline, "run did not complete");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(call(&["worker", "list"]).as_array().unwrap().len(), 2);
    let launched = call(&["worker", "launch", "--count", "1", "--wait"]);
    assert_eq!(launched["request_ids"].as_array().unwrap().len(), 1);
    assert!(
        !instance
            .output(&[
                "worker",
                "requests",
                "cancel",
                launched["request_ids"][0].as_str().unwrap()
            ])
            .status
            .success()
    );
    assert_eq!(call(&["worker", "list"]).as_array().unwrap().len(), 3);
    let worker = call(&["worker", "list", "--state", "all"])[0].clone();
    let name = worker["node_name"].as_str().unwrap();
    let inspected = call(&["worker", "inspect", name]);
    assert_eq!(inspected["node_name"], name);
    assert_eq!(inspected["state"], "live");
    assert!(inspected.get("live").is_none() && inspected.get("offline").is_none());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let http = reqwest::Client::new();
        for path in [
            "/admin/db/restart",
            "/admin/workers/save-for-restart",
            "/admin/control/shutdown",
            "/nodes/auto-run",
            "/node-launch-requests",
        ] {
            let response = http
                .post(format!("{endpoint}/api{path}"))
                .body("{}")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND, "{path}");
        }
        let response = http
            .post(format!("{endpoint}/api/runs"))
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
        let response = http
            .get(format!("{endpoint}/api/workers?state=all"))
            .send()
            .await
            .unwrap();
        let workers: Value = response.json().await.unwrap();
        assert_eq!(workers.as_array().unwrap().len(), 3);
        assert_eq!(
            workers[0].as_object().unwrap().keys().collect::<Vec<_>>(),
            inspected.as_object().unwrap().keys().collect::<Vec<_>>()
        );
    });
    assert_eq!(
        call(&["run", "list"]).as_array().unwrap().len(),
        1,
        "retired reset route must not delete data"
    );

    assert_eq!(
        call(&["run", "inspect", &id])["run"]["lifecycle_state"],
        "completed"
    );
    assert_eq!(
        call(&["run", "wait", &id, "--until", "completed"])["unfinished_tasks"],
        0
    );
    let clone = call(&["run", "duplicate", &id, "copy"])["run_id"]
        .as_i64()
        .unwrap()
        .to_string();
    let tasks = call(&["run", "task", "list", &clone]);
    let task = tasks[0]["id"].as_str().unwrap().to_string();
    let new_task = call(&["run", "task", "duplicate", &clone, &task])[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    call(&["run", "task", "remove", &clone, &new_task, "--yes"]);
    call(&["run", "remove", &clone, "--yes"]);
    assert_eq!(call(&["config", "validate"])["valid"], true);
    assert_eq!(call(&["status"])["database"]["reachable"], true);
    let definition_file = instance.root.path().join("export.toml");
    call(&[
        "run",
        "export",
        &id,
        "--kind",
        "definition",
        "--output",
        definition_file.to_str().unwrap(),
    ]);
    assert!(
        fs::read_to_string(definition_file)
            .unwrap()
            .contains("remote-client")
    );
    let backup = instance.root.path().join("backup.dump");
    call(&["db", "backup", "--output", backup.to_str().unwrap()]);
    assert!(fs::read(backup).unwrap().starts_with(b"PGDMP"));
    let output = instance.root.path().join("download.json");
    call(&[
        "run",
        "export",
        &id,
        "--kind",
        "results",
        "--output",
        output.to_str().unwrap(),
    ]);
    let downloaded: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(downloaded["run"]["lifecycle_state"], "completed");
    assert_eq!(
        call(&["stop", "--wait", "--timeout", "60s"])["stop_result"]["phase"],
        "ready_to_exit"
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let receipt: Value = serde_json::from_slice(
            &fs::read(instance.root.path().join("control/status.json")).unwrap(),
        )
        .unwrap();
        if receipt["state"] == "stopped" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "service did not exit after acknowledging application shutdown"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn instance_configuration_is_explicit_and_offline() {
    let instance = Instance::configured(false);
    let config = instance.ok(&["local", "config", "validate"]);
    assert_eq!(
        config["paths"]["state_dir"],
        instance.root.path().join("control").display().to_string()
    );
    assert!(!instance.root.path().join("postgres").exists());
    let failed = instance.output(&["local", "run", "validate", "does-not-exist.toml"]);
    assert!(!failed.status.success());
    let lines = String::from_utf8(failed.stderr).unwrap();
    assert_eq!(
        lines.lines().count(),
        1,
        "bootstrap should emit one structured error: {lines}"
    );
    let error: Value = serde_json::from_str(&lines).unwrap();
    assert!(error["error"]["message"].is_string());
}

#[test]
fn external_gate_prevents_bootstrap_without_a_database() {
    let instance = Instance::configured(false);
    let raw = fs::read_to_string(&instance.config).unwrap();
    fs::write(&instance.config, format!("externally_managed=true\n{raw}")).unwrap();
    for args in [
        &["start"][..],
        &["restart"][..],
        &["start", "db"][..],
        &["local", "run", "validate", "missing.toml", "--probe"][..],
        &["local", "run", "validate", "missing.toml"][..],
        &["local", "db", "create"][..],
    ] {
        let output = instance.output(args);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("externally managed"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(!instance.root.path().join("postgres").exists());
}

#[test]
#[ignore = "starts two disposable databases and control services"]
fn independent_instances_and_additive_worker_launches() {
    let a = Instance::new();
    let b = Instance::new();
    a.ok(&["start", "db"]);
    assert!(!a.root.path().join("control/status.json").exists());
    a.ok(&["start", "server", "--workers", "2", "--wait"]);
    a.ok(&["start", "--workers", "1", "--wait"]);
    b.ok(&["restart"]); // restart also starts a stopped instance
    b.ok(&["restart"]); // exactly one JSON response after ordered teardown/start
    assert!(!b.output(&["restart", "server"]).status.success());
    assert!(!b.output(&["restart", "db"]).status.success());
    assert_eq!(b.ok(&["status"])["ready"], true);
    assert_eq!(a.ok(&["worker", "list"]).as_array().unwrap().len(), 3);
    assert_eq!(b.ok(&["worker", "list"]).as_array().unwrap().len(), 0);
    let created = a.ok(&["run", "create", a.card("isolated", 100).to_str().unwrap()]);
    assert!(created["created"]["run_id"].is_number());
    assert!(b.ok(&["run", "list"]).as_array().unwrap().is_empty());
    a.ok(&["stop"]);
    assert_eq!(b.ok(&["status"])["ready"], true);
    // start must wait for owner teardown after application-level acknowledgment.
    a.ok(&["start", "--restore-workers"]);
    let deadline = Instant::now() + Duration::from_secs(30);
    while a.ok(&["worker", "list"]).as_array().unwrap().len() != 3 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "starts an externally managed service and simulates scheduler cleanup"]
fn external_gate_and_cleanup_handshake() {
    let instance = Instance::new();
    let raw = fs::read_to_string(&instance.config).unwrap();
    fs::write(&instance.config, format!("externally_managed=true\n{raw}")).unwrap();
    instance.ok(&["--allow-local-processes", "start"]);
    assert!(
        !instance
            .output(&[
                "--allow-local-processes",
                "worker",
                "launch",
                "--count",
                "1"
            ])
            .status
            .success()
    );
    let receipt = instance.ok(&["worker", "launch", "--count", "2"]);
    let id = receipt["request_ids"][0].as_str().unwrap();
    assert_eq!(
        instance.ok(&["worker", "requests", "inspect", id])["backend"],
        "external"
    );
    assert_eq!(
        instance.ok(&["worker", "list"]).as_array().unwrap().len(),
        0
    );
    let timed_out = instance.output(&["stop", "--wait", "--timeout", "500ms"]);
    assert!(!timed_out.status.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = instance.ok(&["status"]);
        if status["stop_result"]["phase"] == "cleanup_timeout" {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !instance
            .output(&[
                "run",
                "create",
                instance.card("fenced", 100).to_str().unwrap()
            ])
            .status
            .success()
    );
    let previous = instance.ok(&["status"])["stop_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut stop = instance
        .cmd()
        .args(["stop", "--wait", "--timeout", "15s"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = instance.ok(&["status"]);
        let id = status["stop_id"].as_str().unwrap();
        if id != previous && status["cleanup_required"] == true {
            assert!(
                !instance
                    .output(&["control", "acknowledge-cleanup", &previous])
                    .status
                    .success()
            );
            instance.ok(&["control", "acknowledge-cleanup", id]);
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(stop.wait().unwrap().success());
    instance.wait_stopped();
}

#[test]
#[ignore = "tests persisted exports and recovery after killing a disposable service"]
fn export_recovery_after_service_restart() {
    let instance = Instance::new();
    instance.ok(&["start", "--workers", "2", "--wait"]);
    let destination = instance.root.path().join("result.json");
    let created = instance.ok(&[
        "run",
        "create",
        instance.card("export-recovery", 100).to_str().unwrap(),
        "--export-path",
        destination.to_str().unwrap(),
    ]);
    let id = created["created"]["run_id"].as_i64().unwrap().to_string();
    fs::write(&destination, "do not overwrite").unwrap();
    instance.ok(&["run", "start", &id]);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = instance.ok(&["status"]);
        if status["exports"][0]["error"].is_string() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !instance
            .output(&["stop", "--wait", "--timeout", "5s"])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(&destination).unwrap(),
        "do not overwrite"
    );
    let pid = instance.ok(&["status"])["pid"].as_u64().unwrap();
    // This PID is the disposable service started by this test, never an external deployment.
    assert!(
        Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    std::thread::sleep(Duration::from_millis(300));
    fs::remove_file(&destination).unwrap();
    instance.ok(&["start"]);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !destination.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(100));
    }
    let result: Value = serde_json::from_slice(&fs::read(&destination).unwrap()).unwrap();
    assert_eq!(result["run"]["lifecycle_state"], "completed");
    instance.ok(&["stop", "--wait"]);
    instance.wait_stopped();
}

#[test]
#[ignore = "starts an authenticated disposable service and tests machine credentials"]
fn authenticated_client_and_static_dashboard() {
    use argon2::{
        Argon2,
        password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
    };
    use std::io::Write;
    let instance = Instance::new();
    let hash = Argon2::default()
        .hash_password(b"test-password ", &SaltString::generate(&mut OsRng))
        .unwrap()
        .to_string();
    let mut config = fs::OpenOptions::new()
        .append(true)
        .open(&instance.config)
        .unwrap();
    writeln!(config,"\n[server.auth]\nadmin_password_hash={hash:?}\nsession_secret=\"test-secret-that-is-only-used-in-tests\"").unwrap();
    drop(config);
    // Local bootstrap uses the signing configuration; the remote client has no implicit credential.
    instance.ok(&["start"]);
    assert!(!instance.output(&["status"]).status.success());
    let token = instance.root.path().join("token");
    let definition: toml::Value =
        toml::from_str(&fs::read_to_string(&instance.config).unwrap()).unwrap();
    let connection = instance.root.path().join("client.toml");
    fs::write(
        &connection,
        format!(
            "[client]\nendpoint='http://127.0.0.1:{}'\ntoken_file='token'\n",
            definition["server"]["api_port"].as_integer().unwrap()
        ),
    )
    .unwrap();
    // Login must work when the configured output token does not exist yet.
    let mut login = Command::new(Path::new(env!("CARGO_BIN_EXE_gammaboard")).with_file_name("gb"))
        .args(["--instance", connection.to_str().unwrap(), "--json"])
        .args([
            "auth",
            "login",
            "--password-stdin",
            "--output",
            token.to_str().unwrap(),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    login
        .stdin
        .take()
        .unwrap()
        .write_all(b"test-password \n")
        .unwrap();
    assert!(login.wait().unwrap().success());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&token).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        instance.ok(&["--token-file", token.to_str().unwrap(), "status"])["ready"],
        true
    );
    let redacted =
        instance.ok(&["--token-file", token.to_str().unwrap(), "status"])["configuration"]
            .to_string();
    assert!(!redacted.contains(&hash));
    assert!(!redacted.contains("test-secret-that"));
    let definition: toml::Value =
        toml::from_str(&fs::read_to_string(&instance.config).unwrap()).unwrap();
    let port = definition["server"]["api_port"].as_integer().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    assert_eq!(
        runtime.block_on(async {
            reqwest::get(format!("http://127.0.0.1:{port}/"))
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        }),
        "local instance test"
    );
    runtime.block_on(async {
        let response = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/api/v1/manage"))
            .header("Origin", "http://localhost:39491")
            .json(&serde_json::json!({"operation":"reload_origins"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers()["access-control-allow-origin"],
            "http://localhost:39491"
        );
    });
    instance.ok(&[
        "--token-file",
        token.to_str().unwrap(),
        "config",
        "reload",
        "--origins",
    ]);
    instance.ok(&["--token-file", token.to_str().unwrap(), "stop"]);
    instance.wait_stopped();
}
#[cfg(feature = "symbolica")]
#[test]
#[ignore = "executes the native tetrahedron example with disposable services"]
fn native_tetrahedron_workflow_preserves_1d_plot() {
    let instance = Instance::new();
    instance.ok(&["start", "--workers", "2", "--wait"]);
    let output = instance.root.path().join("tetrahedron.json");
    let card = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources/templates/runs/tetrahedron-native.toml");
    let created = instance.ok(&[
        "run",
        "create",
        card.to_str().unwrap(),
        "--start",
        "--max-evaluators",
        "1",
        "--export-path",
        output.to_str().unwrap(),
    ]);
    let run = created["created"]["run_id"].as_i64().unwrap().to_string();
    instance.ok(&[
        "run",
        "wait",
        &run,
        "--until",
        "completed",
        "--timeout",
        "90s",
    ]);
    let tasks = instance.ok(&["run", "task", "list", &run]);
    let task = tasks
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["name"] == "components-on-line")
        .unwrap();
    let endpoint = instance.ok(&["status"])["api_url"]
        .as_str()
        .unwrap()
        .to_owned();
    let panel: Value = tokio::runtime::Runtime::new().unwrap().block_on(async {
        reqwest::Client::new()
            .post(format!(
                "{endpoint}/runs/{run}/tasks/{}/output",
                task["id"].as_str().unwrap()
            ))
            .json(&serde_json::json!({"section":"result","phase":"current"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    });
    let plot = panel["updates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|update| update["panel"]["panel_id"] == "line_components")
        .expect("completed line task must retain its plotted values");
    let series = plot["panel"]["series"].as_array().unwrap();
    assert_eq!(series.len(), 3);
    for component in series {
        let points = component["points"].as_array().unwrap();
        assert_eq!(points.len(), 1025);
        for point in points {
            let t = point["x"].as_f64().unwrap();
            let x = (8.0 * (2.0 * std::f64::consts::PI * t).cos()).tanh();
            let y = (8.0 * (2.0 * std::f64::consts::PI * t).sin()).tanh();
            let expected = match component["id"].as_str().unwrap() {
                "x" => x,
                "y" => y,
                "z" => x * y,
                _ => panic!("unexpected component"),
            };
            assert!((point["y"].as_f64().unwrap() - expected).abs() < 1e-12);
        }
    }
    instance.ok(&["stop", "--wait", "--timeout", "90s"]);
    instance.wait_stopped();
    let result: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(result["run"]["lifecycle_state"], "completed");
    assert_eq!(result["tasks"].as_array().unwrap().len(), 4);
}

#[test]
#[ignore = "tests queued requests on an externally managed disposable service"]
fn queued_requests_wait_cancel_and_policy() {
    let instance = Instance::new();
    let raw = fs::read_to_string(&instance.config).unwrap();
    fs::write(&instance.config, format!("externally_managed=true\n{raw}")).unwrap();
    instance.ok(&["--allow-local-processes", "start"]);
    let request = instance.ok(&["worker", "launch", "--count", "2"]);
    let id = request["request_ids"][0].as_str().unwrap();
    assert_eq!(
        instance.ok(&["worker", "requests", "inspect", id])["state"],
        "pending"
    );
    let started = Instant::now();
    let waited = instance.output(&["worker", "requests", "wait", id, "--timeout", "150ms"]);
    assert!(!waited.status.success());
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(
        instance.ok(&["worker", "requests", "cancel", id])["state"],
        "canceled"
    );
    instance.ok(&["worker", "requests", "cancel", id]);
    assert_eq!(
        instance.ok(&["worker", "list"]).as_array().unwrap().len(),
        0
    );
    assert!(
        !instance
            .output(&["worker", "requests", "wait", id])
            .status
            .success()
    );
    instance.ok(&["stop"]);
    instance.wait_stopped();
}

#[test]
#[ignore = "starts a disposable service to test atomic starts, retry receipts and archived results"]
fn retry_safe_create_wait_and_archive_workflow() {
    let instance = Instance::new();
    instance.ok(&[
        "start",
        "--workers",
        "1",
        "--wait",
        "--allowed-origin",
        "http://localhost:39491",
    ]);
    assert!(instance.root.path().join("control/resources").is_dir());
    let card = instance.card("retry-safe", 1024);
    let create = [
        "run",
        "create",
        card.to_str().unwrap(),
        "--start",
        "--request-id",
        "retry-safe-001",
        "--timeout",
        "0",
    ];
    let blocked = instance.output(&create);
    assert!(!blocked.status.success());
    let receipt: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(receipt["start"]["state"], "blocked");
    let id = receipt["created"]["run_id"].to_string();
    let repeated = instance.output(&create);
    let replay: Value = serde_json::from_slice(&repeated.stdout).unwrap();
    assert_eq!(receipt["created"], replay["created"]);
    assert_eq!(instance.ok(&["run", "list"]).as_array().unwrap().len(), 1);
    let workers = instance.ok(&["worker", "list"]);
    for worker in workers.as_array().unwrap() {
        assert!(
            worker["pool_run_id"].is_null(),
            "blocked admission took half a group: {worker}"
        );
    }
    let conflict = instance.output(&[
        "run",
        "create",
        card.to_str().unwrap(),
        "--start",
        "--request-id",
        "retry-safe-001",
        "--name",
        "different",
    ]);
    assert!(!conflict.status.success());
    let timed_out = instance.output(&["run", "start", &id, "--timeout", "150ms"]);
    assert!(!timed_out.status.success());
    let timed_out: Value = serde_json::from_slice(&timed_out.stdout).unwrap();
    assert_eq!(timed_out["start"]["state"], "timed_out");
    assert_eq!(timed_out["run_id"].to_string(), id);
    let waiting = instance
        .cmd()
        .args(["run", "start", &id])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    instance.ok(&["worker", "launch", "--count", "1", "--wait"]);
    let output = waiting.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    instance.ok(&["run", "wait", &id, "--timeout", "30s"]);
    let next = instance.card("next", 1024);
    let second = instance.ok(&[
        "run",
        "create",
        next.to_str().unwrap(),
        "--start",
        "--timeout",
        "15s",
    ]);
    let next_id = second["created"]["run_id"].to_string();
    instance.ok(&[
        "run",
        "wait",
        &next_id,
        "--until",
        "completed",
        "--timeout",
        "30s",
    ]);
    let results = instance.root.path().join("results.json");
    instance.ok(&["run", "results", &id, "--output", results.to_str().unwrap()]);
    let imported = instance.ok(&[
        "run",
        "import-results",
        results.to_str().unwrap(),
        "--name",
        "Archive",
    ]);
    let archive = imported["run_id"].to_string();
    assert_eq!(imported["archived"], true);
    assert!(
        !instance
            .output(&["run", "start", &archive])
            .status
            .success()
    );
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(&instance.config).unwrap()).unwrap();
    let endpoint = format!(
        "http://127.0.0.1:{}",
        config["server"]["api_port"].as_integer().unwrap()
    );
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let summary = reqwest::get(format!("{endpoint}/api/runs/{archive}"))
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(summary["archived"], true);
        let catalog = reqwest::get(format!("{endpoint}/api/runs"))
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert!(
            catalog["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["run_id"] == imported["run_id"] && r["archived"] == true)
        );
    });
    let marker = instance.root.path().join("must-not-execute");
    let task = instance.root.path().join("archive-task.toml");
    fs::write(
        &task,
        format!(
            r#"[[task_queue]]
kind="sample"
stop_condition={{max_samples=1}}
sampler={{config={{kind="naive_monte_carlo",seed=1}}}}
accumulator={{config="scalar"}}
evaluator={{config={{kind="process_evaluator",command=["sh","-c","touch {}"]}}}}
"#,
            marker.display()
        ),
    )
    .unwrap();
    let rejected = instance.output(&["run", "task", "append", &archive, task.to_str().unwrap()]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("read-only"));
    assert!(!marker.exists());
    let original: Value = serde_json::from_slice(&fs::read(results).unwrap()).unwrap();
    let copy = instance.ok(&["run", "results", &archive]);
    assert_eq!(copy["run"]["archived"], true);
    assert_eq!(
        original["tasks"][0]["snapshot"]["persisted_output"],
        copy["tasks"][0]["snapshot"]["persisted_output"]
    );
    instance.ok(&["run", "remove", &archive, "--yes"]);
    instance.ok(&["stop", "--wait"]);
    instance.wait_stopped();
}

#[test]
#[ignore = "owns disposable PostgreSQL and service processes"]
fn startup_link_tracks_the_served_dashboard() {
    let instance = Instance::new();
    let raw = fs::read_to_string(&instance.config).unwrap();
    fs::write(
        &instance.config,
        raw.replace("[server]", "[server]\nhost = '0.0.0.0'"),
    )
    .unwrap();
    let started = instance.ok(&["start"]);
    let url = started["dashboard_url"].as_str().unwrap().to_owned();
    assert!(
        url.starts_with("http://127.0.0.1:"),
        "wildcard bind needs a usable browser address"
    );
    assert_eq!(started["api_url"], format!("{}api", url));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let page = runtime.block_on(async { reqwest::get(&url).await.unwrap().text().await.unwrap() });
    assert_eq!(page, "local instance test");
    let human = Command::new(Path::new(env!("CARGO_BIN_EXE_gammaboard")).with_file_name("gb"))
        .arg("--instance")
        .arg(&instance.config)
        .arg("start")
        .output()
        .unwrap();
    assert!(human.status.success());
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(text.contains(&format!("Dashboard: {url}")));
    assert!(
        !text.contains('\x1b'),
        "redirected output must remain plain text"
    );
    assert!(instance.ok(&["restart", "--api-only"])["dashboard_url"].is_null());
    assert!(
        instance.ok(&["start"])["dashboard_url"].is_null(),
        "reuse must report the running mode"
    );
    instance.ok(&["stop"]);
    instance.wait_stopped();
    fs::remove_file(instance.root.path().join("dashboard/index.html")).unwrap();
    let missing = instance.ok(&["start"]);
    assert_eq!(missing["ready"], true);
    assert!(missing["dashboard_url"].is_null());
    assert!(missing["api_url"].is_string());
    instance.ok(&["stop"]);
    instance.wait_stopped();
}

#[test]
fn missing_postgres_tools_are_reported_before_detached_startup() {
    let instance = Instance::configured(false);
    let output = instance
        .cmd()
        .env("PATH", instance.root.path().join("no-tools"))
        .arg("start")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    let message = error["error"]["message"].as_str().unwrap();
    for expected in ["initdb", "pg_ctl", "Install PostgreSQL", "PATH"] {
        assert!(message.contains(expected), "{message}");
    }
    assert!(!message.contains("nix develop"));
    assert!(!instance.root.path().join("postgres").exists());
    assert!(!instance.root.path().join("control/status.json").exists());
}

#[test]
#[ignore = "owns disposable services to test live browser-origin policy reload"]
fn browser_origin_defaults_and_atomic_reload() {
    let instance = Instance::new();
    let raw = fs::read_to_string(&instance.config).unwrap();
    let started = instance.ok(&[
        "start",
        "--workers",
        "2",
        "--wait",
        "--allowed-origin",
        "https://startup.example.org",
    ]);
    assert_eq!(
        started["configuration"]["server"]["allow_loopback_origins"],
        true
    );
    let endpoint = started["api_url"].as_str().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let http = reqwest::Client::new();
    let check = |origin: &str, allowed: bool| {
        runtime.block_on(async {
            let response = http
                .post(format!("{endpoint}/auth/session"))
                .header("Origin", origin)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status().is_success(), allowed, "{origin}");
            assert_eq!(
                response
                    .headers()
                    .get("access-control-allow-origin")
                    .and_then(|v| v.to_str().ok()),
                allowed.then_some(origin)
            );
            let preflight = http
                .request(reqwest::Method::OPTIONS, format!("{endpoint}/v1/manage"))
                .header("Origin", origin)
                .header("Access-Control-Request-Method", "POST")
                .header("Access-Control-Request-Headers", "content-type")
                .send()
                .await
                .unwrap();
            assert_eq!(
                preflight
                    .headers()
                    .get("access-control-allow-origin")
                    .and_then(|v| v.to_str().ok()),
                allowed.then_some(origin)
            );
        });
    };
    for origin in [
        "http://localhost:39491",
        "http://127.0.0.1:45678",
        "https://[::1]:12345",
    ] {
        check(origin, true);
    }
    for origin in [
        "https://untrusted.example",
        "http://localhost.evil.test",
        "null",
    ] {
        check(origin, false);
    }
    check("https://startup.example.org", true);
    let write_policy = |settings: &str| {
        fs::write(
            &instance.config,
            raw.replace("[server]", &format!("[server]\n{settings}")),
        )
        .unwrap();
    };
    write_policy("allow_loopback_origins = false\nallowed_origins = ['https://board.example.org']");
    // Exercise a remote connection: the client does not send the deployment path.
    let reload = || instance.ok(&["--endpoint", endpoint, "config", "reload", "--origins"]);
    let policy = reload();
    assert_eq!(policy["allow_loopback_origins"], false);
    check("http://localhost:39491", false);
    check("https://board.example.org", true);
    check("https://startup.example.org", false);
    let status = instance.ok(&["status"]);
    assert_eq!(
        status["configuration"]["server"]["allowed_origins"],
        policy["allowed_origins"]
    );
    assert_eq!(instance.ok(&["start"])["pid"], started["pid"]);
    assert_eq!(
        instance.ok(&["worker", "list"]).as_array().unwrap().len(),
        2
    );

    write_policy("allow_loopback_origins = true\nallowed_origins = ['*']");
    assert!(
        !instance
            .output(&["config", "reload", "--origins"])
            .status
            .success()
    );
    check("http://localhost:39491", false);
    check("https://board.example.org", true);
    assert_eq!(
        instance.ok(&["status"])["configuration"]["server"]["allow_loopback_origins"],
        false
    );

    write_policy("allow_loopback_origins = true\nallowed_origins = []");
    reload();
    check("http://localhost:39491", true);
    check("https://board.example.org", false);
    assert_eq!(instance.ok(&["start"])["pid"], started["pid"]);
    // An unrelated valid edit is never reported as applied by origin-only reload.
    fs::write(
        &instance.config,
        fs::read_to_string(&instance.config)
            .unwrap()
            .replace("name = \"test\"", "name = \"changed\""),
    )
    .unwrap();
    reload();
    assert_eq!(instance.ok(&["status"])["configuration"]["name"], "test");
    assert!(!instance.output(&["start"]).status.success());
    instance.ok(&["stop"]);
    instance.wait_stopped();
}
