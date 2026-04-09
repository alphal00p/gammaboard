use pyo3::prelude::*;

pub struct ScalarPythonEvaluator {
    domain: Domain,
    input_dim: usize,
    integrand: Py<PyAny>,
}

pub struct ComplexPythonEvaluator {
    domain: Domain,
    input_dim: usize,
    integrand: Py<PyAny>,
}
