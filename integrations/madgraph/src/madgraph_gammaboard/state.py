from __future__ import annotations

import os
import sys
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal

import numpy as np


@dataclass(frozen=True)
class MadSpaceIntegrand:
    subprocess_index: int
    channel_index: int
    runtime: Any


@dataclass
class MadSpaceState:
    random_dim: int
    integrands: list[MadSpaceIntegrand]

    @property
    def nr_integrands(self) -> int:
        return len(self.integrands)

    @property
    def metadata(self) -> dict[str, Any]:
        return {
            "random_dim": self.random_dim,
            "nr_integrands": self.nr_integrands,
            "integrands": [
                {
                    "index": index,
                    "subprocess_index": integrand.subprocess_index,
                    "channel_index": integrand.channel_index,
                }
                for index, integrand in enumerate(self.integrands)
            ],
        }

    @classmethod
    def load(
        cls,
        *,
        state_path: str,
        madgraph_root: str | None = None,
        subprocess_index: int = 0,
        subprocess_indices: list[int] | Literal["all"] | None = None,
        phase_space: str = "flat",
        channel_index: int = 0,
        channel_indices: list[int] | Literal["all"] | None = None,
        device: str = "cppnone",
        thread_count: int = -1,
        flattened_integrands: bool = False,
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
            selected_subprocesses = _select_indices(
                value=subprocess_indices,
                fallback=subprocess_index,
                upper_bound=len(process.subprocesses),
                label="subprocess",
                allow_all=flattened_integrands,
            )
            runtime_context = _select_context(process, ms, device, thread_count)
            loaded: list[MadSpaceIntegrand] = []
            random_dim: int | None = None

            for selected_subprocess_index in selected_subprocesses:
                subprocess = process.subprocesses[selected_subprocess_index]
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
                selected_channels = _select_indices(
                    value=channel_indices,
                    fallback=channel_index,
                    upper_bound=len(integrands),
                    label=f"channel for subprocess {selected_subprocess_index}",
                    allow_all=flattened_integrands,
                )

                for selected_channel_index in selected_channels:
                    integrand = integrands[selected_channel_index]
                    integrand_random_dim = int(integrand.random_dim())
                    if random_dim is None:
                        random_dim = integrand_random_dim
                    elif integrand_random_dim != random_dim:
                        raise ValueError(
                            "flattened MadSpace integrands must have a common "
                            f"random_dim; got {integrand_random_dim} for "
                            f"subprocess={selected_subprocess_index}, "
                            f"channel={selected_channel_index}, expected {random_dim}"
                        )
                    loaded.append(
                        MadSpaceIntegrand(
                            subprocess_index=selected_subprocess_index,
                            channel_index=selected_channel_index,
                            runtime=ms.FunctionRuntime(
                                integrand.function(), runtime_context
                            ),
                        )
                    )

            if not loaded or random_dim is None:
                raise ValueError("MadSpace state did not expose any integrands")
            if not flattened_integrands and len(loaded) != 1:
                raise ValueError("single-integrand mode loaded more than one integrand")
            return cls(random_dim=random_dim, integrands=loaded)

    def evaluate(
        self, xs: np.ndarray, integrand_indices: np.ndarray | None = None
    ) -> np.ndarray:
        xs = np.asarray(xs, dtype=np.float64)
        if xs.ndim != 2 or xs.shape[1] != self.random_dim:
            raise ValueError(
                f"MadSpace integrand expects xs shape (n, {self.random_dim}), got {xs.shape}"
            )
        if integrand_indices is None:
            if self.nr_integrands != 1:
                raise ValueError(
                    "integrand_indices are required when multiple MadSpace integrands are loaded"
                )
            return self._evaluate_runtime(self.integrands[0].runtime, xs)

        indices = np.asarray(integrand_indices, dtype=np.int64).reshape((xs.shape[0],))
        if ((indices < 0) | (indices >= self.nr_integrands)).any():
            raise ValueError(
                f"integrand index out of bounds for {self.nr_integrands} loaded integrands"
            )

        values = np.empty((xs.shape[0],), dtype=np.float64)
        for integrand_index in np.unique(indices):
            mask = indices == integrand_index
            runtime = self.integrands[int(integrand_index)].runtime
            values[mask] = self._evaluate_runtime(runtime, xs[mask])

        # GammaBoard samples the discrete integrand axis uniformly. Convert the
        # selected summand into an unbiased estimate of the sum over integrands.
        return values * float(self.nr_integrands)

    @staticmethod
    def _evaluate_runtime(runtime: Any, xs: np.ndarray) -> np.ndarray:
        outputs = runtime.call([np.ascontiguousarray(xs)])
        if len(outputs) != 1:
            raise ValueError(
                f"expected one MadSpace integrand output named 'weight', got {len(outputs)}"
            )
        return np.asarray(np.from_dlpack(outputs[0]), dtype=np.float64).reshape(
            (xs.shape[0],)
        )


def _select_indices(
    *,
    value: list[int] | Literal["all"] | None,
    fallback: int,
    upper_bound: int,
    label: str,
    allow_all: bool,
) -> list[int]:
    if upper_bound <= 0:
        raise ValueError(f"MadSpace state exposes no {label} entries")
    if value == "all":
        if not allow_all:
            raise ValueError(f"{label}_indices='all' requires flattened_integrands=true")
        return list(range(upper_bound))
    if isinstance(value, str):
        raise ValueError(f"{label}_indices must be 'all' or a list of integer indices")
    raw_indices = [fallback] if value is None else [int(index) for index in value]
    if not raw_indices:
        raise ValueError(f"{label}_indices must not be empty")
    for index in raw_indices:
        if index < 0 or index >= upper_bound:
            raise ValueError(
                f"{label}_index={index} is out of range for {upper_bound} {label} entries"
            )
    return raw_indices


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
