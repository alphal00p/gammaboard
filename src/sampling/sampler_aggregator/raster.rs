use crate::core::EngineError;
use crate::core::EngineResultExt;
use crate::core::{LineRasterGeometry, PlaneRasterGeometry};
use crate::evaluation::{Batch, Point};
use crate::sampling::{DiscreteSubspace, Generation, LatentBatch, PdfPoint, Sampler, SavedSampler};
use crate::utils::domain::Domain;
use num::Integer;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RasterSamplerParams<G> {
    pub geometry: G,
}
pub type RasterPlaneSamplerParams = RasterSamplerParams<PlaneRasterGeometry>;
pub type RasterLineSamplerParams = RasterSamplerParams<LineRasterGeometry>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterSamplerSnapshot {
    next_index: usize,
    stride: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfAdaptationRasterSnapshot {
    next_index: usize,
    stride: usize,
    ingested_samples: usize,
    output_state: PdfAdaptationImageOutputState,
    global_abs_integrand_norm: Option<f64>,
    source_sampler_snapshot: SavedSampler,
}

pub(crate) trait RasterGeometry: Clone + Send {
    fn validate_domain(&self, domain: &Domain) -> Result<(), EngineError>;
    fn nr_points(&self) -> usize;
    fn point_at(&self, index: usize) -> Vec<f64>;
    fn discrete(&self) -> &[i64];
    fn with_discrete(&self, discrete: Vec<i64>) -> Self;
    fn line(&self) -> Option<&LineRasterGeometry> {
        None
    }
}
impl RasterGeometry for PlaneRasterGeometry {
    fn with_discrete(&self, discrete: Vec<i64>) -> Self {
        Self {
            discrete,
            ..self.clone()
        }
    }
    fn validate_domain(&self, domain: &Domain) -> Result<(), EngineError> {
        validate_plane_geometry(self, domain)
    }
    fn nr_points(&self) -> usize {
        self.nr_points()
    }
    fn point_at(&self, index: usize) -> Vec<f64> {
        self.point_at(index)
    }
    fn discrete(&self) -> &[i64] {
        &self.discrete
    }
}
impl RasterGeometry for LineRasterGeometry {
    fn line(&self) -> Option<&LineRasterGeometry> {
        Some(self)
    }
    fn with_discrete(&self, discrete: Vec<i64>) -> Self {
        Self {
            discrete,
            ..self.clone()
        }
    }
    fn validate_domain(&self, domain: &Domain) -> Result<(), EngineError> {
        validate_line_geometry(self, domain)
    }
    fn nr_points(&self) -> usize {
        self.nr_points()
    }
    fn point_at(&self, index: usize) -> Vec<f64> {
        self.point_at(index)
    }
    fn discrete(&self) -> &[i64] {
        &self.discrete
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfAdaptationImageOutputState {
    pub integrand_values: Vec<Option<f64>>,
    pub pdf_values: Vec<Option<f64>>,
    pub strata: Option<PdfStrataOutput>,
    pub target_label: Option<String>,
    pub pdf_steps: Option<PdfSteps>,
}

/// Exact step representation of a native Havana PDF restricted to a line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfSteps {
    pub edges: Vec<f64>,
    /// One row per stratum, or a single ordinary proposal row.
    pub densities: Vec<Vec<Option<f64>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfStrataOutput {
    pub prefixes: Vec<Vec<i64>>,
    pub probabilities: Vec<Option<f64>>,
    /// Joint densities on the same sampler-coordinate slice, including leakage.
    pub pdf_values: Vec<Vec<Option<f64>>>,
    pub actual_strata: Vec<Option<usize>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfAdaptationImagePersistedOutput {
    pub processed: usize,
    pub global_abs_integrand_norm: Option<f64>,
    pub integrand_values: Vec<Option<f64>>,
    pub pdf_values: Vec<Option<f64>>,
    pub strata: Option<PdfStrataOutput>,
    pub target_label: Option<String>,
    pub pdf_steps: Option<PdfSteps>,
}

pub struct RasterSampler<G> {
    params: RasterSamplerParams<G>,
    next_index: usize,
    stride: usize,
}
pub type RasterPlaneSampler = RasterSampler<PlaneRasterGeometry>;
pub type RasterLineSampler = RasterSampler<LineRasterGeometry>;

pub struct PdfAdaptationRasterSampler<G> {
    params: RasterSamplerParams<G>,
    next_index: usize,
    stride: usize,
    ingested_samples: usize,
    output_state: PdfAdaptationImageOutputState,
    global_abs_integrand_norm: Option<f64>,
    source_sampler_snapshot: SavedSampler,
    source_sampler: Box<dyn Sampler>,
    evaluation_discrete: Vec<i64>,
}
pub type PdfAdaptationRasterPlaneSampler = PdfAdaptationRasterSampler<PlaneRasterGeometry>;
pub type PdfAdaptationRasterLineSampler = PdfAdaptationRasterSampler<LineRasterGeometry>;

impl<G: RasterGeometry> RasterSampler<G> {
    pub fn from_params_and_domain(
        params: RasterSamplerParams<G>,
        domain: &Domain,
    ) -> Result<Self, EngineError> {
        params.geometry.validate_domain(domain)?;
        let total_samples = params.geometry.nr_points();
        Ok(Self {
            params,
            next_index: 0,
            stride: coprime_stride(total_samples),
        })
    }

    pub fn from_snapshot(
        params: RasterSamplerParams<G>,
        snapshot: RasterSamplerSnapshot,
        domain: &Domain,
    ) -> Result<Self, EngineError> {
        let sampler = Self::from_params_and_domain(params, domain)?;
        Ok(Self {
            next_index: snapshot.next_index,
            stride: snapshot.stride,
            ..sampler
        })
    }
}

impl<G: RasterGeometry> PdfAdaptationRasterSampler<G> {
    pub(crate) fn with_target_config(
        mut self,
        config: Option<crate::core::AccumulatorConfig>,
    ) -> Self {
        use crate::core::{AccumulatorConfig, TrainingProjection};
        self.output_state.target_label = config.map(|c| match c {
            AccumulatorConfig::Vector {
                training_projection: TrainingProjection::Norm,
                ..
            } => "‖f‖₂".into(),
            AccumulatorConfig::Vector {
                training_projection: TrainingProjection::Component { name },
                ..
            } => format!("|{name}|"),
            _ => "Absolute evaluator training projection".into(),
        });
        self
    }
    fn record_strata(&mut self, feedback: &crate::sampling::Feedback) -> Result<(), EngineError> {
        let geometry = &self.params.geometry;
        let total = geometry.nr_points();
        let output = self.output_state.strata.as_mut().expect("stratum output");
        let assignments = feedback.strata.as_ref().ok_or_else(|| {
            EngineError::eval("PDF diagnostics require actual stratum assignments")
        })?;
        if assignments.prefixes != output.prefixes
            || assignments.requested.iter().any(|&k| k != 0)
            || self.ingested_samples + feedback.len() > total
        {
            return Err(EngineError::eval("PDF diagnostic feedback layout mismatch"));
        }
        let indices = (0..feedback.len())
            .map(|i| permuted_raster_index(self.ingested_samples + i, total, self.stride))
            .collect::<Vec<_>>();
        for (k, prefix) in output.prefixes.iter().enumerate() {
            let discrete = [prefix.as_slice(), geometry.discrete()].concat();
            let points = indices
                .iter()
                .map(|&i| (discrete.clone(), geometry.point_at(i)))
                .collect::<Vec<_>>();
            let values = self.source_sampler.pdf_batch(&points)?;
            if values.len() != points.len()
                || values.iter().flatten().any(|p| !p.is_finite() || *p < 0.0)
            {
                return Err(EngineError::eval("invalid PDF diagnostic density response"));
            }
            for (&i, value) in indices.iter().zip(values) {
                output.pdf_values[k][i] = value;
            }
        }
        for ((&i, &value), &actual) in indices
            .iter()
            .zip(&feedback.unmasked_values)
            .zip(&assignments.actual)
        {
            self.output_state.integrand_values[i] = Some(value);
            output.actual_strata[i] = Some(actual);
            self.output_state.pdf_values[i] = output
                .pdf_values
                .iter()
                .map(|row| row[i])
                .collect::<Option<Vec<_>>>()
                .map(|values| values.iter().sum());
        }
        self.ingested_samples += feedback.len();
        Ok(())
    }

    pub fn from_params_and_snapshot(
        params: RasterSamplerParams<G>,
        source_sampler_snapshot: SavedSampler,
        global_abs_integrand_norm: Option<f64>,
        domain: &Domain,
    ) -> Result<Self, EngineError> {
        params.geometry.validate_domain(domain)?;
        let total_samples = params.geometry.nr_points();
        if source_sampler_snapshot.config.materializer.is_some() {
            return Err(EngineError::invalid_input(
                "PDF slices require a sampler-coordinate PDF and direct point materialization; custom materializers cannot be compared on an integrand-coordinate raster",
            ));
        }
        let mut source_sampler = source_sampler_snapshot
            .clone()
            .restore(domain, serde_json::json!({}))?;
        let mut output_state = PdfAdaptationImageOutputState::new(total_samples);
        let mut evaluation_discrete = params.geometry.discrete().to_vec();
        if let Some(partition) = &source_sampler_snapshot.config.stratification {
            let prefixes = partition.prefixes().cloned().collect::<Vec<_>>();
            let queries = prefixes
                .iter()
                .map(|prefix| DiscreteSubspace {
                    fixed_dims: prefix.iter().copied().enumerate().collect(),
                })
                .collect::<Vec<_>>();
            let probabilities = source_sampler.discrete_mass_batch(&queries)?;
            if probabilities.len() != prefixes.len()
                || probabilities
                    .iter()
                    .flatten()
                    .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
            {
                return Err(EngineError::eval(
                    "invalid stratum probabilities for PDF diagnostics",
                ));
            }
            evaluation_discrete = [prefixes[0].as_slice(), params.geometry.discrete()].concat();
            params
                .geometry
                .with_discrete(evaluation_discrete.clone())
                .validate_domain(&partition.sampling_domain(domain)?)?;
            output_state.strata = Some(PdfStrataOutput {
                pdf_values: vec![vec![None; total_samples]; prefixes.len()],
                prefixes,
                probabilities,
                actual_strata: vec![None; total_samples],
            });
        }
        if let Some(line) = params.geometry.line() {
            output_state.pdf_steps = native_pdf_steps(
                &source_sampler_snapshot,
                source_sampler.as_mut(),
                line,
                domain,
            )?;
        }
        Ok(Self {
            params,
            next_index: 0,
            stride: coprime_stride(total_samples),
            ingested_samples: 0,
            output_state,
            global_abs_integrand_norm,
            source_sampler_snapshot,
            source_sampler,
            evaluation_discrete,
        })
    }

    pub fn from_snapshot(
        params: RasterSamplerParams<G>,
        snapshot: PdfAdaptationRasterSnapshot,
        domain: &Domain,
    ) -> Result<Self, EngineError> {
        let sampler = Self::from_params_and_snapshot(
            params,
            snapshot.source_sampler_snapshot,
            snapshot.global_abs_integrand_norm,
            domain,
        )?;
        Ok(Self {
            next_index: snapshot.next_index,
            stride: snapshot.stride,
            ingested_samples: snapshot.ingested_samples,
            output_state: snapshot.output_state,
            ..sampler
        })
    }
}

impl PdfAdaptationImageOutputState {
    fn new(total_samples: usize) -> Self {
        Self {
            integrand_values: vec![None; total_samples],
            pdf_values: vec![None; total_samples],
            strata: None,
            target_label: None,
            pdf_steps: None,
        }
    }

    fn persisted_output(
        &self,
        processed: usize,
        global_abs_integrand_norm: Option<f64>,
    ) -> PdfAdaptationImagePersistedOutput {
        PdfAdaptationImagePersistedOutput {
            processed,
            global_abs_integrand_norm,
            integrand_values: self.integrand_values.clone(),
            pdf_values: self.pdf_values.clone(),
            strata: self.strata.clone(),
            target_label: self.target_label.clone(),
            pdf_steps: self.pdf_steps.clone(),
        }
    }
}

fn native_pdf_steps(
    saved: &SavedSampler,
    source: &mut dyn Sampler,
    line: &LineRasterGeometry,
    domain: &Domain,
) -> Result<Option<PdfSteps>, EngineError> {
    use symbolica::numerical_integration::{ContinuousGrid, Grid};
    if !saved.contains_havana_grid() || line.linspace.start == line.linspace.stop {
        return Ok(None);
    }
    fn continuous<'a>(grid: &'a Grid<f64>, path: &[i64]) -> Option<&'a ContinuousGrid<f64>> {
        match grid {
            Grid::Continuous(g) if path.is_empty() => Some(g),
            Grid::Uniform(_, g) => Some(g),
            Grid::Discrete(g) => {
                let (&index, rest) = path.split_first()?;
                continuous(
                    g.bins
                        .get(usize::try_from(index).ok()?)?
                        .sub_grid
                        .as_ref()?,
                    rest,
                )
            }
            _ => None,
        }
    }
    let grid: Grid<f64> = serde_json::from_value(saved.state["grid"].clone()).engine_err()?;
    let prefixes = saved
        .config
        .stratification
        .as_ref()
        .map(|p| p.prefixes().cloned().collect::<Vec<_>>())
        .unwrap_or_else(|| vec![vec![]]);
    let sampling_domain = saved.config.sampling_domain(domain)?;
    let lo = line.linspace.start.min(line.linspace.stop);
    let hi = line.linspace.start.max(line.linspace.stop);
    let mut edges = vec![lo, hi];
    for prefix in &prefixes {
        let mut path = [prefix.as_slice(), line.discrete.as_slice()].concat();
        crate::sampling::havana_grid::map_discrete_path(&sampling_domain, &mut path, false)?;
        let Some(grid) = continuous(&grid, &path) else {
            return Ok(None);
        };
        for (dim, (&offset, &direction)) in grid
            .continuous_dimensions
            .iter()
            .zip(line.offset.iter().zip(&line.direction))
        {
            if direction == 0.0 {
                continue;
            }
            edges.extend(
                dim.partitioning
                    .iter()
                    .map(|edge| (edge - offset) / direction)
                    .filter(|t| t.is_finite() && *t > lo && *t < hi),
            );
        }
    }
    edges.sort_by(f64::total_cmp);
    edges.dedup();
    let mut densities = Vec::new();
    for prefix in prefixes {
        let discrete = [prefix.as_slice(), line.discrete.as_slice()].concat();
        let points = edges
            .windows(2)
            .map(|e| {
                let t = e[0] + (e[1] - e[0]) * 0.5;
                (
                    discrete.clone(),
                    line.offset
                        .iter()
                        .zip(&line.direction)
                        .map(|(offset, d)| offset + t * d)
                        .collect(),
                )
            })
            .collect::<Vec<_>>();
        let values = source.pdf_batch(&points)?;
        if values.len() != points.len()
            || values.iter().flatten().any(|v| !v.is_finite() || *v < 0.)
        {
            return Err(EngineError::eval("invalid exact PDF step values"));
        }
        densities.push(values);
    }
    Ok(Some(PdfSteps { edges, densities }))
}

fn record_pdf_adaptation_values(
    output_state: &mut PdfAdaptationImageOutputState,
    source_sampler: &mut dyn Sampler,
    canonical_indices: &[usize],
    training_values: &[f64],
    point_at: impl Fn(usize) -> PdfPoint,
    geometry: &str,
) -> Result<(), EngineError> {
    let mut pdf_points = Vec::new();
    let mut pdf_indices = Vec::new();
    for (&canonical_index, &training_value) in canonical_indices.iter().zip(training_values) {
        if !training_value.is_finite() {
            output_state.integrand_values[canonical_index] = None;
            output_state.pdf_values[canonical_index] = None;
            continue;
        }
        output_state.integrand_values[canonical_index] = Some(training_value);
        pdf_indices.push(canonical_index);
        pdf_points.push(point_at(canonical_index));
    }
    let pdf_values = source_sampler.pdf_batch(&pdf_points)?;
    if pdf_values.len() != pdf_indices.len() {
        return Err(EngineError::engine(format!(
            "pdf adaptation raster {geometry} sampler pdf output size mismatch: expected {}, got {}",
            pdf_indices.len(),
            pdf_values.len()
        )));
    }
    for (canonical_index, pdf) in pdf_indices.into_iter().zip(pdf_values) {
        output_state.pdf_values[canonical_index] = pdf.filter(|pdf| pdf.is_finite());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn ingest_pdf_adaptation_values(
    ingested_samples: &mut usize,
    output_state: &mut PdfAdaptationImageOutputState,
    source_sampler: &mut dyn Sampler,
    total_samples: usize,
    stride: usize,
    training_values: &[f64],
    point_at: impl Fn(usize) -> PdfPoint,
    geometry: &str,
) -> Result<(), EngineError> {
    if *ingested_samples + training_values.len() > total_samples {
        return Err(EngineError::engine(format!(
            "pdf adaptation raster {geometry} sampler ingest overflow: {} + {} exceeds {}",
            *ingested_samples,
            training_values.len(),
            total_samples,
        )));
    }
    let canonical_indices = (0..training_values.len())
        .map(|offset| permuted_raster_index(*ingested_samples + offset, total_samples, stride))
        .collect::<Vec<_>>();
    record_pdf_adaptation_values(
        output_state,
        source_sampler,
        &canonical_indices,
        training_values,
        point_at,
        geometry,
    )?;
    *ingested_samples += training_values.len();
    Ok(())
}

fn produce_raster_batch(
    next_index: &mut usize,
    total_samples: usize,
    stride: usize,
    discrete: &[i64],
    point_at: impl Fn(usize) -> Vec<f64>,
    requested_samples: usize,
    sampler_name: &str,
) -> Result<LatentBatch, EngineError> {
    let nr_samples = requested_samples.min(total_samples.saturating_sub(*next_index));
    if nr_samples == 0 {
        return Err(EngineError::engine(format!(
            "{sampler_name} cannot produce an empty batch"
        )));
    }
    let batch = Batch::from_iter((0..nr_samples).map(|row_idx| {
        Point::new(
            point_at(permuted_raster_index(
                *next_index + row_idx,
                total_samples,
                stride,
            )),
            discrete.to_vec(),
            1.0,
        )
    }));
    *next_index += nr_samples;
    Ok(LatentBatch::from_batch(&batch))
}

impl<G: RasterGeometry> Sampler for RasterSampler<G> {
    fn validate_domain(&self, domain: &Domain) -> Result<(), EngineError> {
        self.params.geometry.validate_domain(domain)
    }

    fn generate(&mut self, max_samples: usize) -> Result<Generation, EngineError> {
        if max_samples == 0 {
            return Err(EngineError::invalid_input("max_samples must be positive"));
        }
        if self.next_index == self.params.geometry.nr_points() {
            return Ok(Generation::Finished);
        }
        let geometry = &self.params.geometry;
        let batch = produce_raster_batch(
            &mut self.next_index,
            geometry.nr_points(),
            self.stride,
            geometry.discrete(),
            |index| geometry.point_at(index),
            max_samples,
            "raster sampler",
        )?;
        Ok(Generation::batch(batch, None))
    }

    fn feedback(&mut self, _feedback: &crate::sampling::Feedback) -> Result<(), EngineError> {
        Ok(())
    }

    fn snapshot(&mut self) -> Result<serde_json::Value, EngineError> {
        serde_json::to_value(RasterSamplerSnapshot {
            next_index: self.next_index,
            stride: self.stride,
        })
        .engine_err()
    }
}

impl<G: RasterGeometry> Sampler for PdfAdaptationRasterSampler<G> {
    fn validate_domain(&self, domain: &Domain) -> Result<(), EngineError> {
        self.params
            .geometry
            .with_discrete(self.evaluation_discrete.clone())
            .validate_domain(domain)
    }

    fn generate(&mut self, max_samples: usize) -> Result<Generation, EngineError> {
        if max_samples == 0 {
            return Err(EngineError::invalid_input("max_samples must be positive"));
        }
        if self.next_index == self.params.geometry.nr_points() {
            return Ok(Generation::Finished);
        }
        let geometry = &self.params.geometry;
        let batch = produce_raster_batch(
            &mut self.next_index,
            geometry.nr_points(),
            self.stride,
            &self.evaluation_discrete,
            |index| geometry.point_at(index),
            max_samples,
            "pdf adaptation raster sampler",
        )?;
        let training_remaining = geometry.nr_points() - self.next_index + batch.nr_samples;
        Ok(Generation::batch(batch, Some(training_remaining)))
    }

    fn feedback(&mut self, feedback: &crate::sampling::Feedback) -> Result<(), EngineError> {
        if self.output_state.strata.is_some() {
            feedback.validate()?;
            return self.record_strata(feedback);
        }
        let training_values = &feedback.unmasked_values;
        let geometry = &self.params.geometry;
        ingest_pdf_adaptation_values(
            &mut self.ingested_samples,
            &mut self.output_state,
            self.source_sampler.as_mut(),
            geometry.nr_points(),
            self.stride,
            training_values,
            |index| (geometry.discrete().to_vec(), geometry.point_at(index)),
            "raster",
        )
    }

    fn pdf_batch(&mut self, points: &[PdfPoint]) -> Result<Vec<Option<f64>>, EngineError> {
        self.source_sampler.pdf_batch(points)
    }

    fn discrete_mass_batch(
        &mut self,
        subspaces: &[DiscreteSubspace],
    ) -> Result<Vec<Option<f64>>, EngineError> {
        self.source_sampler.discrete_mass_batch(subspaces)
    }

    fn persisted_output(&mut self) -> Result<Option<serde_json::Value>, EngineError> {
        serde_json::to_value(
            self.output_state
                .persisted_output(self.ingested_samples, self.global_abs_integrand_norm),
        )
        .map(Some)
        .engine_err()
    }

    fn snapshot(&mut self) -> Result<serde_json::Value, EngineError> {
        serde_json::to_value(PdfAdaptationRasterSnapshot {
            next_index: self.next_index,
            stride: self.stride,
            ingested_samples: self.ingested_samples,
            output_state: self.output_state.clone(),
            global_abs_integrand_norm: self.global_abs_integrand_norm,
            source_sampler_snapshot: self.source_sampler_snapshot.clone(),
        })
        .engine_err()
    }
}

fn validate_plane_geometry(
    geometry: &PlaneRasterGeometry,
    domain: &Domain,
) -> Result<(), EngineError> {
    geometry.validate().map_err(EngineError::invalid_input)?;
    let continuous_dims = domain
        .continuous_dims_at_discrete_path(&geometry.discrete)
        .map_err(|err| {
            EngineError::incompatible(format!(
                "plane geometry discrete path {:?} is not valid for domain: {err}",
                geometry.discrete
            ))
        })?;
    if geometry.offset.len() != continuous_dims {
        return Err(EngineError::incompatible(format!(
            "plane geometry continuous dimension mismatch: expected {}, got {}",
            continuous_dims,
            geometry.offset.len()
        )));
    }
    Ok(())
}

fn validate_line_geometry(
    geometry: &LineRasterGeometry,
    domain: &Domain,
) -> Result<(), EngineError> {
    geometry.validate().map_err(EngineError::invalid_input)?;
    let continuous_dims = domain
        .continuous_dims_at_discrete_path(&geometry.discrete)
        .map_err(|err| {
            EngineError::incompatible(format!(
                "line geometry discrete path {:?} is not valid for domain: {err}",
                geometry.discrete
            ))
        })?;
    if geometry.offset.len() != continuous_dims {
        return Err(EngineError::incompatible(format!(
            "line geometry continuous dimension mismatch: expected {}, got {}",
            continuous_dims,
            geometry.offset.len()
        )));
    }
    Ok(())
}

pub(crate) fn permuted_raster_index(index: usize, total_samples: usize, stride: usize) -> usize {
    if total_samples <= 1 {
        return index.min(total_samples.saturating_sub(1));
    }
    (index * stride) % total_samples
}

pub(crate) fn coprime_stride(total_samples: usize) -> usize {
    if total_samples <= 1 {
        return 1;
    }

    let phi_minus_one = 0.618_033_988_749_894_9_f64;
    let mut candidate =
        ((total_samples as f64 * phi_minus_one).floor() as usize).clamp(1, total_samples - 1);
    while candidate.gcd(&total_samples) != 1 {
        candidate += 1;
        if candidate >= total_samples {
            candidate = 1;
        }
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::{
        PdfAdaptationRasterPlaneSampler, RasterLineSampler, RasterLineSamplerParams,
        RasterPlaneSampler, RasterPlaneSamplerParams, coprime_stride, permuted_raster_index,
    };
    use crate::core::{LineRasterGeometry, Linspace, PlaneRasterGeometry, SamplingConfig};
    use crate::sampling::Sampler;
    use crate::utils::domain::{Domain, DomainBranch};
    use num::Integer;

    #[test]
    fn native_pdf_steps_preserve_narrow_bins_and_integrals() {
        use crate::core::{AccumulatorConfig, SamplerConfig, TrainingProjection};
        let domain = Domain::continuous(1);
        let config = SamplingConfig {
            stratification: None,
            materializer: None,
            transforms: None,
            sampler: SamplerConfig::HavanaTraining {
                params: crate::sampling::HavanaSamplerParams {
                    bins: 4,
                    ..Default::default()
                },
            },
        };
        let mut source = config
            .create(domain.clone(), Some(32), serde_json::json!({}))
            .unwrap();
        let mut saved = config.snapshot(&mut *source).unwrap();
        saved.state["grid"]["Continuous"]["continuous_dimensions"][0]["partitioning"] =
            serde_json::json!([0.0, 0.4999, 0.5, 0.8, 1.0]);
        let params = RasterLineSamplerParams {
            geometry: LineRasterGeometry {
                offset: vec![0.0],
                direction: vec![1.0],
                linspace: Linspace {
                    start: 0.0,
                    stop: 1.0,
                    count: 3,
                },
                discrete: vec![],
            },
        };
        let raster = super::PdfAdaptationRasterLineSampler::from_params_and_snapshot(
            params, saved, None, &domain,
        )
        .unwrap()
        .with_target_config(Some(AccumulatorConfig::vector(
            vec!["x".into()],
            TrainingProjection::Norm,
        )));
        let steps = raster.output_state.pdf_steps.as_ref().unwrap();
        assert_eq!(steps.edges, vec![0., 0.4999, 0.5, 0.8, 1.]);
        assert!(steps.densities[0][1].unwrap() > 2000.);
        let integral = steps
            .edges
            .windows(2)
            .zip(&steps.densities[0])
            .map(|(e, q)| (e[1] - e[0]) * q.unwrap())
            .sum::<f64>();
        assert!((integral - 1.).abs() < 1e-14);
        assert_eq!(raster.output_state.target_label.as_deref(), Some("‖f‖₂"));
    }

    #[test]
    fn stratified_pdf_ragged_prefixes_preserve_unmasked_values_and_recover() {
        use crate::DiscreteAxisRole;
        use crate::core::StratificationConfig;
        use crate::sampling::{Feedback, StratumAssignments};
        let branch = |branches| Domain::Discrete {
            axis_label: None,
            axis_role: DiscreteAxisRole::Stratum,
            branches,
        };
        let prefix_domain = branch(vec![
            DomainBranch::new(0, Domain::continuous(0)),
            DomainBranch::new(
                1,
                branch(vec![
                    DomainBranch::new(0, Domain::continuous(0)),
                    DomainBranch::new(3, Domain::continuous(0)),
                ]),
            ),
        ]);
        let config = SamplingConfig {
            sampler: crate::core::SamplerConfig::NaiveMonteCarlo {
                params: Default::default(),
            },
            materializer: None,
            transforms: None,
            stratification: Some(StratificationConfig::Voronoi {
                domain: prefix_domain,
                centers: [
                    (vec![0], vec![0.]),
                    (vec![1, 0], vec![0.5]),
                    (vec![1, 3], vec![1.]),
                ]
                .into(),
                scales: None,
            }),
        };
        let domain = Domain::rectangular_with_cardinalities(1, [2]);
        let mut source = config
            .create(domain.clone(), None, serde_json::json!({}))
            .unwrap();
        let saved = config.snapshot(&mut *source).unwrap();
        let params = RasterLineSamplerParams {
            geometry: LineRasterGeometry {
                offset: vec![0.],
                direction: vec![1.],
                linspace: Linspace {
                    start: 0.,
                    stop: 1.,
                    count: 9,
                },
                discrete: vec![1],
            },
        };
        let mut raster = super::PdfAdaptationRasterLineSampler::from_params_and_snapshot(
            params.clone(),
            saved,
            None,
            &domain,
        )
        .unwrap();
        let consume = |raster: &mut super::PdfAdaptationRasterLineSampler, n| {
            let batch = raster
                .generate(n)
                .unwrap()
                .into_batch()
                .unwrap()
                .payload
                .into_batch()
                .unwrap();
            assert!(batch.points().iter().all(|p| p.discrete == vec![0, 1]));
            raster
                .feedback(&Feedback {
                    unmasked_values: batch
                        .points()
                        .iter()
                        .map(|p| 1. + p.continuous[0])
                        .collect(),
                    strata: Some(StratumAssignments {
                        prefixes: vec![vec![0], vec![1, 0], vec![1, 3]],
                        requested: vec![0; batch.len()],
                        actual: batch
                            .points()
                            .iter()
                            .map(|p| (p.continuous[0] * 2.).round() as usize)
                            .collect(),
                    }),
                })
                .unwrap();
        };
        consume(&mut raster, 4);
        let checkpoint = serde_json::from_value(raster.snapshot().unwrap()).unwrap();
        let mut restored =
            super::PdfAdaptationRasterLineSampler::from_snapshot(params, checkpoint, &domain)
                .unwrap();
        consume(&mut restored, 5);
        consume(&mut raster, 5);
        assert_eq!(
            raster.persisted_output().unwrap(),
            restored.persisted_output().unwrap()
        );
        let output = raster.output_state;
        assert_eq!(
            output.integrand_values,
            (0..9).map(|i| Some(1. + i as f64 / 8.)).collect::<Vec<_>>()
        );
        let strata = output.strata.unwrap();
        assert_eq!(
            strata.probabilities,
            vec![Some(0.5), Some(0.25), Some(0.25)]
        );
        assert_eq!(output.pdf_values, vec![Some(0.5); 9]);
        assert_eq!(strata.pdf_values[0], vec![Some(0.25); 9]);
        assert_eq!(strata.pdf_values[2], vec![Some(0.125); 9]);
        assert_eq!(strata.actual_strata[8], Some(2));
    }

    #[test]
    fn permuted_raster_index_visits_each_sample_once() {
        for total_samples in 1..32 {
            let stride = coprime_stride(total_samples);
            let mut seen = vec![false; total_samples];
            for index in 0..total_samples {
                let permuted = permuted_raster_index(index, total_samples, stride);
                assert!(permuted < total_samples);
                assert!(!seen[permuted], "duplicate index for n={total_samples}");
                seen[permuted] = true;
            }
            assert!(seen.into_iter().all(|value| value));
            assert_eq!(coprime_stride(total_samples).gcd(&total_samples), 1);
        }
    }

    #[test]
    fn raster_line_snapshot_restores_shuffled_progress() {
        let domain = Domain::rectangular(1, 0);
        let params = RasterLineSamplerParams {
            geometry: LineRasterGeometry {
                offset: vec![0.0],
                direction: vec![1.0],
                linspace: Linspace {
                    start: 0.0,
                    stop: 4.0,
                    count: 5,
                },
                discrete: Vec::new(),
            },
        };
        let mut sampler = RasterLineSampler::from_params_and_domain(params.clone(), &domain)
            .expect("build sampler");
        let first_batch = sampler
            .generate(2)
            .and_then(|generated| generated.into_batch())
            .expect("first batch");
        let snapshot = sampler.snapshot().expect("snapshot");
        let restored_snapshot = serde_json::from_value(snapshot).expect("decode raster line state");
        let mut restored =
            RasterLineSampler::from_snapshot(params, restored_snapshot, &domain).expect("restore");
        let second_batch = restored
            .generate(3)
            .and_then(|generated| generated.into_batch())
            .expect("second batch");

        let first_batch = first_batch.payload.as_batch().expect("decode first batch");
        let second_batch = second_batch
            .payload
            .as_batch()
            .expect("decode second batch");
        let first_points = first_batch
            .points()
            .iter()
            .map(|point| point.continuous[0])
            .collect::<Vec<_>>();
        let second_points = second_batch
            .points()
            .iter()
            .map(|point| point.continuous[0])
            .collect::<Vec<_>>();

        assert_eq!(first_points, vec![0.0, 3.0]);
        assert_eq!(second_points, vec![1.0, 4.0, 2.0]);
    }

    fn inhomogeneous_domain() -> Domain {
        Domain::discrete(
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
        )
    }

    #[test]
    fn raster_geometry_uses_discrete_path_to_select_continuous_dims() {
        let domain = inhomogeneous_domain();
        RasterPlaneSampler::from_params_and_domain(
            RasterPlaneSamplerParams {
                geometry: PlaneRasterGeometry {
                    offset: vec![0.0; 5],
                    u_vector: vec![1.0, 0.0, 0.0, 0.0, 0.0],
                    v_vector: vec![0.0, 1.0, 0.0, 0.0, 0.0],
                    u_linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    v_linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    discrete: vec![1, 1, 3],
                },
            },
            &domain,
        )
        .expect("build plane sampler on selected leaf");

        RasterLineSampler::from_params_and_domain(
            RasterLineSamplerParams {
                geometry: LineRasterGeometry {
                    offset: vec![0.0],
                    direction: vec![1.0],
                    linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    discrete: vec![1, 0],
                },
            },
            &domain,
        )
        .expect("build line sampler on selected leaf");
    }

    #[test]
    fn raster_geometry_rejects_continuous_dims_not_matching_selected_path() {
        let domain = inhomogeneous_domain();
        let result = RasterLineSampler::from_params_and_domain(
            RasterLineSamplerParams {
                geometry: LineRasterGeometry {
                    offset: vec![0.0, 0.0],
                    direction: vec![1.0, 0.0],
                    linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    discrete: vec![1, 0],
                },
            },
            &domain,
        )
        .map(|_| ());
        let err = result.expect_err("reject wrong continuous dimension");
        assert!(err.to_string().contains("continuous dimension mismatch"));
    }

    #[test]
    fn pdf_adaptation_raster_plane_delegates_pdf_to_source_sampler() {
        let domain = Domain::rectangular(2, 0);
        let source_config = SamplingConfig {
            stratification: None,
            sampler: crate::core::SamplerConfig::NaiveMonteCarlo {
                params: crate::sampling::NaiveMonteCarloSamplerParams::default(),
            },
            materializer: None,
            transforms: None,
        };
        let mut source = source_config
            .create(domain.clone(), None, serde_json::json!({}))
            .unwrap();
        let source_snapshot = source_config.snapshot(&mut *source).unwrap();

        let mut sampler = PdfAdaptationRasterPlaneSampler::from_params_and_snapshot(
            RasterPlaneSamplerParams {
                geometry: PlaneRasterGeometry {
                    offset: vec![0.0, 0.0],
                    u_vector: vec![1.0, 0.0],
                    v_vector: vec![0.0, 1.0],
                    u_linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    v_linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    discrete: Vec::new(),
                },
            },
            source_snapshot,
            None,
            &domain,
        )
        .expect("build pdf adaptation sampler");

        assert_eq!(
            sampler
                .pdf_batch(&[(Vec::new(), vec![0.25, 0.75])])
                .unwrap(),
            vec![Some(1.0)]
        );
    }

    #[test]
    fn pdf_adaptation_raster_plane_persists_integrand_and_pdf_images() {
        let domain = Domain::rectangular(2, 0);
        let source_config = SamplingConfig {
            stratification: None,
            sampler: crate::core::SamplerConfig::NaiveMonteCarlo {
                params: crate::sampling::NaiveMonteCarloSamplerParams::default(),
            },
            materializer: None,
            transforms: None,
        };
        let mut source = source_config
            .create(domain.clone(), None, serde_json::json!({}))
            .unwrap();
        let source_snapshot = source_config.snapshot(&mut *source).unwrap();

        let mut sampler = PdfAdaptationRasterPlaneSampler::from_params_and_snapshot(
            RasterPlaneSamplerParams {
                geometry: PlaneRasterGeometry {
                    offset: vec![0.0, 0.0],
                    u_vector: vec![1.0, 0.0],
                    v_vector: vec![0.0, 1.0],
                    u_linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    v_linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 1,
                    },
                    discrete: Vec::new(),
                },
            },
            source_snapshot,
            None,
            &domain,
        )
        .expect("build pdf adaptation sampler");

        sampler
            .feedback(&crate::sampling::Feedback::new(([2.0, 4.0]).to_vec()))
            .expect("ingest weights");
        let output = sampler
            .persisted_output()
            .expect("persisted output")
            .expect("custom payload");
        let output: super::PdfAdaptationImagePersistedOutput =
            serde_json::from_value(output).expect("decode payload");

        assert_eq!(output.processed, 2);
        assert_eq!(output.integrand_values, vec![Some(2.0), Some(4.0)]);
        assert_eq!(output.pdf_values, vec![Some(1.0), Some(1.0)]);
    }

    #[test]
    fn pdf_adaptation_raster_plane_ignores_non_finite_training_values() {
        let domain = Domain::rectangular(2, 0);
        let source_config = SamplingConfig {
            stratification: None,
            sampler: crate::core::SamplerConfig::NaiveMonteCarlo {
                params: crate::sampling::NaiveMonteCarloSamplerParams::default(),
            },
            materializer: None,
            transforms: None,
        };
        let mut source = source_config
            .create(domain.clone(), None, serde_json::json!({}))
            .unwrap();
        let source_snapshot = source_config.snapshot(&mut *source).unwrap();

        let mut sampler = PdfAdaptationRasterPlaneSampler::from_params_and_snapshot(
            RasterPlaneSamplerParams {
                geometry: PlaneRasterGeometry {
                    offset: vec![0.0, 0.0],
                    u_vector: vec![1.0, 0.0],
                    v_vector: vec![0.0, 1.0],
                    u_linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    v_linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 1,
                    },
                    discrete: Vec::new(),
                },
            },
            source_snapshot,
            None,
            &domain,
        )
        .expect("build pdf adaptation sampler");

        sampler
            .feedback(&crate::sampling::Feedback::new(([f64::NAN, 4.0]).to_vec()))
            .expect("ingest weights");
        let output = sampler
            .persisted_output()
            .expect("persisted output")
            .expect("custom payload");
        let output: super::PdfAdaptationImagePersistedOutput =
            serde_json::from_value(output).expect("decode payload");

        assert_eq!(output.processed, 2);
        assert_eq!(output.integrand_values, vec![None, Some(4.0)]);
        assert_eq!(output.pdf_values, vec![None, Some(1.0)]);
    }

    #[test]
    fn pdf_adaptation_raster_plane_uses_linspace_parameter_range() {
        let domain = Domain::rectangular(2, 0);
        let source_config = SamplingConfig {
            stratification: None,
            sampler: crate::core::SamplerConfig::NaiveMonteCarlo {
                params: crate::sampling::NaiveMonteCarloSamplerParams::default(),
            },
            materializer: None,
            transforms: None,
        };
        let mut source = source_config
            .create(domain.clone(), None, serde_json::json!({}))
            .unwrap();
        let source_snapshot = source_config.snapshot(&mut *source).unwrap();

        let mut sampler = PdfAdaptationRasterPlaneSampler::from_params_and_snapshot(
            RasterPlaneSamplerParams {
                geometry: PlaneRasterGeometry {
                    offset: vec![0.0, 0.0],
                    u_vector: vec![1.0, 0.0],
                    v_vector: vec![0.0, 1.0],
                    u_linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    v_linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    discrete: Vec::new(),
                },
            },
            source_snapshot,
            None,
            &domain,
        )
        .expect("build pdf adaptation sampler");

        let batch_spec = sampler
            .generate(4)
            .and_then(|generated| generated.into_batch())
            .expect("produce batch");
        let batch = batch_spec.payload.as_batch().expect("decode batch");
        let mut points = batch
            .points()
            .iter()
            .map(|point| point.continuous.clone())
            .collect::<Vec<_>>();
        points.sort_by(|a, b| {
            a[0].partial_cmp(&b[0])
                .expect("finite x")
                .then(a[1].partial_cmp(&b[1]).expect("finite y"))
        });

        assert_eq!(
            points,
            vec![
                vec![0.0, 0.0],
                vec![0.0, 1.0],
                vec![1.0, 0.0],
                vec![1.0, 1.0],
            ]
        );
    }
}
