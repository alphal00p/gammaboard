use std::collections::VecDeque;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::{Arc, Mutex};

use serde_json::Value;

pub(crate) const PROCESS_PROTOCOL: &str = "gammaboard-jsonrpc-v1";
const JSON_RPC_VERSION: &str = "2.0";
const MAX_STDOUT_LOG_BYTES_BEFORE_FRAME: usize = 64 * 1024;
const MAX_STDERR_TAIL_LINES: usize = 40;

pub(crate) type ProcessStderrTail = Arc<Mutex<VecDeque<String>>>;

pub(crate) struct ProcessWorker {
    label: String,
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    stdout_log_bytes_before_frame: usize,
    stderr_tail: ProcessStderrTail,
}

impl ProcessWorker {
    pub(crate) fn new(
        label: impl Into<String>,
        child: Child,
        stdin: ChildStdin,
        stdout: ChildStdout,
        stderr_tail: ProcessStderrTail,
    ) -> Self {
        Self {
            label: label.into(),
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            next_id: 1,
            stdout_log_bytes_before_frame: 0,
            stderr_tail,
        }
    }

    pub(crate) fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.allocate_request_id();
        let request = serde_json::json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_frame(&request)?;
        let response = self.read_response(id)?;
        if let Some(error) = response.get("error") {
            return Err(format_json_rpc_error(error));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| format!("{} response missing 'result'", self.label))
    }

    fn write_frame(&mut self, value: &Value) -> Result<(), String> {
        let payload = serde_json::to_vec(value)
            .map_err(|error| format!("failed to serialize {} request: {error}", self.label))?;
        write!(self.stdin, "Content-Length: {}\r\n\r\n", payload.len())
            .map_err(|error| format!("failed writing {} frame header: {error}", self.label))?;
        self.stdin
            .write_all(&payload)
            .map_err(|error| format!("failed writing {} frame payload: {error}", self.label))?;
        self.stdin
            .flush()
            .map_err(|error| format!("failed flushing {} request: {error}", self.label))
    }

    fn read_response(&mut self, expected_id: u64) -> Result<Value, String> {
        let Some(content_len) = self.read_frame_header()? else {
            return Err(self.worker_terminated_message(&format!(
                "{} worker terminated before responding",
                self.label
            )));
        };
        let mut payload = vec![0_u8; content_len];
        self.stdout.read_exact(&mut payload).map_err(|error| {
            self.worker_terminated_message(&format!(
                "failed reading {} frame payload: {error}",
                self.label
            ))
        })?;
        let response = serde_json::from_slice::<Value>(&payload).map_err(|error| {
            format!(
                "failed to parse {} response frame as JSON: {error}; payload='{}'",
                self.label,
                String::from_utf8_lossy(&payload)
            )
        })?;
        validate_response_envelope(&self.label, &response, expected_id)?;
        Ok(response)
    }

    fn read_frame_header(&mut self) -> Result<Option<usize>, String> {
        loop {
            let mut line = String::new();
            let read = self
                .stdout
                .read_line(&mut line)
                .map_err(|error| format!("failed reading {} frame header: {error}", self.label))?;
            if read == 0 {
                return Ok(None);
            }
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
            let content_len = value.trim().parse::<usize>().map_err(|error| {
                format!(
                    "invalid {} Content-Length header value {:?}: {error}",
                    self.label,
                    value.trim()
                )
            })?;
            loop {
                let mut header_line = String::new();
                let read = self.stdout.read_line(&mut header_line).map_err(|error| {
                    format!(
                        "failed reading {} frame header continuation: {error}",
                        self.label
                    )
                })?;
                if read == 0 {
                    return Err(self.worker_terminated_message(&format!(
                        "{} worker terminated inside frame header",
                        self.label
                    )));
                }
                if header_line == "\r\n" || header_line == "\n" {
                    break;
                }
            }
            return Ok(Some(content_len));
        }
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
            .child
            .try_wait()
            .ok()
            .flatten()
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
}

impl Drop for ProcessWorker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

pub(crate) fn pipe_process_stderr(
    label: &'static str,
    stderr: impl std::io::Read + Send + 'static,
) -> ProcessStderrTail {
    let tail = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_STDERR_TAIL_LINES)));
    let tail_for_thread = Arc::clone(&tail);
    // Capture the caller's span (carries run_id/node_name/node_uuid) and re-enter
    // it on the detached reader thread, which otherwise has no span context.
    let context_span = tracing::Span::current();
    std::thread::spawn(move || {
        let _entered = context_span.enter();
        let reader = std::io::BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let display = emit_worker_stderr_line(label, &line);
            if let Ok(mut lines) = tail_for_thread.lock() {
                if lines.len() == MAX_STDERR_TAIL_LINES {
                    lines.pop_front();
                }
                lines.push_back(display);
            }
        }
    });
    tail
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
        JSON_RPC_VERSION, emit_worker_stderr_line, format_json_rpc_error,
        validate_response_envelope,
    };
    use serde_json::json;

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
}
