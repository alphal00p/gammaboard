use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};
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

pub fn read_request(reader: &mut impl BufRead) -> Result<Option<Request>, String> {
    let Some((content_length, binary_length)) = read_frame_lengths(reader)? else {
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

pub fn send_result<T: Serialize>(
    writer: &mut impl Write,
    id: Value,
    result: T,
    binary: &[u8],
) -> Result<(), String> {
    #[derive(Serialize)]
    struct Response<T> {
        jsonrpc: &'static str,
        id: Value,
        result: T,
    }
    send_frame(
        writer,
        Response {
            jsonrpc: "2.0",
            id,
            result,
        },
        binary,
    )
}

pub fn send_error(writer: &mut impl Write, id: Value, message: &str) -> Result<(), String> {
    send_frame(
        writer,
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
            return if content_length.is_none() && binary_length == 0 {
                Ok(None)
            } else {
                Err("truncated frame header".into())
            };
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

fn send_frame(
    stdout: &mut impl Write,
    payload: impl Serialize,
    binary: &[u8],
) -> Result<(), String> {
    let body = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_frames_preserve_binary_boundaries_and_eof() {
        let mut stream = Vec::new();
        for id in 0..2 {
            send_frame(
                &mut stream,
                serde_json::json!({"jsonrpc":"2.0", "id":id, "method":"eval_batch"}),
                b"\r\n\0Content-Length: 1",
            )
            .unwrap();
        }
        let mut input = stream.as_slice();
        for id in 0..2 {
            let request = read_request(&mut input).unwrap().unwrap();
            assert_eq!(request.id, id);
            assert_eq!(request.binary, b"\r\n\0Content-Length: 1");
        }
        assert!(read_request(&mut input).unwrap().is_none());
        stream.pop();
        let mut input = stream.as_slice();
        read_request(&mut input).unwrap();
        assert!(read_request(&mut input).is_err());
        assert!(read_request(&mut b"Content-Length: 10\r\n".as_slice()).is_err());
    }
}
