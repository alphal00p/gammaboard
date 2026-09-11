use std::collections::VecDeque;
use std::io::{BufRead, BufReader, BufWriter, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

pub(crate) const PROCESS_PROTOCOL: &str = "gammaboard-jsonrpc-v2";
const JSON_RPC_VERSION: &str = "2.0";
const MAX_STDOUT_LOG_BYTES_BEFORE_FRAME: usize = 64 * 1024;
const MAX_STDERR_TAIL_LINES: usize = 40;
const MAX_FRAME_HEADER_LINE_BYTES: usize = 8 * 1024;
const MAX_JSON_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_BINARY_FRAME_BYTES: usize = 256 * 1024 * 1024;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_SHUTDOWN_GRACE_SECONDS: u64 = 30;
const DEFAULT_TERMINATE_GRACE: Duration = Duration::from_secs(5);
const EXIT_STATUS_GRACE: Duration = Duration::from_millis(50);
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) type ProcessStderrTail = Arc<Mutex<VecDeque<String>>>;

pub(crate) struct ProcessWorker {
    label: String,
    diagnostic_context: String,
    stderr_log_path: Option<std::path::PathBuf>,
    activity: Option<crate::runners::activity::Handle>,
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    stdout_log_bytes_before_frame: usize,
    stderr_tail: ProcessStderrTail,
    request_timeout: Duration,
    shutdown_grace: Duration,
    terminate_grace: Duration,
}

