use crate::core::{BuildError, EvalError, ObservableConfig};
use crate::evaluation::{
    Batch, BatchResult, ComplexBatchEvaluator, EvalBatchOptions, Evaluator, ObservableState,
    ScalarBatchEvaluator,
};
use crate::utils::domain::Domain;
use num::complex::Complex64;
use pyo3::prelude::*;
use pyo3::types::PyModule;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PythonScalarParams {
    pub flake_ref: String,
    pub module: String,
    pub class: String,
    pub input_dim: usize,
}

impl Default for PythonScalarParams {
    fn default() -> Self {
        Self {
            flake_ref: "path:./integrand_api/examples/python_scalar_sin#runtime".to_string(),
            module: "demo_integrand".to_string(),
            class: "SinIntegrand".to_string(),
            input_dim: 1,
        }
    }
}

pub struct ScalarPythonEvaluator {
    domain: Domain,
    input_dim: usize,
    integrand: Py<PyAny>,
}

impl ScalarPythonEvaluator {
    pub fn new(input_dim: usize, integrand: Py<PyAny>) -> Self {
        Self {
            domain: Domain::continuous(input_dim),
            input_dim,
            integrand,
        }
    }

    pub fn from_params(params: PythonScalarParams) -> Result<Self, BuildError> {
        let flake_out = resolve_flake_output_path(&normalize_flake_ref(&params.flake_ref))?;
        let module_name = params.module.clone();
        let class_name = params.class.clone();
        let module_root = flake_out.display().to_string();
        let site_packages = collect_site_packages(&flake_out)
            .into_iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        let integrand = with_python_build(|py| {
            let sys = PyModule::import(py, "sys")
                .map_err(|err| BuildError::build(format!("failed importing python sys: {err}")))?;
            let sys_path = sys
                .getattr("path")
                .map_err(|err| BuildError::build(format!("failed reading sys.path: {err}")))?;
            sys_path
                .call_method1("insert", (0, module_root.as_str()))
                .map_err(|err| BuildError::build(format!("failed adding module root to sys.path: {err}")))?;
            for site in &site_packages {
                sys_path.call_method1("append", (site.as_str(),)).map_err(|err| {
                    BuildError::build(format!("failed adding site-packages to sys.path: {err}"))
                })?;
            }
            let module = PyModule::import(py, module_name.as_str()).map_err(|err| {
                BuildError::build(format!(
                    "failed importing python module '{}': {err}",
                    module_name
                ))
            })?;
            let class = module.getattr(class_name.as_str()).map_err(|err| {
                BuildError::build(format!(
                    "failed reading python class '{}.{}': {err}",
                    module_name, class_name
                ))
            })?;
            class
                .call0()
                .map(|instance| instance.unbind())
                .map_err(|err| {
                    BuildError::build(format!(
                        "failed constructing python integrand '{}.{}': {err}",
                        module_name, class_name
                    ))
                })
        })?;
        Ok(Self::new(params.input_dim, integrand))
    }
}

fn normalize_flake_ref(flake_ref: &str) -> String {
    if flake_ref.starts_with("path:") {
        return flake_ref.to_string();
    }
    let Some((path_part, attr_part)) = flake_ref.rsplit_once('#') else {
        return flake_ref.to_string();
    };
    if path_part.starts_with("./")
        || path_part.starts_with("../")
        || path_part.starts_with('/')
        || path_part.starts_with("~")
    {
        return format!("path:{path_part}#{attr_part}");
    }
    flake_ref.to_string()
}

impl ScalarBatchEvaluator for ScalarPythonEvaluator {
    fn input_dim(&self) -> usize {
        self.input_dim
    }

    fn eval_scalar_dense_batch(
        &mut self,
        xs_row_major: &[f64],
        nr_samples: usize,
    ) -> Result<Vec<f64>, EvalError> {
        with_python_eval(|py| {
            let np = PyModule::import(py, "numpy")
                .map_err(|err| EvalError::eval(format!("failed importing numpy: {err}")))?;
            let xs = np
                .call_method1("array", (xs_row_major,))
                .and_then(|value| value.call_method1("reshape", (nr_samples, self.input_dim)))
                .map_err(|err| EvalError::eval(format!("failed preparing numpy input batch: {err}")))?;
            let values = self
                .integrand
                .bind(py)
                .call_method1("eval", (xs,))
                .map_err(|err| EvalError::eval(format!("python integrand eval() failed: {err}")))?;
            let values = np
                .call_method1("asarray", (values,))
                .and_then(|value| value.call_method1("reshape", (nr_samples,)))
                .map_err(|err| EvalError::eval(format!("failed normalizing scalar output shape: {err}")))?;
            values
                .call_method0("tolist")
                .and_then(|value| value.extract::<Vec<f64>>())
                .map_err(|err| EvalError::eval(format!("failed extracting scalar batch outputs: {err}")))
        })
    }
}

impl Evaluator for ScalarPythonEvaluator {
    fn get_domain(&self) -> Domain {
        self.domain.clone()
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable: &ObservableConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let mut observable_state = ObservableState::from_config(observable);
        let weighted_values = match &mut observable_state {
            ObservableState::Scalar(observable) => {
                self.eval_scalar_batch_into(batch, observable, options.require_training_values)?
            }
            ObservableState::FullScalar(observable) => {
                self.eval_scalar_batch_into(batch, observable, options.require_training_values)?
            }
            other => {
                return Err(EvalError::eval(format!(
                    "python scalar evaluator does not support observable kind {}",
                    other.kind_str()
                )));
            }
        };
        Ok(BatchResult::new(weighted_values, observable_state))
    }
}

pub struct ComplexPythonEvaluator {
    domain: Domain,
    input_dim: usize,
    integrand: Py<PyAny>,
}

