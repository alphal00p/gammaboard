"""Structured logging for GammaBoard process workers.

The protocol channel (framed JSON-RPC) owns the worker's real stdout, so logs
travel on stderr. `log()` writes sentinel-framed lines that the GammaBoard host
parses and re-emits at the matching log level; plain `print()` is rerouted here
at INFO. Non-sentinel stderr (tracebacks, native libraries) stays unstructured
and is recorded by the host at WARN.
"""

from __future__ import annotations

import os
import sys
import threading

# Must match SENTINEL in src/process_worker.rs.
_SENTINEL = "@gblog"
_LEVELS = ("trace", "debug", "info", "warn", "error")
_ALIASES = {"warning": "warn", "err": "error"}
_RANK = {name: rank for rank, name in enumerate(_LEVELS)}

_lock = threading.Lock()


def _normalize_level(level: str) -> str:
    name = _ALIASES.get(str(level).lower(), str(level).lower())
    return name if name in _RANK else "info"


def _worker_threshold() -> int:
    raw = os.environ.get("GAMMABOARD_LOG_LEVEL", "info")
    return _RANK.get(_normalize_level(raw), _RANK["info"])


def log(message: object, *, level: str = "info") -> None:
    """Emit a structured log line to the GammaBoard host (and runtime log DB).

    `level` is one of trace, debug, info, warn, error (default info). Lines
    below `GAMMABOARD_LOG_LEVEL` (default info) are dropped in the worker; the
    server additionally drops anything below its `db_gammaboard_level`.
    """
    name = _normalize_level(level)
    if _RANK[name] < _worker_threshold():
        return
    stream = sys.__stderr__
    if stream is None:
        return
    with _lock:
        for line in str(message).split("\n"):
            stream.write(f"{_SENTINEL}\t{name}\t{line}\n")
        stream.flush()


class _PrintToLog:
    """Line-buffered file-like shim that routes `print()` to `log()`."""

    def __init__(self, level: str = "info") -> None:
        self._level = level
        self._buffer = ""

    def write(self, data: str) -> int:
        self._buffer += data
        while "\n" in self._buffer:
            line, self._buffer = self._buffer.split("\n", 1)
            log(line, level=self._level)
        return len(data)

    def flush(self) -> None:
        if self._buffer:
            log(self._buffer, level=self._level)
            self._buffer = ""

    def isatty(self) -> bool:
        return False


def install_print_redirect(level: str = "info") -> None:
    """Route `sys.stdout` (i.e. `print`) to the host log at `level`."""
    sys.stdout = _PrintToLog(level)
