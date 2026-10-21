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
    global_pdf_norm: f64,
    source_sampler_snapshot: SavedSampler,
}

pub(crate) trait RasterGeometry: Clone + Send {
    fn validate_domain(&self, domain: &Domain) -> Result<(), EngineError>;
    fn nr_points(&self) -> usize;
    fn point_at(&self, index: usize) -> Vec<f64>;
    fn discrete(&self) -> &[i64];
}
impl RasterGeometry for PlaneRasterGeometry {
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfAdaptationImagePersistedOutput {
    pub processed: usize,
    pub global_abs_integrand_norm: Option<f64>,
    #[serde(default = "default_pdf_global_norm")]
    pub global_pdf_norm: f64,
    pub integrand_values: Vec<Option<f64>>,
    pub pdf_values: Vec<Option<f64>>,
}

fn default_pdf_global_norm() -> f64 {
    1.0
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
    global_pdf_norm: f64,
    source_sampler_snapshot: SavedSampler,
    source_sampler: Box<dyn Sampler>,
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
    pub fn from_params_and_snapshot(
        params: RasterSamplerParams<G>,
        source_sampler_snapshot: SavedSampler,
        global_abs_integrand_norm: Option<f64>,
        domain: &Domain,
    ) -> Result<Self, EngineError> {
        params.geometry.validate_domain(domain)?;
        let total_samples = params.geometry.nr_points();
        let mut source_sampler = source_sampler_snapshot
            .clone()
            .restore(domain, serde_json::json!({}))?;
        let global_pdf_norm = source_sampler.global_pdf_norm().map_err(|err| {
            EngineError::build(format!(
                "failed to read global pdf normalization from source sampler: {err}"
            ))
        })?;
        Ok(Self {
            params,
            next_index: 0,
            stride: coprime_stride(total_samples),
            ingested_samples: 0,
            output_state: PdfAdaptationImageOutputState::new(total_samples),
            global_abs_integrand_norm,
            global_pdf_norm,
            source_sampler_snapshot,
            source_sampler,
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
            global_pdf_norm: snapshot.global_pdf_norm,
            ..sampler
        })
    }
}

impl PdfAdaptationImageOutputState {
    fn new(total_samples: usize) -> Self {
        Self {
            integrand_values: vec![None; total_samples],
            pdf_values: vec![None; total_samples],
        }
    }

    fn persisted_output(
        &self,
        processed: usize,
        global_abs_integrand_norm: Option<f64>,
        global_pdf_norm: f64,
    ) -> PdfAdaptationImagePersistedOutput {
        PdfAdaptationImagePersistedOutput {
            processed,
            global_abs_integrand_norm,
            global_pdf_norm,
            integrand_values: self.integrand_values.clone(),
            pdf_values: self.pdf_values.clone(),
        }
    }
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

    fn feedback(&mut self, _training_values: &[f64]) -> Result<(), EngineError> {
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
            "pdf adaptation raster sampler",
        )?;
        let training_remaining = geometry.nr_points() - self.next_index + batch.nr_samples;
        Ok(Generation::batch(batch, Some(training_remaining)))
    }

    fn feedback(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
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

    fn discrete_pdf_batch(
        &mut self,
        subspaces: &[DiscreteSubspace],
    ) -> Result<Vec<Option<f64>>, EngineError> {
        self.source_sampler.discrete_pdf_batch(subspaces)
    }

    fn persisted_output(&mut self) -> Result<Option<serde_json::Value>, EngineError> {
        serde_json::to_value(self.output_state.persisted_output(
            self.ingested_samples,
            self.global_abs_integrand_norm,
            self.global_pdf_norm,
        ))
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
            global_pdf_norm: self.global_pdf_norm,
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

        sampler.feedback(&[2.0, 4.0]).expect("ingest weights");
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

        sampler.feedback(&[f64::NAN, 4.0]).expect("ingest weights");
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
