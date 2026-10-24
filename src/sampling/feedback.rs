//! One return path for generic and stratum-aware training algorithms.
use crate::core::EngineError;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StratumAssignments {
    pub prefixes: Vec<Vec<i64>>,
    pub requested: Vec<usize>,
    pub actual: Vec<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feedback {
    pub unmasked_values: Vec<f64>,
    pub strata: Option<StratumAssignments>,
}

impl Feedback {
    pub fn new(unmasked_values: Vec<f64>) -> Self {
        Self {
            unmasked_values,
            strata: None,
        }
    }
    pub fn len(&self) -> usize {
        self.unmasked_values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.unmasked_values.is_empty()
    }

    /// Generic samplers consume this view without interpreting stratum metadata.
    pub fn values(&self) -> Cow<'_, [f64]> {
        match &self.strata {
            None => Cow::Borrowed(&self.unmasked_values),
            Some(strata) => Cow::Owned(
                self.unmasked_values
                    .iter()
                    .zip(&strata.requested)
                    .zip(&strata.actual)
                    .map(
                        |((&value, requested), actual)| {
                            if requested == actual { value } else { 0.0 }
                        },
                    )
                    .collect(),
            ),
        }
    }

    pub fn validate(&self) -> Result<(), EngineError> {
        if self.unmasked_values.iter().any(|value| !value.is_finite()) {
            return Err(EngineError::eval("non-finite training feedback"));
        }
        if let Some(strata) = &self.strata
            && (strata.prefixes.is_empty()
                || strata
                    .prefixes
                    .iter()
                    .any(|p| p.is_empty() || p.iter().any(|v| *v < 0))
                || strata
                    .prefixes
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1] || pair[1].starts_with(&pair[0]))
                || strata.requested.len() != self.len()
                || strata.actual.len() != self.len()
                || strata
                    .requested
                    .iter()
                    .chain(&strata.actual)
                    .any(|&k| k >= strata.prefixes.len()))
        {
            return Err(EngineError::eval("invalid feedback stratum assignments"));
        }
        Ok(())
    }

    /// Assemble evaluator fragments while preserving one draw's original order.
    pub(crate) fn append(&mut self, other: &Self) -> Result<(), EngineError> {
        other.validate()?;
        if self.is_empty() {
            *self = other.clone();
            return Ok(());
        }
        match (&mut self.strata, &other.strata) {
            (None, None) => {}
            (Some(a), Some(b)) if a.prefixes == b.prefixes => {
                a.requested.extend_from_slice(&b.requested);
                a.actual.extend_from_slice(&b.actual);
            }
            _ => {
                return Err(EngineError::eval(
                    "inconsistent feedback stratum layout within a draw",
                ));
            }
        }
        self.unmasked_values
            .extend_from_slice(&other.unmasked_values);
        Ok(())
    }
}

impl StratumAssignments {
    pub fn requested_prefixes(&self) -> impl Iterator<Item = &[i64]> {
        self.requested.iter().map(|&i| self.prefixes[i].as_slice())
    }
    pub fn actual_prefixes(&self) -> impl Iterator<Item = &[i64]> {
        self.actual.iter().map(|&i| self.prefixes[i].as_slice())
    }
}
