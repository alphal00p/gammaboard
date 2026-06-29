use std::io::{self, BufRead, Read, Write};

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Request {
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    #[serde(skip)]
    pub binary: Vec<u8>,
}

#[derive(Debug, Deserialize)]
pub struct InitializeParams {
    pub protocol: String,
    pub role: String,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default)]
    pub domain: Value,
}

#[derive(Debug, Deserialize)]
pub struct EvalBatchParams {
    pub nr_samples: usize,
    #[serde(default)]
    pub xs_discrete_offsets: Option<Vec<usize>>,
    #[serde(default)]
    pub xs_continuous_offsets: Option<Vec<usize>>,
}

pub fn read_request() -> Result<Option<Request>, String> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let Some((content_length, binary_length)) = read_frame_lengths(&mut reader)? else {
        return Ok(None);
    };
    let mut body = vec![0_u8; content_length];
    reader
        .read_exact(&mut body)
        .map_err(|error| format!("failed to read request body: {error}"))?;
    let mut request: Request = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    request.binary = vec![0_u8; binary_length];
    reader
        .read_exact(&mut request.binary)
        .map_err(|error| format!("failed to read request binary block: {error}"))?;
    Ok(Some(request))
}

pub fn send_result(id: Value, result: Value, binary: &[u8]) -> Result<(), String> {
    send_frame(
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }),
        binary,
    )
}

pub fn send_error(id: Value, message: &str) -> Result<(), String> {
    send_frame(
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32000,
                "message": message,
            },
        }),
        &[],
    )
}

fn read_frame_lengths(reader: &mut impl BufRead) -> Result<Option<(usize, usize)>, String> {
    let mut content_length = None;
    let mut binary_length = 0;
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
                .map(|content_length| Some((content_length, binary_length)))
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
        } else if name.eq_ignore_ascii_case("binary-length") {
            binary_length = value
                .trim()
                .parse::<usize>()
                .map_err(|error| format!("invalid Binary-Length header: {error}"))?;
        }
    }
}

fn send_frame(payload: Value, binary: &[u8]) -> Result<(), String> {
    let body = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write!(stdout, "Content-Length: {}\r\n", body.len())
        .map_err(|error| format!("failed to write response header: {error}"))?;
    if !binary.is_empty() {
        write!(stdout, "Binary-Length: {}\r\n", binary.len())
            .map_err(|error| format!("failed to write response header: {error}"))?;
    }
    stdout
        .write_all(b"\r\n")
        .map_err(|error| format!("failed to write response header: {error}"))?;
    stdout
        .write_all(&body)
        .map_err(|error| format!("failed to write response body: {error}"))?;
    stdout
        .write_all(binary)
        .map_err(|error| format!("failed to write response binary block: {error}"))?;
    stdout
        .flush()
        .map_err(|error| format!("failed to flush response: {error}"))
}
