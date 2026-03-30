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

    pub fn discrete(
        axis_label: impl Into<Option<String>>,
        branches: impl IntoIterator<Item = DomainBranch>,
    ) -> Self {
        Self::Discrete {
            axis_label: axis_label.into(),
            branches: branches.into_iter().collect(),
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
