mod evaluator;
mod protocol;

use evaluator::BranchingDomainEvaluator;
use protocol::{read_request, send_error, send_result, EvalBatchParams, InitializeParams};

fn main() {
    let mut evaluator: Option<BranchingDomainEvaluator> = None;
    let mut fixed_widths: Option<(Option<usize>, Option<usize>)> = None;

    loop {
        let request = match read_request() {
            Ok(Some(request)) => request,
            Ok(None) => break,
            Err(error) => {
                eprintln!("failed to read request: {error}");
                break;
            }
        };

        let result: Result<(serde_json::Value, Vec<u8>), String> = match request.method.as_str() {
            "initialize" => {
                let params: Result<InitializeParams, _> = serde_json::from_value(request.params);
                match params.and_then(|params| {
                    if params.protocol != "gammaboard-jsonrpc-v2" {
                        return Err(serde_json::Error::io(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!("unsupported protocol: {}", params.protocol),
                        )));
                    }
                    if params.role != "evaluator" {
                        return Err(serde_json::Error::io(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!("expected evaluator role, got {}", params.role),
                        )));
                    }
                    if params.components.len() > 1 {
                        return Err(serde_json::Error::io(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "rust ragged-domain evaluator example returns exactly one component",
                        )));
                    }
                    fixed_widths = Some(domain_fixed_widths(&params.domain));
                    evaluator = Some(BranchingDomainEvaluator::from_args(&params.args).map_err(
                        |error| {
                            serde_json::Error::io(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                error,
                            ))
                        },
                    )?);
                    Ok((serde_json::json!({ "ok": true }), Vec::new()))
                }) {
                    Ok(value) => Ok(value),
                    Err(error) => Err(error.to_string()),
                }
            }
            "eval_batch" => {
                let params: Result<EvalBatchParams, _> = serde_json::from_value(request.params);
                match (evaluator.as_ref(), fixed_widths, params) {
                    (None, _, _) | (_, None, _) => Err("worker not initialized".to_string()),
                    (Some(evaluator), Some(widths), Ok(params)) => {
                        decode_eval_batch(params, &request.binary, widths).and_then(
                            |(
                                discrete,
                                discrete_offsets,
                                continuous,
                                continuous_offsets,
                                nr_samples,
                            )| {
                                evaluator
                                    .eval_batch(
                                        &discrete,
                                        &discrete_offsets,
                                        &continuous,
                                        &continuous_offsets,
                                        nr_samples,
                                    )
                                    .map(|values| {
                                        let binary = values
                                            .iter()
                                            .flat_map(|value| value.to_le_bytes())
                                            .collect();
                                        (serde_json::json!({}), binary)
                                    })
                            },
                        )
                    }
                    (_, _, Err(error)) => Err(error.to_string()),
                }
            }
            other => Err(format!("unknown method: {other}")),
        };

        match result {
            Ok((value, binary)) => {
                if let Err(error) = send_result(request.id, value, &binary) {
                    eprintln!("failed to send result: {error}");
                    break;
                }
            }
            Err(error) => {
                if let Err(send_error) = send_error(request.id, &error) {
                    eprintln!("failed to send error response: {send_error}");
                    break;
                }
            }
        }
    }
}

type DecodedBatch = (Vec<i64>, Vec<usize>, Vec<f64>, Vec<usize>, usize);

fn decode_eval_batch(
    params: EvalBatchParams,
    binary: &[u8],
    fixed_widths: (Option<usize>, Option<usize>),
) -> Result<DecodedBatch, String> {
    let discrete_offsets = resolve_offsets(
        params.xs_discrete_offsets,
        params.nr_samples,
        fixed_widths.0,
        "discrete",
    )?;
    let continuous_offsets = resolve_offsets(
        params.xs_continuous_offsets,
        params.nr_samples,
        fixed_widths.1,
        "continuous",
    )?;
    let discrete_len = *discrete_offsets.last().unwrap_or(&0);
    let continuous_len = *continuous_offsets.last().unwrap_or(&0);
    let expected_bytes = (discrete_len + continuous_len) * 8;
    if binary.len() != expected_bytes {
        return Err(format!(
            "eval_batch binary length mismatch: expected {expected_bytes}, got {}",
            binary.len()
        ));
    }
    let split = discrete_len * 8;
    let discrete = binary[..split]
        .chunks_exact(8)
        .map(|chunk| i64::from_le_bytes(chunk.try_into().expect("8-byte chunk")))
        .collect();
    let continuous = binary[split..]
        .chunks_exact(8)
        .map(|chunk| f64::from_le_bytes(chunk.try_into().expect("8-byte chunk")))
        .collect();
    Ok((
        discrete,
        discrete_offsets,
        continuous,
        continuous_offsets,
        params.nr_samples,
    ))
}

fn resolve_offsets(
    offsets: Option<Vec<usize>>,
    nr_samples: usize,
    fixed_width: Option<usize>,
    label: &str,
) -> Result<Vec<usize>, String> {
    match offsets {
        Some(offsets) => Ok(offsets),
        None => fixed_width
            .map(|width| (0..=nr_samples).map(|index| index * width).collect())
            .ok_or_else(|| format!("missing xs_{label}_offsets for ragged domain")),
    }
}

fn domain_fixed_widths(domain: &serde_json::Value) -> (Option<usize>, Option<usize>) {
    if let Some(continuous) = domain.get("continuous") {
        return (
            Some(0),
            continuous
                .get("dims")
                .and_then(serde_json::Value::as_u64)
                .map(|v| v as usize),
        );
    }
    if let Some(rectangular) = domain.get("rectangular") {
        return (
            rectangular
                .get("discrete_cardinalities")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            rectangular
                .get("continuous_dims")
                .and_then(serde_json::Value::as_u64)
                .map(|v| v as usize),
        );
    }
    let Some(branches) = domain
        .get("discrete")
        .and_then(|value| value.get("branches"))
        .and_then(serde_json::Value::as_array)
    else {
        return (None, None);
    };
    let widths: Vec<_> = branches
        .iter()
        .filter_map(|branch| branch.get("domain"))
        .map(domain_fixed_widths)
        .collect();
    let continuous = common_width(widths.iter().map(|width| width.1));
    let discrete = common_width(widths.iter().map(|width| width.0)).map(|width| width + 1);
    (discrete, continuous)
}

fn common_width(mut widths: impl Iterator<Item = Option<usize>>) -> Option<usize> {
    let first = widths.next()??;
    widths.all(|width| width == Some(first)).then_some(first)
}
