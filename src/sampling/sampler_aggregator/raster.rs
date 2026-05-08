use crate::core::{BuildError, EngineError};
use crate::core::{LineRasterGeometry, PlaneRasterGeometry};
use crate::evaluation::{Batch, Point};
use crate::sampling::{
    LatentBatchSpec, PdfPoint, SamplePlan, SamplerAggregator, SamplerAggregatorSnapshot,
};
use crate::utils::domain::Domain;
use num::Integer;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RasterPlaneSamplerParams {
    pub geometry: PlaneRasterGeometry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RasterLineSamplerParams {
    pub geometry: LineRasterGeometry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfAdaptationRasterPlaneSamplerParams {
    pub geometry: PlaneRasterGeometry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfAdaptationRasterLineSamplerParams {
    pub geometry: LineRasterGeometry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterPlaneSamplerSnapshot {
    params: RasterPlaneSamplerParams,
    next_index: usize,
    stride: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterLineSamplerSnapshot {
    params: RasterLineSamplerParams,
    next_index: usize,
    stride: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfAdaptationRasterPlaneSamplerSnapshot {
    params: PdfAdaptationRasterPlaneSamplerParams,
    next_index: usize,
    stride: usize,
    ingested_samples: usize,
    output_state: PdfAdaptationImageOutputState,
    source_sampler_snapshot: SamplerAggregatorSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfAdaptationRasterLineSamplerSnapshot {
    params: PdfAdaptationRasterLineSamplerParams,
    next_index: usize,
    stride: usize,
    ingested_samples: usize,
    output_state: PdfAdaptationImageOutputState,
    source_sampler_snapshot: SamplerAggregatorSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfAdaptationImageOutputState {
    pub signed_integrand_values: Vec<Option<f64>>,
    pub abs_integrand_values: Vec<Option<f64>>,
    pub pdf_values: Vec<Option<f64>>,
    pub abs_integrand_sum: f64,
    pub abs_integrand_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfAdaptationImagePersistedOutput {
    pub processed: usize,
    pub abs_integrand_mean: Option<f64>,
    pub signed_integrand_values: Vec<Option<f64>>,
    pub abs_integrand_values: Vec<Option<f64>>,
    pub pdf_values: Vec<Option<f64>>,
}

pub struct RasterPlaneSampler {
    params: RasterPlaneSamplerParams,
    next_index: usize,
    stride: usize,
}

pub struct RasterLineSampler {
    params: RasterLineSamplerParams,
    next_index: usize,
    stride: usize,
}

pub struct PdfAdaptationRasterPlaneSampler {
    params: PdfAdaptationRasterPlaneSamplerParams,
    next_index: usize,
    stride: usize,
    ingested_samples: usize,
    output_state: PdfAdaptationImageOutputState,
    source_sampler_snapshot: SamplerAggregatorSnapshot,
    source_sampler: Box<dyn SamplerAggregator>,
}

pub struct PdfAdaptationRasterLineSampler {
    params: PdfAdaptationRasterLineSamplerParams,
    next_index: usize,
    stride: usize,
    ingested_samples: usize,
    output_state: PdfAdaptationImageOutputState,
    source_sampler_snapshot: SamplerAggregatorSnapshot,
    source_sampler: Box<dyn SamplerAggregator>,
}

impl RasterPlaneSampler {
    pub fn from_params_and_domain(
        params: RasterPlaneSamplerParams,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_plane_geometry(&params.geometry, domain)?;
        let total_samples = params.geometry.nr_points();
        Ok(Self {
            params,
            next_index: 0,
            stride: coprime_stride(total_samples),
        })
    }

    pub fn from_snapshot(
        snapshot: RasterPlaneSamplerSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        let sampler = Self::from_params_and_domain(snapshot.params, domain)?;
        Ok(Self {
            next_index: snapshot.next_index,
            stride: snapshot.stride,
            ..sampler
        })
    }

    fn total_samples(&self) -> usize {
        self.params.geometry.nr_points()
    }

    fn point_at(&self, index: usize) -> Vec<f64> {
        self.params.geometry.point_at(index)
    }

    fn permuted_index(&self, index: usize) -> usize {
        permuted_raster_index(index, self.total_samples(), self.stride)
    }
}

impl RasterLineSampler {
    pub fn from_params_and_domain(
        params: RasterLineSamplerParams,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_line_geometry(&params.geometry, domain)?;
        let total_samples = params.geometry.nr_points();
        Ok(Self {
            params,
            next_index: 0,
            stride: coprime_stride(total_samples),
        })
    }

    pub fn from_snapshot(
        snapshot: RasterLineSamplerSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        let sampler = Self::from_params_and_domain(snapshot.params, domain)?;
        Ok(Self {
            next_index: snapshot.next_index,
            stride: snapshot.stride,
            ..sampler
        })
    }

    fn total_samples(&self) -> usize {
        self.params.geometry.nr_points()
    }

    fn point_at(&self, index: usize) -> Vec<f64> {
        self.params.geometry.point_at(index)
    }

    fn permuted_index(&self, index: usize) -> usize {
        permuted_raster_index(index, self.total_samples(), self.stride)
    }
}

impl PdfAdaptationRasterPlaneSampler {
    pub fn from_params_and_snapshot(
        params: PdfAdaptationRasterPlaneSamplerParams,
        source_sampler_snapshot: SamplerAggregatorSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_plane_geometry(&params.geometry, domain)?;
        let total_samples = params.geometry.nr_points();
        let source_sampler = source_sampler_snapshot.clone().into_runtime(domain)?;
        Ok(Self {
            params,
            next_index: 0,
            stride: coprime_stride(total_samples),
            ingested_samples: 0,
            output_state: PdfAdaptationImageOutputState::new(total_samples),
            source_sampler_snapshot,
            source_sampler,
        })
    }

    pub fn from_snapshot(
        snapshot: PdfAdaptationRasterPlaneSamplerSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        let sampler = Self::from_params_and_snapshot(
            snapshot.params,
            snapshot.source_sampler_snapshot,
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

    fn total_samples(&self) -> usize {
        self.params.geometry.nr_points()
    }

    fn point_at(&self, index: usize) -> Vec<f64> {
        self.params.geometry.point_at(index)
    }

    fn permuted_index(&self, index: usize) -> usize {
        permuted_raster_index(index, self.total_samples(), self.stride)
    }

    fn output_for_frontend(&self) -> PdfAdaptationImagePersistedOutput {
        PdfAdaptationImagePersistedOutput {
            processed: self.ingested_samples,
            abs_integrand_mean: self.output_state.abs_integrand_mean(),
            signed_integrand_values: self.output_state.signed_integrand_values.clone(),
            abs_integrand_values: self.output_state.abs_integrand_values.clone(),
            pdf_values: self.output_state.pdf_values.clone(),
        }
    }

    fn record_training_value(
        &mut self,
        canonical_index: usize,
        training_value: f64,
    ) -> Result<(), EngineError> {
        if !training_value.is_finite() {
            self.output_state.signed_integrand_values[canonical_index] = None;
            self.output_state.abs_integrand_values[canonical_index] = None;
            self.output_state.pdf_values[canonical_index] = None;
            return Ok(());
        }

        let point = (
            self.params.geometry.discrete.clone(),
            self.point_at(canonical_index),
        );
        self.output_state.signed_integrand_values[canonical_index] = Some(training_value);
        self.output_state.abs_integrand_values[canonical_index] = Some(training_value.abs());
        self.output_state.pdf_values[canonical_index] = self
            .source_sampler
            .pdf(&point)?
            .filter(|pdf| pdf.is_finite());
        self.output_state.abs_integrand_sum += training_value.abs();
        self.output_state.abs_integrand_count += 1;
        Ok(())
    }
}

impl PdfAdaptationRasterLineSampler {
    pub fn from_params_and_snapshot(
        params: PdfAdaptationRasterLineSamplerParams,
        source_sampler_snapshot: SamplerAggregatorSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_line_geometry(&params.geometry, domain)?;
        let total_samples = params.geometry.nr_points();
        let source_sampler = source_sampler_snapshot.clone().into_runtime(domain)?;
        Ok(Self {
            params,
            next_index: 0,
            stride: coprime_stride(total_samples),
            ingested_samples: 0,
            output_state: PdfAdaptationImageOutputState::new(total_samples),
            source_sampler_snapshot,
            source_sampler,
        })
    }

    pub fn from_snapshot(
        snapshot: PdfAdaptationRasterLineSamplerSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        let sampler = Self::from_params_and_snapshot(
            snapshot.params,
            snapshot.source_sampler_snapshot,
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

    fn total_samples(&self) -> usize {
        self.params.geometry.nr_points()
    }

    fn point_at(&self, index: usize) -> Vec<f64> {
        self.params.geometry.point_at(index)
    }

    fn permuted_index(&self, index: usize) -> usize {
        permuted_raster_index(index, self.total_samples(), self.stride)
    }

    fn output_for_frontend(&self) -> PdfAdaptationImagePersistedOutput {
        PdfAdaptationImagePersistedOutput {
            processed: self.ingested_samples,
            abs_integrand_mean: self.output_state.abs_integrand_mean(),
            signed_integrand_values: self.output_state.signed_integrand_values.clone(),
            abs_integrand_values: self.output_state.abs_integrand_values.clone(),
            pdf_values: self.output_state.pdf_values.clone(),
        }
    }

    fn record_training_value(
        &mut self,
        canonical_index: usize,
        training_value: f64,
    ) -> Result<(), EngineError> {
        if !training_value.is_finite() {
            self.output_state.signed_integrand_values[canonical_index] = None;
            self.output_state.abs_integrand_values[canonical_index] = None;
            self.output_state.pdf_values[canonical_index] = None;
            return Ok(());
        }

        let point = (
            self.params.geometry.discrete.clone(),
            self.point_at(canonical_index),
        );
        self.output_state.signed_integrand_values[canonical_index] = Some(training_value);
        self.output_state.abs_integrand_values[canonical_index] = Some(training_value.abs());
        self.output_state.pdf_values[canonical_index] = self
            .source_sampler
            .pdf(&point)?
            .filter(|pdf| pdf.is_finite());
        self.output_state.abs_integrand_sum += training_value.abs();
        self.output_state.abs_integrand_count += 1;
        Ok(())
    }
}

impl PdfAdaptationImageOutputState {
    fn new(total_samples: usize) -> Self {
        Self {
            signed_integrand_values: vec![None; total_samples],
            abs_integrand_values: vec![None; total_samples],
            pdf_values: vec![None; total_samples],
            abs_integrand_sum: 0.0,
            abs_integrand_count: 0,
        }
    }

    fn abs_integrand_mean(&self) -> Option<f64> {
        (self.abs_integrand_count > 0)
            .then_some(self.abs_integrand_sum / self.abs_integrand_count as f64)
    }
}

impl SamplerAggregator for RasterPlaneSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        validate_plane_geometry(&self.params.geometry, domain)
    }

    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        let remaining = self.total_samples().saturating_sub(self.next_index);
        if remaining == 0 {
            Ok(SamplePlan::Pause)
        } else {
            Ok(SamplePlan::Produce {
                nr_samples: remaining,
            })
        }
    }

    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<LatentBatchSpec, EngineError> {
        let remaining = self.total_samples().saturating_sub(self.next_index);
        let nr_samples = nr_samples.min(remaining);
        if nr_samples == 0 {
            return Err(EngineError::engine(
                "raster plane sampler cannot produce an empty batch",
            ));
        }
        let batch = Batch::from_points((0..nr_samples).map(|row_idx| {
            Point::new(
                self.point_at(self.permuted_index(self.next_index + row_idx)),
                self.params.geometry.discrete.clone(),
                1.0,
            )
        }))
        .map_err(|err| EngineError::engine(err.to_string()))?;
        self.next_index += nr_samples;
        Ok(LatentBatchSpec::from_batch(&batch))
    }

    fn ingest_training_values(&mut self, _training_values: &[f64]) -> Result<(), EngineError> {
        Ok(())
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        Ok(SamplerAggregatorSnapshot::RasterPlane {
            raw: serde_json::to_value(RasterPlaneSamplerSnapshot {
                params: self.params.clone(),
                next_index: self.next_index,
                stride: self.stride,
            })
            .map_err(|err| EngineError::engine(err.to_string()))?,
        })
    }
}

impl SamplerAggregator for RasterLineSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        validate_line_geometry(&self.params.geometry, domain)
    }

    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        let remaining = self.total_samples().saturating_sub(self.next_index);
        if remaining == 0 {
            Ok(SamplePlan::Pause)
        } else {
            Ok(SamplePlan::Produce {
                nr_samples: remaining,
            })
        }
    }

    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<LatentBatchSpec, EngineError> {
        let remaining = self.total_samples().saturating_sub(self.next_index);
        let nr_samples = nr_samples.min(remaining);
        if nr_samples == 0 {
            return Err(EngineError::engine(
                "raster line sampler cannot produce an empty batch",
            ));
        }
        let batch = Batch::from_points((0..nr_samples).map(|row_idx| {
            Point::new(
                self.point_at(self.permuted_index(self.next_index + row_idx)),
                self.params.geometry.discrete.clone(),
                1.0,
            )
        }))
        .map_err(|err| EngineError::engine(err.to_string()))?;
        self.next_index += nr_samples;
        Ok(LatentBatchSpec::from_batch(&batch))
    }

    fn ingest_training_values(&mut self, _training_values: &[f64]) -> Result<(), EngineError> {
        Ok(())
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        Ok(SamplerAggregatorSnapshot::RasterLine {
            raw: serde_json::to_value(RasterLineSamplerSnapshot {
                params: self.params.clone(),
                next_index: self.next_index,
                stride: self.stride,
            })
            .map_err(|err| EngineError::engine(err.to_string()))?,
        })
    }
}

impl SamplerAggregator for PdfAdaptationRasterPlaneSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        validate_plane_geometry(&self.params.geometry, domain)
    }

    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        let remaining = self.total_samples().saturating_sub(self.next_index);
        if remaining == 0 {
            Ok(SamplePlan::Pause)
        } else {
            Ok(SamplePlan::Produce {
                nr_samples: remaining,
            })
        }
    }

    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<LatentBatchSpec, EngineError> {
        let remaining = self.total_samples().saturating_sub(self.next_index);
        let nr_samples = nr_samples.min(remaining);
        if nr_samples == 0 {
            return Err(EngineError::engine(
                "pdf adaptation raster plane sampler cannot produce an empty batch",
            ));
        }
        let batch = Batch::from_points((0..nr_samples).map(|row_idx| {
            Point::new(
                self.point_at(self.permuted_index(self.next_index + row_idx)),
                self.params.geometry.discrete.clone(),
                1.0,
            )
        }))
        .map_err(|err| EngineError::engine(err.to_string()))?;
        self.next_index += nr_samples;
        Ok(LatentBatchSpec::from_batch(&batch))
    }

    fn ingest_training_values(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        let total_samples = self.total_samples();
        if self.ingested_samples + training_values.len() > total_samples {
            return Err(EngineError::engine(format!(
                "pdf adaptation raster plane sampler ingest overflow: {} + {} exceeds {}",
                self.ingested_samples,
                training_values.len(),
                total_samples,
            )));
        }
        for (offset, training_value) in training_values.iter().copied().enumerate() {
            let shuffled_index = self.ingested_samples + offset;
            let canonical_index = self.permuted_index(shuffled_index);
            self.record_training_value(canonical_index, training_value)?;
        }
        self.ingested_samples += training_values.len();
        Ok(())
    }

    fn pdf(&mut self, point: &PdfPoint) -> Result<Option<f64>, EngineError> {
        self.source_sampler.pdf(point)
    }

    fn persisted_output(&mut self) -> Result<Option<serde_json::Value>, EngineError> {
        serde_json::to_value(self.output_for_frontend())
            .map(Some)
            .map_err(|err| EngineError::engine(err.to_string()))
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        Ok(SamplerAggregatorSnapshot::PdfAdaptationRasterPlane {
            raw: serde_json::to_value(PdfAdaptationRasterPlaneSamplerSnapshot {
                params: self.params.clone(),
                next_index: self.next_index,
                stride: self.stride,
                ingested_samples: self.ingested_samples,
                output_state: self.output_state.clone(),
                source_sampler_snapshot: self.source_sampler_snapshot.clone(),
            })
            .map_err(|err| EngineError::engine(err.to_string()))?,
        })
    }
}

impl SamplerAggregator for PdfAdaptationRasterLineSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        validate_line_geometry(&self.params.geometry, domain)
    }

    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        let remaining = self.total_samples().saturating_sub(self.next_index);
        if remaining == 0 {
            Ok(SamplePlan::Pause)
        } else {
            Ok(SamplePlan::Produce {
                nr_samples: remaining,
            })
        }
    }

    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<LatentBatchSpec, EngineError> {
        let remaining = self.total_samples().saturating_sub(self.next_index);
        let nr_samples = nr_samples.min(remaining);
        if nr_samples == 0 {
            return Err(EngineError::engine(
                "pdf adaptation raster line sampler cannot produce an empty batch",
            ));
        }
        let batch = Batch::from_points((0..nr_samples).map(|row_idx| {
            Point::new(
                self.point_at(self.permuted_index(self.next_index + row_idx)),
                self.params.geometry.discrete.clone(),
                1.0,
            )
        }))
        .map_err(|err| EngineError::engine(err.to_string()))?;
        self.next_index += nr_samples;
        Ok(LatentBatchSpec::from_batch(&batch))
    }

    fn ingest_training_values(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        let total_samples = self.total_samples();
        if self.ingested_samples + training_values.len() > total_samples {
            return Err(EngineError::engine(format!(
                "pdf adaptation raster line sampler ingest overflow: {} + {} exceeds {}",
                self.ingested_samples,
                training_values.len(),
                total_samples,
            )));
        }
        for (offset, training_value) in training_values.iter().copied().enumerate() {
            let shuffled_index = self.ingested_samples + offset;
            let canonical_index = self.permuted_index(shuffled_index);
            self.record_training_value(canonical_index, training_value)?;
        }
        self.ingested_samples += training_values.len();
        Ok(())
    }

    fn pdf(&mut self, point: &PdfPoint) -> Result<Option<f64>, EngineError> {
        self.source_sampler.pdf(point)
    }

    fn persisted_output(&mut self) -> Result<Option<serde_json::Value>, EngineError> {
        serde_json::to_value(self.output_for_frontend())
            .map(Some)
            .map_err(|err| EngineError::engine(err.to_string()))
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        Ok(SamplerAggregatorSnapshot::PdfAdaptationRasterLine {
            raw: serde_json::to_value(PdfAdaptationRasterLineSamplerSnapshot {
                params: self.params.clone(),
                next_index: self.next_index,
                stride: self.stride,
                ingested_samples: self.ingested_samples,
                output_state: self.output_state.clone(),
                source_sampler_snapshot: self.source_sampler_snapshot.clone(),
            })
            .map_err(|err| EngineError::engine(err.to_string()))?,
        })
    }
}

fn validate_plane_geometry(
    geometry: &PlaneRasterGeometry,
    domain: &Domain,
) -> Result<(), BuildError> {
    let (continuous_dims, discrete_dims) = domain.fixed_rectangular_dims().ok_or_else(|| {
        BuildError::incompatible("plane geometry requires a fixed rectangular domain")
    })?;
    geometry.validate().map_err(BuildError::invalid_input)?;
    if geometry.offset.len() != continuous_dims {
        return Err(BuildError::incompatible(format!(
            "plane geometry continuous dimension mismatch: expected {}, got {}",
            continuous_dims,
            geometry.offset.len()
        )));
    }
    if geometry.discrete.len() != discrete_dims {
        return Err(BuildError::incompatible(format!(
            "plane geometry discrete dimension mismatch: expected {}, got {}",
            discrete_dims,
            geometry.discrete.len()
        )));
    }
    Ok(())
}

fn validate_line_geometry(
    geometry: &LineRasterGeometry,
    domain: &Domain,
) -> Result<(), BuildError> {
    let (continuous_dims, discrete_dims) = domain.fixed_rectangular_dims().ok_or_else(|| {
        BuildError::incompatible("line geometry requires a fixed rectangular domain")
    })?;
    geometry.validate().map_err(BuildError::invalid_input)?;
    if geometry.offset.len() != continuous_dims {
        return Err(BuildError::incompatible(format!(
            "line geometry continuous dimension mismatch: expected {}, got {}",
            continuous_dims,
            geometry.offset.len()
        )));
    }
    if geometry.discrete.len() != discrete_dims {
        return Err(BuildError::incompatible(format!(
            "line geometry discrete dimension mismatch: expected {}, got {}",
            discrete_dims,
            geometry.discrete.len()
        )));
    }
    Ok(())
}

fn permuted_raster_index(index: usize, total_samples: usize, stride: usize) -> usize {
    if total_samples <= 1 {
        return index.min(total_samples.saturating_sub(1));
    }
    (index * stride) % total_samples
}

fn coprime_stride(total_samples: usize) -> usize {
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
        PdfAdaptationRasterPlaneSampler, PdfAdaptationRasterPlaneSamplerParams, RasterLineSampler,
        RasterLineSamplerParams, coprime_stride, permuted_raster_index,
    };
    use crate::core::{LineRasterGeometry, Linspace, PlaneRasterGeometry, SamplerAggregatorConfig};
    use crate::sampling::SamplerAggregator;
    use crate::utils::domain::Domain;
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
        let first_batch = sampler.produce_latent_batch(2).expect("first batch");
        let snapshot = sampler.snapshot().expect("snapshot");
        let restored_snapshot = match snapshot {
            crate::sampling::SamplerAggregatorSnapshot::RasterLine { raw } => {
                serde_json::from_value(raw).expect("decode raster line snapshot")
            }
            other => panic!("unexpected snapshot kind: {other:?}"),
        };
        let mut restored =
            RasterLineSampler::from_snapshot(restored_snapshot, &domain).expect("restore");
        let second_batch = restored.produce_latent_batch(3).expect("second batch");

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

    #[test]
    fn pdf_adaptation_raster_plane_delegates_pdf_to_source_sampler() {
        let domain = Domain::rectangular(2, 0);
        let source_snapshot = SamplerAggregatorConfig::NaiveMonteCarlo {
            params: crate::sampling::NaiveMonteCarloSamplerParams::default(),
        }
        .build(domain.clone(), None, None)
        .expect("build source sampler")
        .snapshot()
        .expect("source snapshot");

        let mut sampler = PdfAdaptationRasterPlaneSampler::from_params_and_snapshot(
            PdfAdaptationRasterPlaneSamplerParams {
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
            &domain,
        )
        .expect("build pdf adaptation sampler");

        assert_eq!(
            sampler.pdf(&(Vec::new(), vec![0.25, 0.75])).unwrap(),
            Some(1.0)
        );
    }

    #[test]
    fn pdf_adaptation_raster_plane_persists_integrand_and_pdf_images() {
        let domain = Domain::rectangular(2, 0);
        let source_snapshot = SamplerAggregatorConfig::NaiveMonteCarlo {
            params: crate::sampling::NaiveMonteCarloSamplerParams::default(),
        }
        .build(domain.clone(), None, None)
        .expect("build source sampler")
        .snapshot()
        .expect("source snapshot");

        let mut sampler = PdfAdaptationRasterPlaneSampler::from_params_and_snapshot(
            PdfAdaptationRasterPlaneSamplerParams {
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
            &domain,
        )
        .expect("build pdf adaptation sampler");

        sampler
            .ingest_training_values(&[2.0, 4.0])
            .expect("ingest weights");
        let output = sampler
            .persisted_output()
            .expect("persisted output")
            .expect("custom payload");
        let output: super::PdfAdaptationImagePersistedOutput =
            serde_json::from_value(output).expect("decode payload");

        assert_eq!(output.processed, 2);
        assert_eq!(output.abs_integrand_mean, Some(3.0));
        assert_eq!(output.signed_integrand_values, vec![Some(2.0), Some(4.0)]);
        assert_eq!(output.abs_integrand_values, vec![Some(2.0), Some(4.0)]);
        assert_eq!(output.pdf_values, vec![Some(1.0), Some(1.0)]);
    }

    #[test]
    fn pdf_adaptation_raster_plane_ignores_non_finite_training_values() {
        let domain = Domain::rectangular(2, 0);
        let source_snapshot = SamplerAggregatorConfig::NaiveMonteCarlo {
            params: crate::sampling::NaiveMonteCarloSamplerParams::default(),
        }
        .build(domain.clone(), None, None)
        .expect("build source sampler")
        .snapshot()
        .expect("source snapshot");

        let mut sampler = PdfAdaptationRasterPlaneSampler::from_params_and_snapshot(
            PdfAdaptationRasterPlaneSamplerParams {
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
            &domain,
        )
        .expect("build pdf adaptation sampler");

        sampler
            .ingest_training_values(&[f64::NAN, 4.0])
            .expect("ingest weights");
        let output = sampler
            .persisted_output()
            .expect("persisted output")
            .expect("custom payload");
        let output: super::PdfAdaptationImagePersistedOutput =
            serde_json::from_value(output).expect("decode payload");

        assert_eq!(output.processed, 2);
        assert_eq!(output.abs_integrand_mean, Some(4.0));
        assert_eq!(output.signed_integrand_values, vec![None, Some(4.0)]);
        assert_eq!(output.abs_integrand_values, vec![None, Some(4.0)]);
        assert_eq!(output.pdf_values, vec![None, Some(1.0)]);
    }
}
