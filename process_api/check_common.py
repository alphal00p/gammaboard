"""Bounded framed RPC transport shared by the standalone conformance checkers."""

import json
import os
import queue
import signal
import subprocess
import threading
import time
from collections import deque

MAX_FRAME_BYTES = 64 * 1024 * 1024


class CheckError(Exception):
    pass


class Unexercised(CheckError):
    pass


class Unsupported(CheckError):
    pass


class RemoteError(CheckError):
    pass


def exact_read(stream, size):
    chunks = bytearray()
    while len(chunks) < size:
        chunk = stream.read(size - len(chunks))
        if not chunk:
            raise CheckError("worker closed stdout mid-frame")
        chunks.extend(chunk)
    return bytes(chunks)


def read_frame(stream):
    fields = {}
    header_size = 0
    while True:
        line = stream.readline(8193)
        if not line:
            raise CheckError("worker exited without a response")
        header_size += len(line)
        if header_size > 8192:
            raise CheckError(
                "invalid/oversized protocol header; stdout must contain framed RPC only"
            )
        if line in (b"\n", b"\r\n"):
            break
        key, sep, value = line.decode("ascii").partition(":")
        if not sep:
            raise CheckError(
                "invalid protocol header; stdout must contain framed RPC only"
            )
        fields[key.strip().lower()] = int(value.strip())
    length = fields.get("content-length", -1)
    binary_length = fields.get("binary-length", 0)
    if not 0 <= length <= MAX_FRAME_BYTES or not 0 <= binary_length <= MAX_FRAME_BYTES:
        raise CheckError(
            "response frame is missing a length or exceeds the 64 MiB test limit"
        )
    body = json.loads(exact_read(stream, length))
    return body, exact_read(stream, binary_length)


class RpcWorker:
    def __init__(self, options, command, role, initialization):
        if time.monotonic() >= options.deadline:
            raise Unexercised("overall time budget exhausted; increase --time-limit")
        self.options = options
        self.sequence = 0
        self.stderr = deque(maxlen=32)
        self.forced_shutdown = False
        env = {
            **os.environ,
            "PYTHONDONTWRITEBYTECODE": "1",
            "GAMMABOARD_PROCESS_RUNTIME": "command",
            "GAMMABOARD_PROCESS_WORKER": role,
        }
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            bufsize=0,
            env=env,
            cwd=options.cwd,
            start_new_session=os.name == "posix",
        )
        self.stderr_thread = threading.Thread(target=self._drain_stderr, daemon=True)
        self.stderr_thread.start()
        try:
            self.initialized, binary = self.call("initialize", initialization)
            if binary or self.initialized.get("ok") is not True:
                raise CheckError("initialize must return ok=true without binary data")
        except BaseException:
            self.close()
            raise

    def _drain_stderr(self):
        try:
            while chunk := self.process.stderr.read(2048):
                self.stderr.append(chunk.decode("utf-8", errors="replace"))
        except (OSError, ValueError):
            pass

    def call(self, method, params, binary=b""):
        remaining = self.options.deadline - time.monotonic()
        if remaining <= 0:
            raise Unexercised("overall time budget exhausted; increase --time-limit")
        self.sequence += 1
        request_id = self.sequence
        body = json.dumps(
            {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params},
            allow_nan=False,
        ).encode()
        header = f"Content-Length: {len(body)}\r\nBinary-Length: {len(binary)}\r\n\r\n".encode()
        result = queue.Queue(maxsize=1)

        def exchange():
            try:
                data = memoryview(header + body + binary)
                while data:
                    written = self.process.stdin.write(data)
                    if not written:
                        raise CheckError("worker closed stdin")
                    data = data[written:]
                self.process.stdin.flush()
                while True:
                    frame = read_frame(self.process.stdout)
                    envelope, data = frame
                    if envelope.get("method") == "progress" and "id" not in envelope:
                        if (
                            data
                            or envelope.get("jsonrpc") != "2.0"
                            or not isinstance(
                                envelope.get("params", {}).get("activity"), str
                            )
                        ):
                            raise CheckError("invalid progress notification")
                        continue
                    result.put((frame, None))
                    break
            except Exception as exc:  # noqa: BLE001 - propagate any transport failure to the main thread
                result.put((None, exc))

        threading.Thread(target=exchange, daemon=True).start()
        try:
            response, error = result.get(timeout=min(self.options.timeout, remaining))
        except queue.Empty:
            self.close(force=True)
            if remaining <= self.options.timeout:
                raise Unexercised(
                    f"overall time budget exhausted during {method}; increase --time-limit"
                ) from None
            raise CheckError(
                f"{method} timed out; increase --timeout if expected"
            ) from None
        if error:
            self.stderr_thread.join(timeout=0.05)
            detail = "".join(self.stderr)[-2000:].strip()
            raise CheckError(
                f"{method}: {error}" + (f"\n{detail}" if detail else "")
            ) from error
        envelope, data = response
        if envelope.get("id") != request_id or envelope.get("jsonrpc") != "2.0":
            raise CheckError("RPC response has the wrong id/version")
        if ("error" in envelope) == ("result" in envelope):
            raise CheckError("RPC response must have exactly one result or error")
        if "error" in envelope:
            failure = envelope["error"]
            message = failure.get("message", str(failure))
            if self.options.verbose and failure.get("data"):
                message += "\n" + str(failure["data"])
            raise RemoteError(message)
        if not isinstance(envelope.get("result"), dict):
            raise CheckError("RPC result must be an object")
        return envelope["result"], data

    def close(self, force=False):
        if self.process.poll() is None and not force:
            self.process.stdin.close()
            try:
                self.process.wait(timeout=min(self.options.timeout, 3))
            except subprocess.TimeoutExpired:
                force = True
        if force and self.process.poll() is None:
            self.forced_shutdown = True
            try:
                if os.name == "posix":
                    os.killpg(self.process.pid, signal.SIGKILL)
                else:
                    self.process.kill()
            except ProcessLookupError:
                pass
            self.process.wait()
        self.stderr_thread.join(timeout=0.1)
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            try:
                stream.close()
            except OSError:
                pass

    def __enter__(self):
        return self

    def __exit__(self, exc_type, *_):
        self.close()
        if exc_type is None and (self.forced_shutdown or self.process.returncode != 0):
            raise CheckError("worker did not shut down cleanly after stdin closed")
