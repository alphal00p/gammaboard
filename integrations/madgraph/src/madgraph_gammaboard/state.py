from __future__ import annotations

import os
import sys
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np


@dataclass
class MadSpaceState:
    random_dim: int
    runtime: Any

    @classmethod
    def load(
        cls,
        *,
        state_path: str,
        madgraph_root: str | None = None,
        subprocess_index: int = 0,
        phase_space: str = "flat",
        channel_index: int = 0,
        device: str = "cppnone",
        thread_count: int = -1,
    ) -> "MadSpaceState":
        state_dir = Path(state_path).expanduser().resolve()
        if not state_dir.is_dir():
            raise ValueError(f"MadGraph state_path does not exist: {state_dir}")
        if madgraph_root:
            root = Path(madgraph_root).expanduser().resolve()
            _prepend_sys_path(root)
            # madspace is pre-built in <madgraph_root>/madspace/install/
            madspace_install = root / "madspace" / "install"
            if madspace_install.is_dir():
                _prepend_sys_path(madspace_install)

        _ensure_lhapdf_data_path()

        with _pushd(state_dir):
            from madgraph.iolibs.template_files.mg7.madevent import MadgraphProcess
            import madspace as ms

            process = MadgraphProcess()
            if subprocess_index < 0 or subprocess_index >= len(process.subprocesses):
                raise ValueError(
                    f"subprocess_index={subprocess_index} is out of range for "
                    f"{len(process.subprocesses)} subprocess(es)"
                )

            subprocess = process.subprocesses[subprocess_index]
            if phase_space == "flat":
                phasespace = subprocess.build_flat_phasespace()
            elif phase_space == "multichannel":
                phasespace = subprocess.build_multichannel_phasespace()
            else:
                raise ValueError(
                    "phase_space must be 'flat' or 'multichannel' for direct "
                    "MadSpace integrand evaluation"
                )

            integrands = subprocess.build_integrands(phasespace, 0)
            if channel_index < 0 or channel_index >= len(integrands):
                raise ValueError(
                    f"channel_index={channel_index} is out of range for "
                    f"{len(integrands)} integrand channel(s)"
                )

            integrand = integrands[channel_index]
            runtime_context = _select_context(process, ms, device, thread_count)
            runtime = ms.FunctionRuntime(integrand.function(), runtime_context)
            return cls(random_dim=int(integrand.random_dim()), runtime=runtime)

    def evaluate(self, xs: np.ndarray) -> np.ndarray:
        xs = np.asarray(xs, dtype=np.float64)
        if xs.ndim != 2 or xs.shape[1] != self.random_dim:
            raise ValueError(
                f"MadSpace integrand expects xs shape (n, {self.random_dim}), got {xs.shape}"
            )
        outputs = self.runtime.call([np.ascontiguousarray(xs)])
        if len(outputs) != 1:
            raise ValueError(
                f"expected one MadSpace integrand output named 'weight', got {len(outputs)}"
            )
        return np.asarray(np.from_dlpack(outputs[0]), dtype=np.float64).reshape(
            (xs.shape[0],)
        )


def _select_context(process: Any, ms: Any, device: str, thread_count: int) -> Any:
    if device == "state":
        return process.contexts[0]
    if device.startswith("cuda"):
        _, _, raw_index = device.partition(":")
        return ms.Context(ms.cuda_device(int(raw_index or "0")), thread_count)
    if device.startswith("hip"):
        _, _, raw_index = device.partition(":")
        return ms.Context(ms.hip_device(int(raw_index or "0")), thread_count)

    # Matrix-element libraries in generated MG7 states are selected from the
    # run card. Reusing the state context is safest for C++ backend variants.
    return process.contexts[0]


_LHAPDF_SEARCH_PATHS = [
    "/usr/share/LHAPDF",
    "/usr/local/share/LHAPDF",
]


def _ensure_lhapdf_data_path() -> None:
    if "LHAPDF_DATA_PATH" in os.environ:
        return
    for candidate in _LHAPDF_SEARCH_PATHS:
        if Path(candidate).is_dir():
            os.environ["LHAPDF_DATA_PATH"] = candidate
            return


def _prepend_sys_path(path: Path) -> None:
    value = str(path)
    if value not in sys.path:
        sys.path.insert(0, value)


@contextmanager
def _pushd(path: Path):
    previous = Path.cwd()
    os.chdir(path)
    try:
        yield
    finally:
        os.chdir(previous)
