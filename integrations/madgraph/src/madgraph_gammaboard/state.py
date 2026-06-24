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
    mapping_random_dim: int
    runtime: Any
    subprocess_index: int
    flavor_index: int

    @property
    def metadata(self) -> dict[str, Any]:
        return {
            "random_dim": self.random_dim,
            "mapping_random_dim": self.mapping_random_dim,
            "subprocess_index": self.subprocess_index,
            "flavor_index": self.flavor_index,
            "phase_space": "flat",
            "integrand": "phase_space_mapping+differential_cross_section",
        }

    @classmethod
    def load(
        cls,
        *,
        state_path: str,
        madgraph_root: str | None = None,
        subprocess_index: int = 0,
        flavor_index: int = 0,
    ) -> "MadSpaceState":
        state_dir = Path(state_path).expanduser().resolve()
        if not state_dir.is_dir():
            raise ValueError(f"MadGraph state_path does not exist: {state_dir}")
        if madgraph_root:
            root = Path(madgraph_root).expanduser().resolve()
            _prepend_sys_path(root)
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
            if flavor_index < 0 or flavor_index >= len(subprocess.meta["flavors"]):
                raise ValueError(
                    f"flavor_index={flavor_index} is out of range for "
                    f"{len(subprocess.meta['flavors'])} flavor option(s)"
                )
            mapping = _build_flat_phase_space_mapping(process, subprocess, ms)
            cross_section = _build_differential_cross_section(
                process, subprocess, flavor_index, ms
            )
            runtime, random_dim = _build_runtime(
                process, subprocess, flavor_index, mapping, cross_section, ms
            )
            return cls(
                random_dim=random_dim,
                mapping_random_dim=int(mapping.random_dim()),
                runtime=runtime,
                subprocess_index=subprocess_index,
                flavor_index=flavor_index,
            )

    def evaluate(self, xs: np.ndarray) -> np.ndarray:
        xs = np.asarray(xs, dtype=np.float64)
        if xs.ndim != 2 or xs.shape[1] != self.random_dim:
            raise ValueError(f"MadSpace expects xs shape (n, {self.random_dim}), got {xs.shape}")
        outputs = self.runtime.call([np.ascontiguousarray(xs)])
        if len(outputs) != 1:
            raise ValueError(
                f"expected one MadSpace output named 'weight', got {len(outputs)}"
            )
        return np.asarray(np.from_dlpack(outputs[0]), dtype=np.float64).reshape(
            (xs.shape[0],)
        )


def _build_flat_phase_space_mapping(process: Any, subprocess: Any, ms: Any) -> Any:
    return ms.PhaseSpaceMapping(
        subprocess.incoming_masses + subprocess.outgoing_masses,
        process.e_cm,
        mode=subprocess.t_channel_mode(process.run_card["phasespace"]["flat_mode"]),
        cuts=subprocess.cuts,
        leptonic=process.leptonic,
    )


def _build_differential_cross_section(
    process: Any, subprocess: Any, flavor_index: int, ms: Any
) -> tuple[Any, int]:
    flavor = subprocess.meta["flavors"][flavor_index]
    flavors = [flavor["options"][0]]
    if subprocess.matrix_element:
        matrix_element = ms.MatrixElement(
            subprocess.matrix_element,
            ms.Integrand.matrix_element_inputs,
            ms.Integrand.matrix_element_outputs,
            True,
        )
    else:
        matrix_element = ms.MatrixElement(
            0xBADCAFE,
            subprocess.particle_count,
            ms.Integrand.matrix_element_inputs,
            ms.Integrand.matrix_element_outputs,
            subprocess.meta["diagram_count"],
            True,
        )
    pdf_grid = None if len(flavors) > 1 or process.leptonic else process.pdf_grid
    return ms.DifferentialCrossSection(
        matrix_element=matrix_element,
        cm_energy=process.e_cm,
        running_coupling=process.running_coupling,
        energy_scale=subprocess.scale,
        pid_options=flavors,
        has_pdf1=not process.leptonic,
        has_pdf2=not process.leptonic,
        pdf_grid1=pdf_grid,
        pdf_grid2=pdf_grid,
        has_mirror=subprocess.meta["has_mirror_process"],
        input_momentum_fraction=True,
    )


def _build_runtime(
    process: Any,
    subprocess: Any,
    flavor_index: int,
    mapping: Any,
    cross_section: Any,
    ms: Any,
) -> Any:
    flavor = subprocess.meta["flavors"][flavor_index]
    flavor_remap = [flavor["index"]]
    flavor_factors = [1]
    integrand = ms.Integrand(
        mapping,
        cross_section,
        None,
        None,
        None,
        process.pdf_grid,
        subprocess.scale,
        None,
        None,
        None,
        [0] * subprocess.meta["diagram_count"],
        1,
        0,
        [0],
        [],
        flavor_remap,
        flavor_factors,
    )
    return ms.FunctionRuntime(integrand.function(), process.contexts[0]), int(
        integrand.random_dim()
    )


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