impl ProcessWorker {
    pub(crate) fn spawn(
        command: &mut std::process::Command,
        label: &'static str,
        grace: u64,
    ) -> Result<Self, crate::core::BuildError> {
        use crate::core::BuildError;
        use std::process::Stdio;
        let context = format!(
            "executable={} cwd={}",
            command.get_program().to_string_lossy(),
            command
                .get_current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ".".into())
        );
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|e| {
            BuildError::build(redact_diagnostic(&format!(
                "{label}: operation=spawn {context}; {}; {e}",
                current_worker_context()
            )))
        })?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let log_path = crate::resources::primary_resource_root().ok().map(|root| {
            let dir = std::path::absolute(root.join("logs/processes"))
                .unwrap_or_else(|_| root.join("logs/processes"));
            let _ = std::fs::create_dir_all(&dir);
            dir.join(format!(
                "{}-{}.stderr.log",
                child.id(),
                chrono::Utc::now().timestamp_micros()
            ))
        });
        let tail = pipe_process_stderr_to_file(label, stderr, log_path.clone());
        let mut worker = Self::new(label, child, stdin, stdout, tail, grace);
        worker.diagnostic_context = redact_diagnostic(&context);
        worker.stderr_log_path = log_path;
        Ok(worker)
    }

    pub(crate) fn new(
        label: impl Into<String>,
        child: Child,
        stdin: ChildStdin,
        stdout: ChildStdout,
        stderr_tail: ProcessStderrTail,
        shutdown_grace_seconds: u64,
    ) -> Self {
        Self {
            label: label.into(),
            diagnostic_context: String::new(),
            stderr_log_path: None,
            activity: crate::runners::activity::current(),
            child,
            stdin: Some(BufWriter::new(stdin)),
            stdout: BufReader::new(stdout),
            next_id: 1,
            stdout_log_bytes_before_frame: 0,
            stderr_tail,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            shutdown_grace: Duration::from_secs(shutdown_grace_seconds),
            terminate_grace: DEFAULT_TERMINATE_GRACE,
        }
    }

    pub(crate) fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let (result, _binary) = self.request_with_binary(method, params, &[])?;
        Ok(result)
    }

    /// Like `request`, but attaches `binary` to the request frame and returns the
    /// response JSON `result` together with its (possibly empty) binary block.
    pub(crate) fn request_with_binary(
        &mut self,
        method: &str,
        params: Value,
        binary: &[u8],
    ) -> Result<(Value, Vec<u8>), String> {
        let _activity = crate::runners::activity::PhaseGuard::enter(
            self.activity.clone(),
            if self.label.contains("sampler") {
                "waiting for sampler response"
            } else {
                "waiting for process response"
            },
        );
        let outcome = (|| {
            let id = self.allocate_request_id();
            let request = serde_json::json!({
                "jsonrpc": JSON_RPC_VERSION,
                "id": id,
                "method": method,
                "params": params,
            });
            self.write_frame(&request, binary)?;
            let deadline = Instant::now() + self.request_timeout;
            let (response, response_binary) = self.read_response(id, deadline)?;
            if let Some(error) = response.get("error") {
                return Err(format_json_rpc_error(error));
            }
            let result = response
                .get("result")
                .cloned()
                .ok_or_else(|| format!("{} response missing 'result'", self.label))?;
            Ok((result, response_binary))
        })();
        outcome.map_err(|error:String| {
            let status=self.child.try_wait().ok().flatten().map(|s|s.to_string()).unwrap_or_else(|| "running or unavailable".into());
            let context=self.activity.as_ref().map(crate::runners::activity::snapshot).unwrap_or_default();
            let log=self.stderr_log_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "unavailable".into());
            let tail=if error.contains("recent stderr:") { String::new() } else { self.stderr_tail() };
            redact_diagnostic(&format!("{}: operation={method} {}; worker={} run={} task={} status={status}; stderr_log={log}; {error}; recent stderr:\n{tail}",self.label,self.diagnostic_context,context["node_name"],context["run_id"],context["task_id"]))
        })
    }

    fn write_frame(&mut self, value: &Value, binary: &[u8]) -> Result<(), String> {
        let label = self.label.clone();
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| format!("{label} worker stdin is closed"))?;
        let payload = serde_json::to_vec(value)
            .map_err(|error| format!("failed to serialize {label} request: {error}"))?;
        write!(stdin, "Content-Length: {}\r\n", payload.len())
            .map_err(|error| format!("failed writing {label} frame header: {error}"))?;
        if !binary.is_empty() {
            write!(stdin, "Binary-Length: {}\r\n", binary.len())
                .map_err(|error| format!("failed writing {label} frame header: {error}"))?;
        }
        stdin
            .write_all(b"\r\n")
            .map_err(|error| format!("failed writing {label} frame header: {error}"))?;
        stdin
            .write_all(&payload)
            .map_err(|error| format!("failed writing {label} frame payload: {error}"))?;
        if !binary.is_empty() {
            stdin
                .write_all(binary)
                .map_err(|error| format!("failed writing {label} binary payload: {error}"))?;
        }
        stdin
            .flush()
            .map_err(|error| format!("failed flushing {label} request: {error}"))
    }

    fn read_response(
        &mut self,
        expected_id: u64,
        deadline: Instant,
    ) -> Result<(Value, Vec<u8>), String> {
        loop {
            let Some((content_len, binary_len)) = self.read_frame_header(deadline)? else {
                return Err(self.worker_terminated_message(&format!(
                    "{} worker terminated before responding",
                    self.label
                )));
            };
            let payload = self.read_frame_bytes(content_len, "frame payload", deadline)?;
            let binary = self.read_frame_bytes(binary_len, "binary payload", deadline)?;
            let response = serde_json::from_slice::<Value>(&payload).map_err(|error| {
                format!(
                    "failed to parse {} response frame as JSON: {error}; payload='{}'",
                    self.label,
                    bounded_diagnostic(&String::from_utf8_lossy(&payload), 1024)
                )
            })?;
            if response.get("id").is_none()
                && response.get("method").and_then(Value::as_str) == Some("progress")
            {
                let phase = response["params"]["activity"]
                    .as_str()
                    .ok_or("progress notification requires params.activity")?;
                crate::runners::activity::report_progress(self.activity.as_ref(), phase)?;
                continue;
            }
            validate_response_envelope(&self.label, &response, expected_id)?;
            return Ok((response, binary));
        }
    }

    /// Reads a frame header block, returning `(content_length, binary_length)`.
    /// `Binary-Length` is optional (absent means 0).
    fn read_frame_header(&mut self, deadline: Instant) -> Result<Option<(usize, usize)>, String> {
        loop {
            let Some(line) = self.read_header_line("frame header", deadline)? else {
                return Ok(None);
            };
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                continue;
            }
            let Some((name, value)) = trimmed.split_once(':') else {
                self.log_stdout_text(trimmed)?;
                continue;
            };
            if !name.eq_ignore_ascii_case("Content-Length") {
                self.log_stdout_text(trimmed)?;
                continue;
            }
            let content_len = self.parse_header_len("Content-Length", value)?;
            let mut binary_len = 0usize;
            loop {
                let Some(header_line) =
                    self.read_header_line("frame header continuation", deadline)?
                else {
                    return Err(self.worker_terminated_message(&format!(
                        "{} worker terminated inside frame header",
                        self.label
                    )));
                };
                let trimmed = header_line.trim_end_matches(['\r', '\n']);
                if trimmed.is_empty() {
                    break;
                }
                if let Some((name, value)) = trimmed.split_once(':')
                    && name.eq_ignore_ascii_case("Binary-Length")
                {
                    binary_len = self.parse_header_len("Binary-Length", value)?;
                }
            }
            return Ok(Some((content_len, binary_len)));
        }
    }

    fn parse_header_len(&self, header: &str, value: &str) -> Result<usize, String> {
        let parsed = value.trim().parse::<usize>().map_err(|error| {
            format!(
                "invalid {} {header} header value {:?}: {error}",
                self.label,
                value.trim()
            )
        })?;
        let limit = if header == "Content-Length" {
            MAX_JSON_FRAME_BYTES
        } else {
            MAX_BINARY_FRAME_BYTES
        };
        if parsed > limit {
            return Err(format!(
                "{} {header}={parsed} exceeds protocol limit of {limit} bytes",
                self.label
            ));
        }
        Ok(parsed)
    }

    fn read_header_line(
        &mut self,
        context: &str,
        deadline: Instant,
    ) -> Result<Option<String>, String> {
        let mut bytes = Vec::with_capacity(MAX_FRAME_HEADER_LINE_BYTES.min(256));
        loop {
            self.wait_for_stdout(deadline)?;
            let available = self
                .stdout
                .fill_buf()
                .map_err(|error| format!("failed reading {} {context}: {error}", self.label))?;
            if available.is_empty() {
                return if bytes.is_empty() {
                    Ok(None)
                } else {
                    Err(self.worker_terminated_message(&format!(
                        "{} worker terminated inside {context}",
                        self.label
                    )))
                };
            }
            let remaining = MAX_FRAME_HEADER_LINE_BYTES.saturating_sub(bytes.len());
            let newline = available.iter().position(|byte| *byte == b'\n');
            let take = newline.map_or(available.len().min(remaining), |index| index + 1);
            if take > remaining {
                return Err(format!(
                    "{} {context} exceeds protocol limit of {} bytes",
                    self.label, MAX_FRAME_HEADER_LINE_BYTES
                ));
            }
            bytes.extend_from_slice(&available[..take]);
            self.stdout.consume(take);
            if newline.is_some() {
                break;
            }
        }
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|error| format!("{} {context} is not valid UTF-8: {error}", self.label))
    }

    fn read_frame_bytes(
        &mut self,
        len: usize,
        context: &str,
        deadline: Instant,
    ) -> Result<Vec<u8>, String> {
        let mut bytes = vec![0_u8; len];
        let mut offset = 0;
        while offset < len {
            self.wait_for_stdout(deadline)?;
            let available = self
                .stdout
                .fill_buf()
                .map_err(|error| format!("failed reading {} {context}: {error}", self.label))?;
            if available.is_empty() {
                return Err(self.worker_terminated_message(&format!(
                    "{} worker terminated inside {context}",
                    self.label
                )));
            }
            let count = available.len().min(len - offset);
            bytes[offset..offset + count].copy_from_slice(&available[..count]);
            self.stdout.consume(count);
            offset += count;
        }
        Ok(bytes)
    }

    fn wait_for_stdout(&mut self, deadline: Instant) -> Result<(), String> {
        if !self.stdout.buffer().is_empty() {
            return Ok(());
        }
        #[cfg(unix)]
        {
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(self.request_timeout_message());
                }
                let timeout_ms = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
                let mut descriptor = libc::pollfd {
                    fd: self.stdout.get_ref().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // `descriptor` points to a valid ChildStdout file descriptor for this call.
                let result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
                if result > 0 {
                    return Ok(());
                }
                if result == 0 {
                    return Err(self.request_timeout_message());
                }
                if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                    return Err(self.worker_terminated_message(&format!(
                        "failed waiting for {} worker response",
                        self.label
                    )));
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = deadline;
            Ok(())
        }
    }

    fn request_timeout_message(&mut self) -> String {
        self.force_kill();
        let status = self
            .child
            .wait()
            .map(|status| status.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let stderr_tail = self.stderr_tail();
        let mut message = format!(
            "{} worker request timed out after {} seconds; status={status}",
            self.label,
            self.request_timeout.as_secs_f64(),
        );
        if !stderr_tail.is_empty() {
            message.push_str("; recent stderr:\n");
            message.push_str(&stderr_tail);
        }
        message
    }

    fn log_stdout_text(&mut self, line: &str) -> Result<(), String> {
        self.stdout_log_bytes_before_frame = self
            .stdout_log_bytes_before_frame
            .saturating_add(line.len())
            .saturating_add(1);
        if self.stdout_log_bytes_before_frame > MAX_STDOUT_LOG_BYTES_BEFORE_FRAME {
            return Err(format!(
                "{} worker wrote more than {} bytes of non-protocol stdout before a frame; write logs to stderr",
                self.label, MAX_STDOUT_LOG_BYTES_BEFORE_FRAME
            ));
        }
        eprintln!("[{} stdout] {}", self.label, line);
        Ok(())
    }

    fn allocate_request_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub(crate) fn worker_terminated_message(&mut self, context: &str) -> String {
        let status = self
            .wait_for_exit(EXIT_STATUS_GRACE)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "running".to_string());
        let stderr_tail = self.stderr_tail();
        if stderr_tail.is_empty() {
            format!("{context}; status={status}")
        } else {
            format!("{context}; status={status}; recent stderr:\n{stderr_tail}")
        }
    }

    fn stderr_tail(&self) -> String {
        let Ok(lines) = self.stderr_tail.lock() else {
            return "<stderr tail unavailable: lock poisoned>".to_string();
        };
        lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(
                EXIT_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    #[cfg(unix)]
    fn signal_process(&mut self, signal: libc::c_int) {
        let pid = self.child.id() as libc::pid_t;
        // Production workers are process-group leaders. Retain a direct-child
        // fallback for tests and for a failed platform-specific group setup.
        let is_group_leader = unsafe { libc::getpgid(pid) } == pid;
        if !is_group_leader || unsafe { libc::kill(-pid, signal) } != 0 {
            let _ = unsafe { libc::kill(pid, signal) };
        }
    }

    #[cfg(not(unix))]
    fn signal_process(&mut self, _signal: i32) {
        let _ = self.child.kill();
    }

    fn force_kill(&mut self) {
        #[cfg(unix)]
        self.signal_process(libc::SIGKILL);
        #[cfg(not(unix))]
        self.signal_process(0);
    }

    fn shutdown(&mut self) {
        // EOF is the process protocol's graceful shutdown signal.
        drop(self.stdin.take());
        if self.wait_for_exit(self.shutdown_grace).is_some() {
            return;
        }
        #[cfg(unix)]
        self.signal_process(libc::SIGTERM);
        #[cfg(not(unix))]
        self.signal_process(0);
        if self.wait_for_exit(self.terminate_grace).is_some() {
            return;
        }
        self.force_kill();
        let _ = self.child.wait();
    }
}

pub(crate) const fn default_process_shutdown_grace_seconds() -> u64 {
    DEFAULT_SHUTDOWN_GRACE_SECONDS
}

impl Drop for ProcessWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn format_json_rpc_error(error: &Value) -> String {
    if let Some(message) = error.get("message").and_then(Value::as_str) {
        if let Some(data) = error.get("data")
            && let Some(traceback) = data.get("traceback").and_then(Value::as_str)
            && !traceback.is_empty()
        {
            return format!("process worker error: {message}\n{traceback}");
        }
        return format!("process worker error: {message}");
    }
    format!("process worker error: {error}")
}

/// Append `values` as little-endian `f64` bytes to `out`.
pub(crate) fn extend_le_f64(out: &mut Vec<u8>, values: &[f64]) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

/// Read `count` little-endian `f64`s from `bytes` starting at `byte_offset`,
/// returning the values and the next byte offset.
pub(crate) fn read_le_f64(
    bytes: &[u8],
    byte_offset: usize,
    count: usize,
) -> Result<(Vec<f64>, usize), String> {
    let end = checked_binary_end(byte_offset, count)?;
    let slice = bytes.get(byte_offset..end).ok_or_else(|| {
        format!(
            "binary block too short: need {end} bytes, have {}",
            bytes.len()
        )
    })?;
    let values = slice
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| f64::from_le_bytes(*chunk))
        .collect();
    Ok((values, end))
}

