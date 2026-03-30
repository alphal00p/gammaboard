use serde::{Deserialize, Serialize};

/// Structural domain tree for concrete sample layouts.
///
/// This mirrors the shape of nested discrete/continuous grids without carrying
/// any sampler-specific adaptation or training state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Domain {
    Continuous {
        dims: usize,
    },
    Discrete {
        axis_label: Option<String>,
        branches: Vec<DomainBranch>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainBranch {
    pub index: usize,
    pub domain: Box<Domain>,
}

impl Domain {
    pub fn continuous(dims: usize) -> Self {
        Self::Continuous { dims }
    }

    pub fn rectangular(continuous_dims: usize, discrete_dims: usize) -> Self {
        let mut domain = Self::continuous(continuous_dims);
        for _ in 0..discrete_dims {
            domain = Self::discrete(None, [DomainBranch::new(0, domain)]);
        }
        domain
    }

    pub fn discrete(
        axis_label: impl Into<Option<String>>,
        branches: impl IntoIterator<Item = DomainBranch>,
    ) -> Self {
        Self::Discrete {
            axis_label: axis_label.into(),
            branches: branches.into_iter().collect(),
        }
    }

    pub fn fixed_continuous_dims(&self) -> Option<usize> {
        match self {
            Self::Continuous { dims } => Some(*dims),
            Self::Discrete { branches, .. } => {
                if branches.is_empty() {
                    None
                } else {
                    let first = branches.first()?.domain.fixed_continuous_dims()?;
                    branches
                        .iter()
                        .all(|branch| branch.domain.fixed_continuous_dims() == Some(first))
                        .then_some(first)
                }
            }
        }
    }

    pub fn fixed_discrete_depth(&self) -> Option<usize> {
        match self {
            Self::Continuous { .. } => Some(0),
            Self::Discrete { branches, .. } => {
                if branches.is_empty() {
                    Some(1)
                } else {
                    let first = branches.first()?.domain.fixed_discrete_depth()?;
                    branches
                        .iter()
                        .all(|branch| branch.domain.fixed_discrete_depth() == Some(first))
                        .then_some(first + 1)
                }
            }
        }
    }

    pub fn fixed_rectangular_dims(&self) -> Option<(usize, usize)> {
        Some((self.fixed_continuous_dims()?, self.fixed_discrete_depth()?))
    }
}

impl DomainBranch {
    pub fn new(index: usize, domain: Domain) -> Self {
        Self {
            index,
            domain: Box::new(domain),
        }
    }
}