impl ComplexPythonEvaluator {
    pub fn new(input_dim: usize, integrand: Py<PyAny>) -> Self {
        Self {
            domain: Domain::continuous(input_dim),
            input_dim,
            integrand,
        }
    }
}

impl ComplexBatchEvaluator for ComplexPythonEvaluator {
    fn input_dim(&self) -> usize {
        self.input_dim
    }

    fn eval_complex_dense_batch(
        &mut self,
        xs_row_major: &[f64],
        nr_samples: usize,
    ) -> Result<Vec<Complex64>, EvalError> {
        with_python_eval(|py| {
            let np = PyModule::import(py, "numpy")
                .map_err(|err| EvalError::eval(format!("failed importing numpy: {err}")))?;
            let xs = np
                .call_method1("array", (xs_row_major,))
                .and_then(|value| value.call_method1("reshape", (nr_samples, self.input_dim)))
                .map_err(|err| EvalError::eval(format!("failed preparing numpy input batch: {err}")))?;
            let values = self
                .integrand
                .bind(py)
                .call_method1("eval", (xs,))
                .map_err(|err| EvalError::eval(format!("python integrand eval() failed: {err}")))?;
            let values = np
                .call_method1("asarray", (values,))
                .and_then(|value| value.call_method1("reshape", (nr_samples,)))
                .map_err(|err| EvalError::eval(format!("failed normalizing complex output shape: {err}")))?;
            let re = values
                .getattr("real")
                .and_then(|value| value.call_method0("tolist"))
                .and_then(|value| value.extract::<Vec<f64>>())
                .map_err(|err| EvalError::eval(format!("failed extracting complex real part: {err}")))?;
            let im = values
                .getattr("imag")
                .and_then(|value| value.call_method0("tolist"))
                .and_then(|value| value.extract::<Vec<f64>>())
                .map_err(|err| EvalError::eval(format!("failed extracting complex imaginary part: {err}")))?;
            if re.len() != nr_samples || im.len() != nr_samples {
                return Err(EvalError::eval(format!(
                    "python complex integrand output size mismatch: got re={}, im={}, expected {nr_samples}",
                    re.len(),
                    im.len()
                )));
            }
            Ok(re
                .into_iter()
                .zip(im)
                .map(|(re, im)| Complex64::new(re, im))
                .collect())
        })
    }
}

impl Evaluator for ComplexPythonEvaluator {
    fn get_domain(&self) -> Domain {
        self.domain.clone()
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable: &ObservableConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let mut observable_state = ObservableState::from_config(observable);
        let weighted_values = match &mut observable_state {
            ObservableState::Complex(observable) => self.eval_complex_batch_into(
                batch,
                observable,
                options.require_training_values,
                |value| value.norm(),
            )?,
            ObservableState::FullComplex(observable) => self.eval_complex_batch_into(
                batch,
                observable,
                options.require_training_values,
                |value| value.norm(),
            )?,
            other => {
                return Err(EvalError::eval(format!(
                    "python complex evaluator does not support observable kind {}",
                    other.kind_str()
                )));
            }
        };
        Ok(BatchResult::new(weighted_values, observable_state))
    }
}

fn resolve_flake_output_path(flake_ref: &str) -> Result<PathBuf, BuildError> {
    let nix = resolve_nix_executable()?;
    let output = Command::new(&nix)
        .arg("build")
        .arg("--no-link")
        .arg("--print-out-paths")
        .arg(flake_ref)
        .output()
        .map_err(|err| {
            BuildError::build(format!(
                "failed running '{} build --no-link --print-out-paths {}': {err}",
                nix.display(),
                flake_ref
            ))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(BuildError::build(format!(
            "nix build failed for '{}': exit={} stderr='{}' stdout='{}'",
            flake_ref,
            output.status,
            stderr.trim(),
            stdout.trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(first_line) = stdout.lines().next().map(str::trim).filter(|line| !line.is_empty()) else {
        return Err(BuildError::build(format!(
            "nix build returned no output paths for '{flake_ref}'"
        )));
    };
    Ok(PathBuf::from(first_line))
}

fn resolve_nix_executable() -> Result<PathBuf, BuildError> {
    let mut candidates = Vec::new();
    if let Some(path_env) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_env) {
            candidates.push(dir.join("nix"));
        }
    }
    candidates.push(PathBuf::from("/run/current-system/sw/bin/nix"));
    candidates.push(PathBuf::from("/nix/var/nix/profiles/default/bin/nix"));
    candidates.push(PathBuf::from("/usr/bin/nix"));

    if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
        return Ok(path);
    }

    Err(BuildError::build(
        "python_scalar evaluator requires nix, but no 'nix' executable was found in PATH (or common system locations). Install nix or run gammaboard in an environment where nix is available.",
    ))
}

fn with_python_build<T>(
    f: impl for<'py> FnOnce(Python<'py>) -> Result<T, BuildError>,
) -> Result<T, BuildError> {
    Python::initialize();
    Python::try_attach(f)
        .ok_or_else(|| BuildError::build("python interpreter is not available on this thread"))?
}

fn with_python_eval<T>(
    f: impl for<'py> FnOnce(Python<'py>) -> Result<T, EvalError>,
) -> Result<T, EvalError> {
    Python::initialize();
    Python::try_attach(f)
        .ok_or_else(|| EvalError::eval("python interpreter is not available on this thread"))?
}

fn collect_site_packages(flake_out: &Path) -> Vec<PathBuf> {
    let lib_dir = flake_out.join("lib");
    let Ok(entries) = fs::read_dir(lib_dir) else {
        return Vec::new();
    };
    let mut site_packages = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("python") {
            continue;
        }
        let site = path.join("site-packages");
        if site.is_dir() {
            site_packages.push(site);
        }
    }
    site_packages
}