/// Read `count` little-endian `i64`s from `bytes` starting at `byte_offset`,
/// returning the values and the next byte offset.
pub(crate) fn read_le_i64(
    bytes: &[u8],
    byte_offset: usize,
    count: usize,
) -> Result<(Vec<i64>, usize), String> {
    let end = checked_binary_end(byte_offset, count)?;
    let slice = bytes.get(byte_offset..end).ok_or_else(|| {
        format!(
            "binary block too short: need {end} bytes, have {}",
            bytes.len()
        )
    })?;
    let values = slice
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| i64::from_le_bytes(*chunk))
        .collect();
    Ok((values, end))
}

fn checked_binary_end(byte_offset: usize, count: usize) -> Result<usize, String> {
    let byte_len = count
        .checked_mul(8)
        .ok_or_else(|| "binary element count is too large".to_string())?;
    byte_offset
        .checked_add(byte_len)
        .ok_or_else(|| "binary byte offset is too large".to_string())
}

#[cfg(test)]
pub(crate) fn pipe_process_stderr(
    label: &'static str,
    stderr: impl std::io::Read + Send + 'static,
) -> ProcessStderrTail {
    pipe_process_stderr_to_file(label, stderr, None)
}

fn pipe_process_stderr_to_file(
    label: &'static str,
    stderr: impl std::io::Read + Send + 'static,
    log_path: Option<std::path::PathBuf>,
) -> ProcessStderrTail {
    let tail = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_STDERR_TAIL_LINES)));
    let tail_for_thread = Arc::clone(&tail);
    let context_span = tracing::Span::current();
    std::thread::spawn(move || {
        let _entered = context_span.enter();
        let mut file = log_path
            .as_ref()
            .and_then(|p| std::fs::File::create(p).ok());
        let mut reader = BufReader::new(stderr);
        let mut continuing_oversized_line = false;
        loop {
            // A worker can print an unlimited line; never allocate it in memory.
            let mut bytes = Vec::new();
            let count = std::io::Read::take(&mut reader, 4096)
                .read_until(b'\n', &mut bytes)
                .unwrap_or(0);
            if count == 0 {
                break;
            }
            let oversized = !bytes.ends_with(b"\n") && bytes.len() == 4096;
            let suppress = oversized || continuing_oversized_line;
            continuing_oversized_line = oversized;
            let line = if suppress {
                "[oversized stderr line omitted]\n".into()
            } else {
                redact_diagnostic(&String::from_utf8_lossy(&bytes))
            };
            if let Some(file) = &mut file {
                let _ = file.write_all(line.as_bytes());
            }
            let display = emit_worker_stderr_line(label, line.trim_end());
            if let Ok(mut lines) = tail_for_thread.lock() {
                if lines.len() == MAX_STDERR_TAIL_LINES {
                    lines.pop_front();
                }
                lines.push_back(display);
                while lines.iter().map(String::len).sum::<usize>() > 8192 {
                    lines.pop_front();
                }
            }
        }
    });
    tail
}

