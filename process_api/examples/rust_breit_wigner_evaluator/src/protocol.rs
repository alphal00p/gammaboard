use std::io::{self, BufRead, Read, Write};

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Request {
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Deserialize)]
pub struct InitializeParams {
    pub protocol: String,
    pub role: String,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub components: Vec<String>,
    pub discrete_cardinalities: Vec<usize>,
    pub continuous_dims: usize,
}

#[derive(Debug, Deserialize)]
pub struct EvalBatchParams {
    pub nr_samples: usize,
    pub xs_discrete_row_major: Vec<i64>,
    pub xs_continuous_row_major: Vec<f64>,
}

pub fn read_request() -> Result<Option<Request>, String> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let Some(content_length) = read_content_length(&mut reader)? else {
        return Ok(None);
    };
    let mut body = vec![0_u8; content_length];
    reader
        .read_exact(&mut body)
        .map_err(|error| format!("failed to read request body: {error}"))?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| error.to_string())
}

pub fn send_result(id: Value, result: Value) -> Result<(), String> {
    send_frame(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
}

pub fn send_error(id: Value, message: &str) -> Result<(), String> {
    send_frame(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32000,
            "message": message,
        },
    }))
}

fn read_content_length(reader: &mut impl BufRead) -> Result<Option<usize>, String> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|error| format!("failed to read header: {error}"))?;
        if bytes == 0 {
            return Ok(None);
        }
        let line = line.trim();
        if line.is_empty() {
            return content_length
                .map(Some)
                .ok_or_else(|| "missing Content-Length header".to_string());
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|error| format!("invalid Content-Length header: {error}"))?,
            );
        }
    }
}

fn send_frame(payload: Value) -> Result<(), String> {
    let body = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write!(stdout, "Content-Length: {}\r\n\r\n", body.len())
        .map_err(|error| format!("failed to write response header: {error}"))?;
    stdout
        .write_all(&body)
        .map_err(|error| format!("failed to write response body: {error}"))?;
    stdout
        .flush()
        .map_err(|error| format!("failed to flush response: {error}"))
}
