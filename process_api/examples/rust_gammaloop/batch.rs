use anyhow::{Result, ensure};
use gammaboard::{Batch, Domain, Point};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalBatch {
    pub nr_samples: usize,
    pub components: Vec<String>,
    #[serde(default)]
    pub observable_encoding: Option<String>,
    xs_discrete_offsets: Option<Vec<usize>>,
    xs_continuous_offsets: Option<Vec<usize>>,
}

impl EvalBatch {
    pub fn decode(self, binary: &[u8], domain: &Domain) -> Result<Batch> {
        // Check sizes before allocating offsets, including a malicious sample count.
        ensure!(
            self.nr_samples <= 10_000_000,
            "batch exceeds example's sample limit"
        );
        let discrete = offsets(
            self.xs_discrete_offsets,
            self.nr_samples,
            domain.fixed_discrete_depth(),
        )?;
        let continuous = offsets(
            self.xs_continuous_offsets,
            self.nr_samples,
            domain.fixed_continuous_dims(),
        )?;
        let discrete_len = discrete[self.nr_samples];
        let expected = discrete_len
            .checked_add(continuous[self.nr_samples])
            .and_then(|n| n.checked_mul(8));
        ensure!(
            expected == Some(binary.len()),
            "binary length does not match coordinates"
        );
        let (discrete_bytes, continuous_bytes) = binary.split_at(discrete_len * 8);
        let ds: Vec<_> = discrete_bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|v| i64::from_le_bytes(*v))
            .collect();
        let xs: Vec<_> = continuous_bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|v| f64::from_le_bytes(*v))
            .collect();
        ensure!(xs.iter().all(|v| v.is_finite()), "non-finite coordinate");
        let batch = Batch::from_iter((0..self.nr_samples).map(|i| {
            Point::new(
                xs[continuous[i]..continuous[i + 1]].to_vec(),
                ds[discrete[i]..discrete[i + 1]].to_vec(),
                1.0, // Sampling weights belong to the host, never to this process.
            )
        }));
        domain.validate_batch(&batch).map_err(anyhow::Error::msg)?;
        Ok(batch)
    }
}

fn offsets(explicit: Option<Vec<usize>>, count: usize, width: Option<usize>) -> Result<Vec<usize>> {
    if let Some(offsets) = explicit {
        ensure!(
            offsets.len() == count + 1,
            "offsets require nr_samples + 1 entries"
        );
        ensure!(offsets[0] == 0, "offsets must start at zero");
        ensure!(
            offsets.windows(2).all(|v| v[0] <= v[1]),
            "offsets must be non-decreasing"
        );
        Ok(offsets)
    } else {
        let width = width.ok_or_else(|| anyhow::anyhow!("ragged coordinates require offsets"))?;
        ensure!(
            count.checked_mul(width).is_some(),
            "coordinate count overflow"
        );
        Ok((0..=count).map(|i| i * width).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_ragged_unit_weight_samples_and_rejects_bad_layouts() {
        let domain: Domain = serde_json::from_value(json!({"discrete": {"branches": [
            {"index":0,"domain":{"rectangular":{"continuous_dims":1,"discrete_cardinalities":[]}}},
            {"index":1,"domain":{"rectangular":{"continuous_dims":2,"discrete_cardinalities":[]}}}
        ]}}))
        .unwrap();
        let params =
            json!({"nr_samples":2,"components":["real","imag"],"xs_continuous_offsets":[0,1,3]});
        let bytes: Vec<_> = [0_i64, 1]
            .into_iter()
            .flat_map(i64::to_le_bytes)
            .chain([0.2_f64, 0.3, 0.4].into_iter().flat_map(f64::to_le_bytes))
            .collect();
        let decode = |p| {
            serde_json::from_value::<EvalBatch>(p)
                .unwrap()
                .decode(&bytes, &domain)
        };
        let batch = decode(params.clone()).unwrap();
        assert_eq!(batch.points()[1], Point::new(vec![0.3, 0.4], vec![1], 1.0));
        for bad in [
            json!([1, 1, 3]),
            json!([0, 3, 1]),
            json!([0, 1]),
            json!([0, 1, 4]),
            json!([0, 2, 3]),
        ] {
            let mut p = params.clone();
            p["xs_continuous_offsets"] = bad;
            assert!(decode(p).is_err());
        }
        let mut missing = params;
        missing
            .as_object_mut()
            .unwrap()
            .remove("xs_continuous_offsets");
        assert!(decode(missing).is_err());
    }
}
