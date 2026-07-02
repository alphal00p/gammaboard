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
    external_discrete_dim: int = 0

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
            import madspace as ms
            from madgraph.iolibs.template_files.mg7.madevent import MadgraphProcess

            try:
                process = MadgraphProcess()
            except Exception as error:
                hint = _madgraph_setup_hint(error)
                if hint is None:
                    raise
                raise RuntimeError(f"{error}\n{hint}") from error

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

            original_flavors = subprocess.meta["flavors"]
            selected_flavor = original_flavors[flavor_index]

            try:
                subprocess.meta["flavors"] = [selected_flavor]

                phasespace = subprocess.build_flat_phasespace()
                integrands = subprocess.build_integrands(
                    phasespace,
                    madnis_training=False,
                    drop_cuts_and_rescale=False,
                )
            finally:
                subprocess.meta["flavors"] = original_flavors

            if not integrands:
                raise RuntimeError("MadSpace build_integrands returned no integrands")

            _initialize_madspace_globals(process, subprocess)

            integrand = integrands[0]
            mapping = integrand.mapping()
            diff_xs = integrand.diff_xs()
            random_dim = int(mapping.random_dim())

            if not getattr(process, "contexts", None):
                raise RuntimeError("MadGraph process has no MadSpace contexts")

            batch = ms.BatchSize("batch_size")
            input_types = ms.NamedTypes(
                [
                    ("xs", ms.Type(ms.DataType.float, batch, [random_dim])),
                    ("flavor", ms.Type(ms.DataType.int, batch, [])),
                    ("pdf_id", ms.Type(ms.DataType.int, batch, [])),
                    ("pdf1", ms.Type(ms.DataType.float, batch, [])),
                    ("pdf2", ms.Type(ms.DataType.float, batch, [])),
                    ("alpha_s", ms.Type(ms.DataType.float, batch, [])),
                ]
            )
            output_types = ms.NamedTypes(
                [
                    ("weight", ms.Type(ms.DataType.float, batch, [])),
                ]
            )
            fb = ms.FunctionBuilder(input_types, output_types)

            mapping_out = mapping.build_forward(fb, [fb.input(0)], [])
            dxs_in = ms.NamedValues(
                [
                    ("momenta", mapping_out["momenta"]),
                    ("flavor", fb.input(1)),
                    ("x1", mapping_out["x1"]),
                    ("x2", mapping_out["x2"]),
                    ("pdf_id", fb.input(2)),
                    ("pdf1", fb.input(3)),
                    ("pdf2", fb.input(4)),
                    ("alpha_s", fb.input(5)),
                ]
            )
            dxs_out = diff_xs.build_function(fb, dxs_in)
            fb.output(0, fb.mul(mapping_out["det"], dxs_out["matrix_element"]))

            runtime = ms.FunctionRuntime(fb.function(), process.contexts[0])

            return cls(
                random_dim=random_dim,
                mapping_random_dim=random_dim,
                runtime=runtime,
                subprocess_index=subprocess_index,
                flavor_index=flavor_index,
                external_discrete_dim=0,
            )

    def evaluate(self, xs: np.ndarray) -> np.ndarray:
        xs = np.asarray(xs, dtype=np.float64)

        if xs.ndim != 2 or xs.shape[1] != self.random_dim:
            raise ValueError(
                f"MadSpace expects xs shape (n, {self.random_dim}), got {xs.shape}"
            )

        n = xs.shape[0]
        xs = np.ascontiguousarray(xs, dtype=np.float64)

        flavor = np.zeros((n,), dtype=np.int32)
        pdf_id = np.zeros((n,), dtype=np.int32)

        # Bare phase-space × matrix/cross-section mode.
        # Replace these with real PDF evaluations if you want physical pp luminosity.
        pdf1 = np.ones((n,), dtype=np.float64)
        pdf2 = np.ones((n,), dtype=np.float64)

        # Fixed alpha_s test value. Replace with running alpha_s if desired.
        alpha_s = np.full((n,), 0.118, dtype=np.float64)

        outputs = self.runtime(xs, flavor, pdf_id, pdf1, pdf2, alpha_s)

        if not isinstance(outputs, tuple):
            return np.asarray(outputs, dtype=np.float64).reshape((n,))

        weight = outputs[0]
        return np.asarray(weight, dtype=np.float64).reshape((n,))


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


def _madgraph_setup_hint(error: Exception) -> str | None:
    message = str(error)
    if "Can't load lhapdf module" in message:
        return (
            "Set LHAPDF_DATA_PATH to a user-writable directory containing the "
            "PDF set selected by Cards/run_card.toml, for example "
            "$HOME/.local/share/LHAPDF. Installing PDF data does not require sudo."
        )
    if "cut_efficiency_threshold" in message:
        return (
            "The MadSpace binary does not match this MadGraph checkout. Rebuild "
            "it with `python <madgraph_root>/madspace/install.py --source "
            "--no-cuda --no-hip --no-simd --no-debug`."
        )
    if isinstance(error, KeyError) and "mirror" in message:
        return (
            "The generated MG7 state uses an older schema. Regenerate it with "
            "the pinned MadGraph7 checkout documented in integrations/madgraph/README.md."
        )
    if "/bin/bash: No such file or directory" in message:
        return (
            "MG7 invoked a generated makefile with /bin/bash. Start GammaBoard "
            "from `nix develop`, which supplies a MAKEFLAGS shell override."
        )
    if "compilation Error" in message:
        return (
            "MG7 could not compile its matrix element. Start from `nix develop` "
            "and verify that make, g++, and gfortran are available."
        )
    return None


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


def _initialize_madspace_globals(process: Any, subprocess: Any) -> None:
    contexts = list(getattr(process, "contexts", []) or [])

    single_context = getattr(process, "context", None)
    if single_context is not None and single_context not in contexts:
        contexts.append(single_context)

    objects = [
        ("alphas_grid", getattr(process, "alphas_grid", None)),
        ("pdf_grid", getattr(process, "pdf_grid", None)),
        ("running_coupling", getattr(process, "running_coupling", None)),
        ("scale", getattr(subprocess, "scale", None)),
    ]

    for name, obj in objects:
        if obj is None:
            continue

        init = getattr(obj, "initialize_globals", None)
        if init is None:
            continue

        for context in contexts:
            try:
                init(context)
            except ValueError as exc:
                msg = str(exc)
                if "already contains a global named" in msg:
                    continue
                raise
