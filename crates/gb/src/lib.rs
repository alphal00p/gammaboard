//! Lightweight HTTP client. PostgreSQL and computation libraries belong to the service.
pub mod cli;
pub mod protocol;
use anyhow::{Context, Result, ensure};
use protocol::Request;
use serde_json::Value;
use std::{path::Path, time::Duration};

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    endpoint: String,
    token: Option<String>,
}
impl Client {
    pub fn new(endpoint: &str, token: Option<String>) -> Result<Self> {
        let url = reqwest::Url::parse(endpoint).context("invalid API endpoint")?;
        ensure!(
            matches!(url.scheme(), "http" | "https"),
            "API endpoint must use HTTP(S)"
        );
        ensure!(
            url.username().is_empty() && url.password().is_none(),
            "use a token file, not credentials in the endpoint"
        );
        ensure!(
            url.query().is_none() && url.fragment().is_none(),
            "API endpoint cannot contain a query or fragment"
        );
        Ok(Self {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()?,
            endpoint: endpoint
                .trim_end_matches('/')
                .trim_end_matches("/api")
                .to_owned(),
            token,
        })
    }
    pub async fn request(&self, request: &Request) -> Result<Value> {
        let timeout = match request {
            Request::RunPerformance {
                duration_ms: Some(ms),
                ..
            } => Duration::from_millis(*ms) + Duration::from_secs(30),
            _ => Duration::from_secs(30),
        };
        self.request_with_timeout(request, timeout).await
    }
    pub async fn request_with_timeout(
        &self,
        request: &Request,
        timeout: Duration,
    ) -> Result<Value> {
        let mut call = self
            .http
            .post(format!("{}/api/v1/manage", self.endpoint))
            .timeout(timeout)
            .json(request);
        if let Some(token) = &self.token {
            call = call.bearer_auth(token);
        }
        let response = call
            .send()
            .await
            .context("cannot reach GammaBoard service")?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .context("failed reading GammaBoard response")?;
        let value = serde_json::from_slice::<Value>(&body);
        if !status.is_success() {
            let message = value
                .as_ref()
                .map(|v| v.get("error").unwrap_or(v).to_string())
                .unwrap_or_else(|_| String::from_utf8_lossy(&body).into_owned());
            anyhow::bail!("HTTP {status}: {message}");
        }
        value.context("invalid GammaBoard response")
    }
    pub async fn login(&self, password: &str) -> Result<String> {
        let response = self
            .http
            .post(format!("{}/api/auth/login", self.endpoint))
            .timeout(Duration::from_secs(30))
            .json(&serde_json::json!({"password":password}))
            .send()
            .await?;
        let status = response.status();
        let value: Value = response.json().await?;
        ensure!(status.is_success(), "login failed: {value}");
        Ok(value["token"]
            .as_str()
            .context("server authentication is disabled; no token is needed")?
            .to_owned())
    }
    pub async fn download(&self, path: &str, output: &Path) -> Result<()> {
        use std::io::Write;
        let mut request = self
            .http
            .post(format!("{}{path}", self.endpoint))
            .timeout(Duration::from_secs(24 * 60 * 60));
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let mut response = request.send().await?;
        ensure!(
            response.status().is_success(),
            "download failed: {}",
            response.text().await?
        );
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        while let Some(bytes) = response.chunk().await? {
            file.write_all(&bytes)?;
        }
        file.as_file().sync_all()?;
        file.persist_noclobber(output).map_err(|e| e.error)?;
        Ok(())
    }
    pub async fn wait_for_requests(&self, ids: &[String], timeout: Duration) -> Result<Value> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let mut requests = Vec::new();
            for id in ids {
                let remaining = deadline
                    .checked_duration_since(std::time::Instant::now())
                    .context(format!(
                        "timed out waiting; requests remain recorded: {ids:?}"
                    ))?;
                let value = self
                    .request_with_timeout(
                        &Request::WorkerRequests {
                            id: Some(id.parse()?),
                        },
                        remaining.min(Duration::from_secs(30)),
                    )
                    .await
                    .with_context(|| format!("waiting for request {id}; work is not canceled"))?;
                ensure!(
                    !matches!(value["state"].as_str(), Some("failed" | "canceled")),
                    "request {id} {}: {}",
                    value["state"],
                    value["error"]
                );
                requests.push(value);
            }
            if requests.iter().all(|r| r["state"] == "fulfilled") {
                return Ok(serde_json::json!({"requests":requests}));
            }
            ensure!(
                std::time::Instant::now() < deadline,
                "timed out waiting; requests remain recorded: {ids:?}"
            );
            tokio::time::sleep(
                Duration::from_millis(100)
                    .min(deadline.saturating_duration_since(std::time::Instant::now())),
            )
            .await;
        }
    }
    pub async fn wait_for_stop(&self, id: &str, timeout: Option<Duration>) -> Result<Value> {
        let started = std::time::Instant::now();
        loop {
            let remaining = timeout
                .map(|limit| {
                    limit
                        .checked_sub(started.elapsed())
                        .context("shutdown wait timed out; request remains active")
                })
                .transpose()?
                .unwrap_or(Duration::from_secs(30));
            let status = self
                .request_with_timeout(&Request::Status {}, remaining.min(Duration::from_secs(30)))
                .await
                .context("shutdown completion was not confirmed; inspect the deployment owner")?;
            ensure!(
                status["stop_id"] == id,
                "shutdown request was superseded; inspect status"
            );
            if let Some(success) = status["stop_result"]["success"].as_bool() {
                ensure!(
                    success,
                    "shutdown did not complete: {}",
                    status["stop_result"]
                );
                return Ok(status);
            }
            ensure!(
                timeout.is_none_or(|t| started.elapsed() < t),
                "timed out waiting; shutdown remains requested: {id}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

/// Connection metadata is resolved before opening any credential file.
#[derive(Debug, Default, serde::Serialize)]
pub struct ConnectionConfig {
    pub endpoint: String,
    pub token_file: Option<std::path::PathBuf>,
}
impl ConnectionConfig {
    pub fn validate(&self) -> Result<()> {
        Client::new(&self.endpoint, None)?;
        if let Some(path) = &self.token_file {
            ensure!(
                path.is_file(),
                "token file does not exist: {} (auth login can create it)",
                path.display()
            );
        }
        Ok(())
    }
    pub fn client(&self, authenticate: bool) -> Result<Client> {
        let token = if authenticate {
            self.token_file
                .as_ref()
                .map(|p| {
                    std::fs::read_to_string(p)
                        .with_context(|| format!("cannot read token file {}", p.display()))
                        .map(|s| s.trim().to_owned())
                })
                .transpose()?
        } else {
            None
        };
        Client::new(&self.endpoint, token)
    }
}
pub fn connection(path: &Path) -> Result<ConnectionConfig> {
    let value: toml::Value = toml::from_str(
        &std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?,
    )?;
    let endpoint = if let Some(client) = value.get("client") {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            endpoint: Option<String>,
            token_file: Option<String>,
        }
        let fields: Fields = client.clone().try_into()?;
        return Ok(ConnectionConfig {
            endpoint: fields.endpoint.unwrap_or_default(),
            token_file: fields
                .token_file
                .map(|name| path.parent().unwrap_or(Path::new(".")).join(name)),
        });
    } else {
        let server = value
            .get("server")
            .context("configuration needs [client] or [server]")?;
        #[derive(serde::Deserialize)]
        struct Address {
            host: Option<String>,
            api_host: Option<String>,
            port: Option<u16>,
            api_port: Option<u16>,
        }
        let address: Address = server.clone().try_into()?;
        let host = address
            .host
            .or(address.api_host)
            .unwrap_or_else(|| "127.0.0.1".into());
        let host = match host.as_str() {
            "0.0.0.0" => "127.0.0.1",
            "::" => "::1",
            host => host,
        };
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.into()
        };
        let port = address.port.or(address.api_port).unwrap_or(4000);
        format!("http://{host}:{port}")
    };
    Ok(ConnectionConfig {
        endpoint,
        token_file: None,
    })
}

/// A foreground child has already emitted its diagnostics; preserve its exit code.
#[derive(Debug)]
pub struct ChildExit(pub u8);
impl std::fmt::Display for ChildExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "service exited with status {}", self.0)
    }
}
impl std::error::Error for ChildExit {}
#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    #[test]
    fn cli_command_tree_is_consistent() {
        cli::Cli::command().debug_assert();
    }
    #[test]
    fn connection_file_needs_no_database_or_server_paths() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("token"), "private-token\n").unwrap();
        let path = root.path().join("client.toml");
        std::fs::write(
            &path,
            "[client]\nendpoint='https://example.org:4444'\ntoken_file='token'\n",
        )
        .unwrap();
        let connection = connection(&path).unwrap();
        assert_eq!(connection.endpoint, "https://example.org:4444");
        assert_eq!(connection.token_file, Some(root.path().join("token")));
        connection.validate().unwrap();
    }
    #[test]
    fn local_outputs_do_not_overwrite_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("result");
        cli::write_output(&path, b"first").unwrap();
        assert!(cli::write_output(&path, b"second").is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"first");
    }
    #[test]
    fn browser_task_identifiers_keep_bigint_precision() {
        let request:Request=serde_json::from_value(serde_json::json!({"operation":"task","run":"1","action":{"action":"remove","id":"9007199254740993"}})).unwrap();
        assert!(matches!(
            request,
            Request::Task {
                action: protocol::TaskAction::Remove {
                    id: 9007199254740993
                },
                ..
            }
        ));
    }
    #[test]
    fn protocol_rejects_unknown_fields_and_operations() {
        assert!(
            serde_json::from_value::<Request>(
                serde_json::json!({"operation":"status","allow_local_processes":true})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<Request>(
                serde_json::json!({"operation":"arbitrary_shell","command":"anything"})
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod consistency_tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn credential_overrides_are_resolved_before_reading() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("client.toml");
        std::fs::write(
            &path,
            "[client]\nendpoint='https://old.example'\ntoken_file='missing'\n",
        )
        .unwrap();
        let token = root.path().join("new.token");
        std::fs::write(&token, "new-secret").unwrap();
        let cli = cli::Cli::parse_from([
            "gb",
            "--instance",
            path.to_str().unwrap(),
            "--endpoint",
            "https://new.example",
            "--token-file",
            token.to_str().unwrap(),
            "status",
        ]);
        let config = cli.connection().unwrap();
        assert_eq!(config.endpoint, "https://new.example");
        config.validate().unwrap();
        config.client(true).unwrap();
        let cli = cli::Cli::parse_from([
            "gb",
            "--instance",
            path.to_str().unwrap(),
            "auth",
            "login",
            "--password-stdin",
            "--output",
            token.to_str().unwrap(),
        ]);
        assert!(
            cli.connection().unwrap().client(false).is_ok(),
            "login must not read the missing old token"
        );
    }
    #[test]
    fn worker_waiting_and_local_scopes_are_explicit() {
        for args in [
            vec!["gb", "worker", "launch", "--local", "2"],
            vec!["gb", "worker", "launch", "--count", "2", "--timeout", "1s"],
            vec!["gb", "restart", "server"],
            vec!["gb", "db", "restore", "file"],
            vec!["gb", "run", "validate", "file", "--probe"],
            vec!["gb", "--quiet", "status"],
        ] {
            assert!(cli::Cli::try_parse_from(&args).is_err(), "{args:?}");
        }
        for args in [
            vec![
                "gb",
                "worker",
                "launch",
                "--count",
                "2",
                "--wait",
                "--timeout",
                "1s",
            ],
            vec!["gb", "local", "db", "restore", "file", "--yes"],
            vec!["gb", "worker", "requests", "cancel", "42"],
            vec!["gb", "start", "--workers", "2", "--wait"],
        ] {
            cli::Cli::try_parse_from(&args).unwrap();
        }
    }
    #[test]
    fn consolidated_commands_have_one_advertised_entry_point() {
        use clap::CommandFactory;
        for args in [
            vec!["gb", "db", "status"],
            vec!["gb", "local", "db", "start"],
            vec!["gb", "run", "export", "1"],
        ] {
            assert!(cli::Cli::try_parse_from(args).is_err());
        }
        for args in [
            vec!["gb", "start", "db"],
            vec!["gb", "local", "config", "validate"],
            vec!["gb", "local", "run", "validate", "card.toml", "--probe"],
            vec!["gb", "run", "export", "1", "--kind", "results"],
            vec!["gb", "worker", "list", "--state", "all"],
            vec!["gb", "worker", "list", "--offline"],
        ] {
            cli::Cli::try_parse_from(args).unwrap();
        }
        let mut tree = cli::Cli::command();
        let help = tree
            .find_subcommand_mut("local")
            .unwrap()
            .render_help()
            .to_string();
        assert!(!help.contains("check-run") && !help.contains("check-config"));
        let help = tree
            .find_subcommand_mut("run")
            .unwrap()
            .find_subcommand_mut("task")
            .unwrap()
            .render_help()
            .to_string();
        assert!(!help.contains("inspect"));
    }
    #[test]
    fn launch_sources_are_explicit_and_do_not_mix_contracts() {
        for source in [
            serde_json::json!({"kind":"new","count":2}),
            serde_json::json!({"kind":"saved","name":"worker","expected_uuid":"previous","restore_only":true}),
            serde_json::json!({"kind":"restore_saved"}),
        ] {
            serde_json::from_value::<Request>(
                serde_json::json!({"operation":"worker_launch","source":source}),
            )
            .unwrap();
        }
        for request in [
            serde_json::json!({"operation":"worker_launch","count":2}),
            serde_json::json!({"operation":"worker_launch","source":{"kind":"saved","name":"worker","count":2}}),
            serde_json::json!({"operation":"database_status"}),
        ] {
            assert!(serde_json::from_value::<Request>(request).is_err());
        }
    }
    #[test]
    fn invalid_connection_configuration_is_rejected_offline() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("client.toml");
        for raw in [
            "[client]\nendpoint=42",
            "[client]\nendpoint='http://example.org'\nendpont='typo'",
            "[server]\napi_port='not a port'",
        ] {
            std::fs::write(&path, raw).unwrap();
            assert!(connection(&path).is_err());
        }
        assert!(Client::new("postgres://localhost/board", None).is_err());
    }
    #[tokio::test]
    async fn polling_deadline_bounds_a_stalled_http_response() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            let _ = stream.read(&mut bytes);
            std::thread::sleep(Duration::from_millis(500));
        });
        let client = Client::new(&endpoint, None).unwrap();
        let started = std::time::Instant::now();
        assert!(
            client
                .wait_for_requests(&["1".into()], Duration::from_millis(100))
                .await
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_millis(450));
        server.join().unwrap();
    }
}
