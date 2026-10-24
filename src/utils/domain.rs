use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscreteAxisRole {
    #[default]
    Integrand,
    Stratum,
}

/// Structural domain tree for concrete sample layouts.
///
/// This mirrors the shape of nested discrete/continuous grids without carrying
/// any sampler-specific adaptation or training state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    Discrete {
        axis_label: Option<String>,
        #[serde(default)]
        axis_role: DiscreteAxisRole,
        branches: Vec<DomainBranch>,
    },
    Rectangular {
        #[serde(default)]
        strata_dims: usize,
        discrete_cardinalities: Vec<usize>,
        continuous_dims: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainBranch {
    pub index: usize,
    pub domain: Box<Domain>,
}

impl Domain {
    pub fn continuous(dims: usize) -> Self {
        Self::rectangular(dims, 0)
    }

    pub fn rectangular(continuous_dims: usize, discrete_dims: usize) -> Self {
        Self::rectangular_with_cardinalities(continuous_dims, vec![1; discrete_dims])
    }

    pub fn rectangular_with_cardinalities(
        continuous_dims: usize,
        discrete_cardinalities: impl IntoIterator<Item = usize>,
    ) -> Self {
        Self::Rectangular {
            strata_dims: 0,
            discrete_cardinalities: discrete_cardinalities.into_iter().collect(),
            continuous_dims,
        }
    }

    pub fn discrete(
        axis_label: impl Into<Option<String>>,
        branches: impl IntoIterator<Item = DomainBranch>,
    ) -> Self {
        Self::Discrete {
            axis_label: axis_label.into(),
            axis_role: DiscreteAxisRole::Integrand,
            branches: branches.into_iter().collect(),
        }
    }

    /// A compact, entirely discrete stratum prefix; attach the integrand later.
    pub fn strata(cardinalities: Vec<usize>) -> Self {
        Self::Rectangular {
            strata_dims: cardinalities.len(),
            discrete_cardinalities: cardinalities,
            continuous_dims: 0,
        }
    }

    /// Validate structural labels and the left-bound stratum rule on every path.
    pub fn validate(&self) -> Result<(), String> {
        fn visit(domain: &Domain, integrand_seen: bool) -> Result<(), String> {
            match domain {
                Domain::Rectangular {
                    strata_dims,
                    discrete_cardinalities,
                    ..
                } => {
                    if discrete_cardinalities.contains(&0)
                        || *strata_dims > discrete_cardinalities.len()
                    {
                        return Err("invalid rectangular cardinalities or strata_dims".into());
                    }
                    if integrand_seen && *strata_dims != 0 {
                        return Err(
                            "stratum dimensions must be left-bound, before integrand dimensions"
                                .into(),
                        );
                    }
                    Ok(())
                }
                Domain::Discrete {
                    axis_role,
                    branches,
                    ..
                } => {
                    if branches.is_empty()
                        || branches.iter().any(|b| b.index > i64::MAX as usize)
                        || branches
                            .windows(2)
                            .any(|pair| pair[0].index >= pair[1].index)
                    {
                        return Err(
                            "domain branches must be nonempty with strictly increasing labels"
                                .into(),
                        );
                    }
                    let stratum = *axis_role == DiscreteAxisRole::Stratum;
                    if integrand_seen && stratum {
                        return Err(
                            "stratum dimensions must be left-bound, before integrand dimensions"
                                .into(),
                        );
                    }
                    for branch in branches {
                        visit(&branch.domain, integrand_seen || !stratum)?;
                    }
                    Ok(())
                }
            }
        }
        visit(self, false)
    }

    /// Enumerate valid joint strata, without expanding any integrand coordinates.
    pub fn stratum_prefixes(&self) -> Result<Vec<Vec<i64>>, String> {
        fn visit(domain: &Domain, prefix: &mut Vec<i64>, out: &mut Vec<Vec<i64>>) {
            match domain {
                Domain::Discrete {
                    axis_role: DiscreteAxisRole::Stratum,
                    branches,
                    ..
                } => {
                    for branch in branches {
                        prefix.push(branch.index as i64);
                        visit(&branch.domain, prefix, out);
                        prefix.pop();
                    }
                }
                Domain::Discrete { .. } => out.push(prefix.clone()),
                Domain::Rectangular {
                    strata_dims,
                    discrete_cardinalities,
                    ..
                } => {
                    fn product(dims: &[usize], prefix: &mut Vec<i64>, out: &mut Vec<Vec<i64>>) {
                        if let Some((&n, tail)) = dims.split_first() {
                            for i in 0..n {
                                prefix.push(i as i64);
                                product(tail, prefix, out);
                                prefix.pop();
                            }
                        } else {
                            out.push(prefix.clone());
                        }
                    }
                    product(&discrete_cardinalities[..*strata_dims], prefix, out);
                }
            }
        }
        self.validate()?;
        let mut out = Vec::new();
        visit(self, &mut Vec::new(), &mut out);
        Ok(out)
    }

    /// Append an integrand domain to an independently specified stratum-only tree.
    pub fn with_integrand(&self, integrand: &Domain) -> Result<Domain, String> {
        if integrand.stratum_prefixes()? != vec![Vec::<i64>::new()]
            || self.stratum_prefixes()?.iter().any(Vec::is_empty)
        {
            return Err(
                "expected a stratum-only prefix and an unstratified integrand domain".into(),
            );
        }
        fn append(prefix: &Domain, integrand: &Domain) -> Result<Domain, String> {
            match prefix {
                Domain::Discrete {
                    axis_label,
                    axis_role: DiscreteAxisRole::Stratum,
                    branches,
                } => {
                    let branches = branches
                        .iter()
                        .map(|b| Ok(DomainBranch::new(b.index, append(&b.domain, integrand)?)))
                        .collect::<Result<_, String>>()?;
                    Ok(Domain::Discrete {
                        axis_label: axis_label.clone(),
                        axis_role: DiscreteAxisRole::Stratum,
                        branches,
                    })
                }
                Domain::Rectangular {
                    strata_dims,
                    discrete_cardinalities,
                    continuous_dims: 0,
                } if *strata_dims == discrete_cardinalities.len() => {
                    if let Domain::Rectangular {
                        discrete_cardinalities: tail,
                        continuous_dims,
                        ..
                    } = integrand
                    {
                        return Ok(Domain::Rectangular {
                            strata_dims: *strata_dims,
                            discrete_cardinalities: discrete_cardinalities
                                .iter()
                                .chain(tail)
                                .copied()
                                .collect(),
                            continuous_dims: *continuous_dims,
                        });
                    }
                    let mut result = integrand.clone();
                    for &n in discrete_cardinalities.iter().rev() {
                        result = Domain::Discrete {
                            axis_label: None,
                            axis_role: DiscreteAxisRole::Stratum,
                            branches: (0..n)
                                .map(|i| DomainBranch::new(i, result.clone()))
                                .collect(),
                        };
                    }
                    Ok(result)
                }
                _ => Err(concat!(
                    "stratification domain must contain only stratum dimensions ",
                    "and zero continuous dimensions"
                )
                .into()),
            }
        }
        append(self, integrand)
    }

    pub fn fixed_continuous_dims(&self) -> Option<usize> {
        match self {
            Self::Rectangular {
                continuous_dims, ..
            } => Some(*continuous_dims),
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
            Self::Rectangular {
                discrete_cardinalities,
                ..
            } => Some(discrete_cardinalities.len()),
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

    pub fn continuous_dims_at_discrete_path(&self, discrete: &[i64]) -> Result<usize, String> {
        if discrete.is_empty() {
            return self.fixed_continuous_dims().ok_or_else(|| {
                "discrete path does not determine a unique continuous dimensionality".to_string()
            });
        }
        match self {
            Self::Rectangular {
                discrete_cardinalities,
                continuous_dims,
                ..
            } => {
                if discrete.len() > discrete_cardinalities.len() {
                    return Err(format!(
                        "discrete path {:?} continues beyond rectangular domain depth {}",
                        discrete,
                        discrete_cardinalities.len()
                    ));
                }
                for (axis, (&branch, &cardinality)) in discrete
                    .iter()
                    .zip(discrete_cardinalities.iter())
                    .enumerate()
                {
                    let branch = usize::try_from(branch).map_err(|_| {
                        format!(
                            "discrete branch index {branch} at axis {axis} is negative and cannot select a domain branch"
                        )
                    })?;
                    if branch >= cardinality {
                        return Err(format!(
                            "discrete branch index {branch} at axis {axis} is outside rectangular cardinality {cardinality}"
                        ));
                    }
                }
                Ok(*continuous_dims)
            }
            Self::Discrete { branches, .. } => {
                let branch_index = usize::try_from(discrete[0]).map_err(|_| {
                    format!(
                        "discrete branch index {} is negative and cannot select a domain branch",
                        discrete[0]
                    )
                })?;
                let branch = branches
                    .iter()
                    .find(|branch| branch.index == branch_index)
                    .ok_or_else(|| {
                        format!("discrete branch index {branch_index} does not exist in domain")
                    })?;
                branch
                    .domain
                    .continuous_dims_at_discrete_path(&discrete[1..])
            }
        }
    }

    pub fn validate_point(&self, point: &crate::evaluation::Point) -> Result<(), String> {
        self.validate_point_parts(&point.discrete, &point.continuous)
    }

    pub fn validate_batch(&self, batch: &crate::evaluation::Batch) -> Result<(), String> {
        for (sample_idx, point) in batch.points().iter().enumerate() {
            self.validate_point(point)
                .map_err(|err| format!("sample {sample_idx}: {err}"))?;
        }
        Ok(())
    }

    pub fn validate_point_parts(&self, discrete: &[i64], continuous: &[f64]) -> Result<(), String> {
        let expected_continuous_dims = self.continuous_dims_at_leaf_path(discrete)?;
        if continuous.len() != expected_continuous_dims {
            return Err(format!(
                "discrete path {:?} expects {} continuous coordinates, got {}",
                discrete,
                expected_continuous_dims,
                continuous.len()
            ));
        }
        Ok(())
    }

    fn continuous_dims_at_leaf_path(&self, discrete: &[i64]) -> Result<usize, String> {
        match self {
            Self::Rectangular {
                discrete_cardinalities,
                continuous_dims,
                ..
            } => {
                if discrete.len() != discrete_cardinalities.len() {
                    return Err(format!(
                        "discrete path {:?} must have rectangular domain depth {}, got {}",
                        discrete,
                        discrete_cardinalities.len(),
                        discrete.len()
                    ));
                }
                self.continuous_dims_at_discrete_path(discrete)?;
                Ok(*continuous_dims)
            }
            Self::Discrete { branches, .. } => {
                let Some((&head, tail)) = discrete.split_first() else {
                    return Err(
                        "discrete path does not reach a concrete discrete domain branch"
                            .to_string(),
                    );
                };
                let branch_index = usize::try_from(head).map_err(|_| {
                    format!(
                        "discrete branch index {head} is negative and cannot select a domain branch"
                    )
                })?;
                let branch = branches
                    .iter()
                    .find(|branch| branch.index == branch_index)
                    .ok_or_else(|| {
                        format!("discrete branch index {branch_index} does not exist in domain")
                    })?;
                branch.domain.continuous_dims_at_leaf_path(tail)
            }
        }
    }

    pub fn fixed_discrete_cardinalities(&self) -> Option<Vec<usize>> {
        match self {
            Self::Rectangular {
                discrete_cardinalities,
                ..
            } => Some(discrete_cardinalities.clone()),
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
    use super::{Domain, DomainBranch};

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

    #[test]
    fn discrete_path_selects_inhomogeneous_continuous_leaf() {
        let domain = Domain::discrete(
            None,
            [
                DomainBranch::new(0, Domain::continuous(3)),
                DomainBranch::new(
                    1,
                    Domain::discrete(
                        None,
                        [
                            DomainBranch::new(0, Domain::continuous(1)),
                            DomainBranch::new(
                                1,
                                Domain::discrete(
                                    None,
                                    (0..5).map(|index| {
                                        DomainBranch::new(index, Domain::continuous(5))
                                    }),
                                ),
                            ),
                        ],
                    ),
                ),
            ],
        );

        assert_eq!(domain.continuous_dims_at_discrete_path(&[0]), Ok(3));
        assert_eq!(domain.continuous_dims_at_discrete_path(&[1, 0]), Ok(1));
        assert_eq!(domain.continuous_dims_at_discrete_path(&[1, 1]), Ok(5));
        assert_eq!(domain.continuous_dims_at_discrete_path(&[1, 1, 4]), Ok(5));
        assert!(domain.continuous_dims_at_discrete_path(&[1, 1, 5]).is_err());
        assert!(domain.continuous_dims_at_discrete_path(&[0, 0]).is_err());
    }

    #[test]
    fn validate_point_accepts_rectangular_cardinality_and_width() {
        let domain = Domain::rectangular_with_cardinalities(2, [3, 4]);
        assert!(domain.validate_point_parts(&[2, 3], &[0.1, 0.2]).is_ok());
        assert!(domain.validate_point_parts(&[3, 0], &[0.1, 0.2]).is_err());
        assert!(domain.validate_point_parts(&[2], &[0.1, 0.2]).is_err());
        assert!(domain.validate_point_parts(&[2, 3], &[0.1]).is_err());
    }

    #[test]
    fn validate_point_accepts_inhomogeneous_branch_widths() {
        let domain = Domain::discrete(
            None,
            [
                DomainBranch::new(0, Domain::continuous(1)),
                DomainBranch::new(1, Domain::continuous(3)),
            ],
        );
        assert!(domain.validate_point_parts(&[0], &[0.1]).is_ok());
        assert!(domain.validate_point_parts(&[1], &[0.1, 0.2, 0.3]).is_ok());
        assert!(domain.validate_point_parts(&[1], &[0.1]).is_err());
    }
    #[test]
    fn continuous_domains_have_one_canonical_representation() {
        let continuous = Domain::continuous(6);
        assert_eq!(continuous, Domain::rectangular_with_cardinalities(6, []));
        let json = serde_json::to_value(&continuous).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"rectangular": {"strata_dims": 0, "discrete_cardinalities": [], "continuous_dims": 6}})
        );
        assert_eq!(serde_json::from_value::<Domain>(json).unwrap(), continuous);
        assert!(
            serde_json::from_value::<Domain>(serde_json::json!({"continuous": {"dims": 6}}))
                .is_err()
        );
    }
}
