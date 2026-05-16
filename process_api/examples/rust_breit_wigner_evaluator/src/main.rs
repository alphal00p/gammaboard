mod evaluator;
mod protocol;

use evaluator::BranchingDomainEvaluator;
use protocol::{read_request, send_error, send_result, EvalBatchParams, InitializeParams};

fn main() {
    let mut evaluator: Option<BranchingDomainEvaluator> = None;

    loop {
        let request = match read_request() {
            Ok(Some(request)) => request,
            Ok(None) => break,
            Err(error) => {
                eprintln!("failed to read request: {error}");
                break;
            }
        };

        let result = match request.method.as_str() {
            "initialize" => {
                let params: Result<InitializeParams, _> = serde_json::from_value(request.params);
                match params.and_then(|params| {
                    if params.protocol != "gammaboard-jsonrpc-v1" {
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
                    let _domain = params.domain;
                    evaluator = Some(BranchingDomainEvaluator::from_args(&params.args).map_err(
                        |error| {
                            serde_json::Error::io(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                error,
                            ))
                        },
                    )?);
                    Ok(serde_json::json!({ "ok": true }))
                }) {
                    Ok(value) => Ok(value),
                    Err(error) => Err(error.to_string()),
                }
            }
            "eval_batch" => {
                let params: Result<EvalBatchParams, _> = serde_json::from_value(request.params);
                match (evaluator.as_ref(), params) {
                    (None, _) => Err("worker not initialized".to_string()),
                    (Some(evaluator), Ok(params)) => evaluator
                        .eval_batch(
                            &params.xs_discrete_row_major,
                            &params.xs_discrete_offsets,
                            &params.xs_continuous_row_major,
                            &params.xs_continuous_offsets,
                            params.nr_samples,
                        )
                        .map(|values| serde_json::json!({ "values_row_major": values })),
                    (_, Err(error)) => Err(error.to_string()),
                }
            }
            other => Err(format!("unknown method: {other}")),
        };

        match result {
            Ok(value) => {
                if let Err(error) = send_result(request.id, value) {
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