fn current_worker_context() -> String {
    crate::runners::activity::current()
        .map(|h| {
            let a = crate::runners::activity::snapshot(&h);
            format!(
                "worker={} run={} task={}",
                a["node_name"], a["run_id"], a["task_id"]
            )
        })
        .unwrap_or_else(|| "worker=validation".into())
}

fn bounded_diagnostic(value: &str, max: usize) -> String {
    let redacted = redact_diagnostic(value);
    let mut result = redacted.chars().take(max).collect::<String>();
    if redacted.chars().count() > max {
        result.push_str("… [truncated]");
    }
    redact_diagnostic(&result)
}

fn redact_diagnostic(value: &str) -> String {
    let mut result = value.to_string();
    for (key, value) in std::env::vars() {
        let key = key.to_ascii_lowercase();
        if value.len() >= 4
            && [
                "license", "password", "token", "secret", "api_key", "apikey",
            ]
            .iter()
            .any(|word| key.contains(word))
        {
            result = result.replace(&value, "[REDACTED]");
        }
    }
    for key in [
        "password",
        "token",
        "secret",
        "license",
        "api_key",
        "apikey",
        "authorization",
    ] {
        let mut start = 0;
        while let Some(offset) = result[start..].to_ascii_lowercase().find(key) {
            let end = start + offset + key.len();
            let rest = &result[end..];
            let skipped = rest.len() - rest.trim_start_matches(['\"', '\'', ' ', '\t']).len();
            let delimiter = end + skipped;
            if matches!(result.as_bytes().get(delimiter), Some(b'=') | Some(b':')) {
                let begin = delimiter + 1;
                let rest = &result[begin..];
                let skip = rest.len() - rest.trim_start_matches(['\"', '\'', ' ', '\t']).len();
                let begin = begin + skip;
                let finish = result[begin..]
                    .find(|c: char| c.is_whitespace() || matches!(c, '\"' | '\'' | ',' | '}'))
                    .map(|i| begin + i)
                    .unwrap_or(result.len());
                let finish = if key == "authorization"
                    && matches!(
                        &result[begin..finish].to_ascii_lowercase()[..],
                        "bearer" | "basic"
                    ) {
                    let tail = &result[finish..];
                    let next = finish + tail.len() - tail.trim_start().len();
                    result[next..]
                        .find(|c: char| c.is_whitespace() || matches!(c, '\"' | '\'' | ',' | '}'))
                        .map(|i| next + i)
                        .unwrap_or(result.len())
                } else {
                    finish
                };
                if finish > begin {
                    result.replace_range(begin..finish, "[REDACTED]");
                    start = begin + 10;
                    continue;
                }
            }
            start = end;
        }
    }
    // User-info in URLs is never useful diagnostic context.
    let words = result
        .split_whitespace()
        .map(|word| {
            if let Some(scheme) = word.find("://")
                && let Some(at) = word[scheme + 3..].find('@')
            {
                return format!(
                    "{}[REDACTED]@{}",
                    &word[..scheme + 3],
                    &word[scheme + 3 + at + 1..]
                );
            }
            word.to_string()
        })
        .collect::<Vec<_>>();
    if result.contains("://") {
        words.join(" ")
    } else {
        result
    }
}

