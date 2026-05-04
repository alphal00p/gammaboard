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
        Self::rectangular_with_cardinalities(continuous_dims, vec![1; discrete_dims])
    }

    pub fn rectangular_with_cardinalities(
        continuous_dims: usize,
        discrete_cardinalities: impl IntoIterator<Item = usize>,
    ) -> Self {
        let mut domain = Self::continuous(continuous_dims);
        let cardinalities: Vec<usize> = discrete_cardinalities.into_iter().collect();
        for cardinality in cardinalities.into_iter().rev() {
            let child = domain;
            let branches = (0..cardinality).map(|index| DomainBranch::new(index, child.clone()));
            domain = Self::discrete(None, branches);
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

    pub fn fixed_discrete_cardinalities(&self) -> Option<Vec<usize>> {
        match self {
            Self::Continuous { .. } => Some(Vec::new()),
            Self::Discrete { branches, .. } => {
                let first_tail = branches.first()?.domain.fixed_discrete_cardinalities()?;
                branches
                    .iter()
                    .all(|branch| {
                        branch.domain.fixed_discrete_cardinalities() == Some(first_tail.clone())
                    })
                    .then(|| {
                        let mut cardinalities = Vec::with_capacity(first_tail.len() + 1);
                        cardinalities.push(branches.len());
                        cardinalities.extend(first_tail);
                        cardinalities
                    })
            }
        }
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

#[cfg(test)]
mod tests {
    use super::Domain;

    #[test]
    fn rectangular_with_cardinalities_preserves_exact_shape() {
        let domain = Domain::rectangular_with_cardinalities(2, [3, 4, 2]);
        assert_eq!(domain.fixed_continuous_dims(), Some(2));
        assert_eq!(domain.fixed_discrete_depth(), Some(3));
        assert_eq!(domain.fixed_discrete_cardinalities(), Some(vec![3, 4, 2]));
    }

    #[test]
    fn rectangular_matches_unit_cardinality_constructor() {
        let a = Domain::rectangular(1, 2);
        let b = Domain::rectangular_with_cardinalities(1, [1, 1]);
        assert_eq!(a, b);
    }
}