/// Sentinel prefix for structured worker logs on stderr. Must match `_SENTINEL`
/// in process_api/python/src/gammaboard_process/log.py.
const WORKER_LOG_SENTINEL: &str = "@gblog\t";

/// Re-emit one worker stderr line through `tracing` and return the text to keep
/// in the stderr tail. Structured `@gblog\t<level>\t<message>` lines are emitted
/// at the matching level; everything else is unstructured worker output at warn.
fn emit_worker_stderr_line(label: &str, line: &str) -> String {
    if let Some(rest) = line.strip_prefix(WORKER_LOG_SENTINEL) {
        let (level, text) = rest.split_once('\t').unwrap_or(("info", rest));
        let message = format!("[{label}] {text}");
        match level {
            "trace" => tracing::trace!(source = "worker", message = message.clone()),
            "debug" => tracing::debug!(source = "worker", message = message.clone()),
            "warn" => tracing::warn!(source = "worker", message = message.clone()),
            "error" => tracing::error!(source = "worker", message = message.clone()),
            _ => tracing::info!(source = "worker", message = message.clone()),
        }
        message
    } else {
        let message = format!("[{label} stderr] {line}");
        tracing::warn!(source = "worker", message = message.clone());
        message
    }
}

fn validate_response_envelope(
    label: &str,
    response: &Value,
    expected_id: u64,
) -> Result<(), String> {
    if !response.is_object() {
        return Err(format!(
            "{label} response frame must be a JSON object, got {response}"
        ));
    }
    let jsonrpc = response
        .get("jsonrpc")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} response missing string 'jsonrpc'"))?;
    if jsonrpc != JSON_RPC_VERSION {
        return Err(format!(
            "{label} response uses unsupported jsonrpc version: expected {JSON_RPC_VERSION:?}, got {jsonrpc:?}",
        ));
    }
    let actual_id = response
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{label} response missing numeric 'id'"))?;
    if actual_id != expected_id {
        return Err(format!(
            "{label} worker returned mismatched response id: expected {expected_id}, got {actual_id}",
        ));
    }
    if response.get("result").is_some() == response.get("error").is_some() {
        return Err(format!(
            "{label} response must contain exactly one of 'result' or 'error'",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        JSON_RPC_VERSION, ProcessWorker, emit_worker_stderr_line, format_json_rpc_error,
        pipe_process_stderr, read_le_f64, read_le_i64, validate_response_envelope,
    };
    use serde_json::json;
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    #[test]
    fn structured_worker_log_is_unwrapped_for_tail() {
        assert_eq!(
            emit_worker_stderr_line("evaluator", "@gblog\tinfo\thello"),
            "[evaluator] hello"
        );
        assert_eq!(
            emit_worker_stderr_line("evaluator", "@gblog\terror\tboom"),
            "[evaluator] boom"
        );
    }

    #[test]
    fn unstructured_worker_stderr_keeps_stderr_prefix() {
        assert_eq!(
            emit_worker_stderr_line("sampler", "Traceback (most recent call last):"),
            "[sampler stderr] Traceback (most recent call last):"
        );
    }

    #[test]
    fn json_rpc_error_prefers_traceback_data() {
        let error = json!({
            "code": -32000,
            "message": "ValueError: bad input",
            "data": { "traceback": "Traceback line" },
        });

        assert_eq!(
            format_json_rpc_error(&error),
            "process worker error: ValueError: bad input\nTraceback line"
        );
    }

    #[test]
    fn protocol_version_constant_matches_json_rpc_2() {
        assert_eq!(JSON_RPC_VERSION, "2.0");
    }

    #[test]
    fn validates_json_rpc_response_envelope() {
        validate_response_envelope(
            "test worker",
            &json!({"jsonrpc": "2.0", "id": 3, "result": {"ok": true}}),
            3,
        )
        .expect("valid response should pass");

        assert!(
            validate_response_envelope(
                "test worker",
                &json!({"jsonrpc": "2.0", "id": 3, "result": {}, "error": {}}),
                3,
            )
            .expect_err("result+error should fail")
            .contains("exactly one")
        );
    }

    #[cfg(unix)]
    #[test]
    fn progress_notifications_are_consumed_before_the_response() {
        let progress =
            json!({"jsonrpc":"2.0","method":"progress","params":{"activity":"updating sampler"}})
                .to_string();
        let response = json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}}).to_string();
        let script = format!(
            "printf 'Content-Length: {}\\r\\n\\r\\n{}Content-Length: {}\\r\\n\\r\\n{}'; cat >/dev/null",
            progress.len(),
            progress,
            response.len(),
            response
        );
        let (mut worker, _) = test_worker(
            &script,
            Duration::from_millis(20),
            Duration::from_millis(20),
        );
        assert_eq!(worker.request("sample", json!({})).unwrap()["ok"], true);
    }

    #[test]
    fn credential_values_are_redacted_from_diagnostics() {
        let text = super::redact_diagnostic(
            "password=abc123 token: xyz987 Authorization: Bearer hidden-bearer https://user:pass@example.org/path",
        );
        for secret in ["abc123", "xyz987", "user:pass", "hidden-bearer"] {
            assert!(!text.contains(secret));
        }
        assert!(text.contains("example.org/path"));
    }

    #[cfg(unix)]
    #[test]
    fn missing_executable_reports_command_workdir_and_operation() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new("gammaboard-definitely-missing-executable");
        command.current_dir(dir.path());
        let error = match ProcessWorker::spawn(&mut command, "process sampler", 0) {
            Ok(_) => panic!("unexpected executable"),
            Err(e) => e.to_string(),
        };
        assert!(error.contains("operation=spawn"));
        assert!(error.contains("gammaboard-definitely-missing-executable"));
        assert!(error.contains(&dir.path().display().to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn crash_reports_exit_and_bounded_stderr() {
        let (mut worker, _) = test_worker(
            "head -c 200000 /dev/zero | tr '\\000' x >&2; echo ' token=do-not-expose' >&2; exit 23",
            Duration::from_millis(20),
            Duration::from_millis(20),
        );
        let error = worker.request("initialize", json!({})).unwrap_err();
        assert!(error.contains("23"));
        assert!(error.contains("operation=initialize"));
        assert!(!error.contains("do-not-expose"));
        assert!(
            error.len() < 18000,
            "diagnostic exceeded bounded tail: {}",
            error.len()
        );
    }

    #[cfg(unix)]
    #[test]
    fn malformed_response_includes_operation_and_does_not_panic() {
        let (mut worker, _) = test_worker(
            "printf 'Content-Length: 5\\r\\n\\r\\nbogus'; cat >/dev/null",
            Duration::from_millis(20),
            Duration::from_millis(20),
        );
        let error = worker.request("sample", json!({})).unwrap_err();
        assert!(error.contains("operation=sample"));
        assert!(error.contains("parse"));
    }

    #[test]
    fn binary_decoders_reject_overflowing_layouts() {
        assert!(
            read_le_f64(&[], usize::MAX, 1)
                .expect_err("overflow must fail")
                .contains("offset")
        );
        assert!(
            read_le_i64(&[], 0, usize::MAX)
                .expect_err("overflow must fail")
                .contains("count")
        );
    }

    #[cfg(unix)]
    #[test]
    fn stalled_worker_request_times_out_and_is_terminated() {
        let (mut worker, _) = test_worker(
            "read _; sleep 5",
            Duration::from_millis(20),
            Duration::from_millis(20),
        );
        worker.request_timeout = Duration::from_millis(20);

        assert!(
            worker
                .request("initialize", json!({}))
                .expect_err("stalled request must fail")
                .contains("timed out")
        );
    }

    #[cfg(unix)]
    fn test_worker(
        command: &str,
        shutdown_grace: Duration,
        terminate_grace: Duration,
    ) -> (ProcessWorker, u32) {
        let mut child = Command::new("sh")
            .args(["-c", command])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .expect("spawn test worker");
        let pid = child.id();
        let stdin = child.stdin.take().expect("worker stdin");
        let stdout = child.stdout.take().expect("worker stdout");
        let stderr = child.stderr.take().expect("worker stderr");
        let mut worker = ProcessWorker::new(
            "test worker",
            child,
            stdin,
            stdout,
            pipe_process_stderr("test worker", stderr),
            shutdown_grace.as_secs(),
        );
        worker.request_timeout = Duration::from_secs(1);
        worker.shutdown_grace = shutdown_grace;
        worker.terminate_grace = terminate_grace;
        (worker, pid)
    }

    #[cfg(unix)]
    #[test]
    fn worker_shutdown_closes_stdin_and_reaps_clean_exit() {
        let (worker, pid) = test_worker(
            "while read -r _; do :; done",
            Duration::from_secs(1),
            Duration::from_millis(20),
        );

        drop(worker);

        assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, -1);
    }

    #[cfg(unix)]
    #[test]
    fn worker_shutdown_escalates_after_bounded_grace_periods() {
        let (worker, pid) = test_worker(
            "trap '' TERM; while :; do :; done",
            Duration::from_millis(20),
            Duration::from_millis(20),
        );
        let started = Instant::now();

        drop(worker);

        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, -1);
    }
}
