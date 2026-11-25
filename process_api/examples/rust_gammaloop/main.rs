//! A process boundary around the same evaluator used by the native backend.
mod batch;
#[path = "../common/wire.rs"]
mod wire;

use anyhow::{Context, Result, ensure};
use clap::{Parser, ValueEnum};
use gammaboard::evaluation::observable_wire::{ENCODING, Encoder, ObservableResponse};
use gammaboard::evaluation::{GammaLoopParams, ProcessOutput};
use gammaboard::{Domain, Evaluator, EvaluatorConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io::Write, path::PathBuf};

#[derive(Parser)]
#[command(about = "GammaLoop evaluator over GammaBoard's stdio process protocol")]
struct Args {
    /// Read GammaLoop parameters from JSON and print the inferred domain and metadata.
    #[arg(long, value_name = "ARGS_JSON")]
    describe: Option<PathBuf>,
    /// Observable transport preference; JSON also supports changing layouts.
    #[arg(long, value_enum, default_value = "binary")]
    observable_format: ObservableFormat,
}

#[derive(Clone, Copy, ValueEnum)]
enum ObservableFormat {
    Binary,
    Json,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Reply {
    Initialized {
        ok: bool,
        metadata: Value,
        observable_encodings: Vec<&'static str>,
        evaluation_protocol: &'static str,
        descriptor: gammaboard::EvaluatorDescriptor,
        evaluation_encodings: Vec<&'static str>,
    },
    Evaluation(ObservableResponse),
    Selected(gammaboard::evaluation::wire::EvaluationResponse),
    Diagnostics {
        diagnostics: Value,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Initialize {
    protocol: String,
    role: String,
    #[serde(default)]
    domain: Option<Domain>,
    #[serde(default)]
    output: ProcessOutput,
    args: GammaLoopParams,
}

struct Worker {
    evaluator: Box<dyn Evaluator>,
    output: ProcessOutput,
    domain: Domain,
    binary_enabled: bool,
    encoder: Encoder,
    schema_sent: bool,
    selected_encoder: gammaboard::evaluation::wire::Encoder,
}

fn build(params: GammaLoopParams) -> Result<Box<dyn Evaluator>> {
    ensure!(
        params.state_folder.is_absolute(),
        "args.state_folder must be an absolute path on the worker host"
    );
    Ok(EvaluatorConfig::Gammaloop { params }.build()?)
}

impl Worker {
    fn initialize(params: Initialize, binary_enabled: bool) -> Result<Self> {
        ensure!(
            params.protocol == "gammaboard-jsonrpc-v10",
            "unsupported protocol"
        );
        ensure!(params.role == "evaluator", "expected evaluator role");
        if let Some(domain) = &params.domain {
            domain.validate().map_err(anyhow::Error::msg)?;
        }
        if let ProcessOutput::Values { components } = &params.output {
            ensure!(
                components == &["value"] || components == &["real", "imag"],
                "GammaLoop output requires [value] or [real, imag] components"
            );
        }
        let evaluator = build(params.args)?;
        let domain = evaluator.domain();
        ensure!(
            params
                .domain
                .as_ref()
                .is_none_or(|expected| expected == &domain),
            "configured domain differs from the GammaLoop domain; use --describe to obtain it: {}",
            serde_json::to_string(&domain)?
        );
        Ok(Self {
            evaluator,
            output: params.output,
            domain,
            binary_enabled,
            encoder: Default::default(),
            schema_sent: false,
            selected_encoder: Default::default(),
        })
    }

    fn evaluate_selected(&mut self, mut params: Value, binary: &[u8]) -> Result<(Reply, Vec<u8>)> {
        let fields = params
            .as_object_mut()
            .context("evaluation params must be an object")?;
        let outputs: gammaboard::EvaluationRequest = serde_json::from_value(
            fields
                .remove("outputs")
                .context("missing requested outputs")?,
        )?;
        let encoding = fields
            .remove("encoding")
            .and_then(|v| v.as_str().map(str::to_owned))
            .context("missing encoding")?;
        ensure!(
            encoding == "json"
                || (self.binary_enabled && encoding == gammaboard::evaluation::wire::ENCODING),
            "unsupported evaluation encoding"
        );
        fields.insert("components".into(), json!(outputs.components));
        let params: batch::EvalBatch = serde_json::from_value(params)?;
        let batch = params.decode(binary, &self.domain)?;
        let mut samples = Vec::new();
        let mut bytes = Vec::new();
        gammaboard::evaluate_requested(self.evaluator.as_mut(), &batch, &outputs, &mut |sample| {
            if encoding == "json" {
                samples.push(sample);
                Ok(())
            } else {
                self.selected_encoder.append(&sample, &outputs, &mut bytes)
            }
        })?;
        let response = if encoding == "json" {
            gammaboard::evaluation::wire::EvaluationResponse {
                samples: Some(samples),
                ..Default::default()
            }
        } else {
            self.selected_encoder.response()?
        };
        Ok((Reply::Selected(response), bytes))
    }

    fn evaluate(&mut self, params: Value, binary: &[u8]) -> Result<(Reply, Vec<u8>)> {
        let params: batch::EvalBatch = serde_json::from_value(params)?;
        let components = match &self.output {
            ProcessOutput::Values { components } => components.clone(),
            ProcessOutput::Gammaloop { .. } => vec!["real".into(), "imag".into()],
        };
        ensure!(
            params.components == components,
            "components differ from initialization"
        );
        let compact = match params.observable_encoding.as_deref() {
            None => false,
            Some(ENCODING)
                if self.binary_enabled
                    && matches!(self.output, ProcessOutput::Gammaloop { .. }) =>
            {
                true
            }
            Some(_) => anyhow::bail!("unsupported observable encoding"),
        };
        let batch = params.decode(binary, &self.domain)?;
        let include_observables = matches!(self.output, ProcessOutput::Gammaloop { .. });
        let mut values = Vec::with_capacity(batch.len() * components.len());
        let mut observables = Vec::new();
        let mut binary_output = Vec::new();
        let mut emitted = 0;
        let outputs = gammaboard::EvaluationRequest {
            components: components.clone(),
            observables: include_observables,
            details: false,
            diagnostics: false,
        };
        gammaboard::evaluate_requested(self.evaluator.as_mut(), &batch, &outputs, &mut |sample| {
            emitted += 1;
            if compact {
                return self.encoder.append(
                    &sample.values,
                    sample.observables.as_ref().ok_or_else(|| {
                        gammaboard::EngineError::eval("missing GammaLoop observable bundle")
                    })?,
                    &mut binary_output,
                );
            }
            values.extend(sample.values);
            if include_observables {
                observables.push(sample.observables.ok_or_else(|| {
                    gammaboard::EngineError::eval("missing GammaLoop observable bundle")
                })?);
            }
            Ok(())
        })?;
        ensure!(emitted == batch.len(), "incorrect result sample count");
        if compact {
            let schema = (!self.schema_sent)
                .then(|| self.encoder.schema().cloned())
                .flatten();
            self.schema_sent |= schema.is_some();
            return Ok((
                Reply::Evaluation(ObservableResponse {
                    observable_encoding: Some(ENCODING.into()),
                    observable_schema: schema,
                    ..Default::default()
                }),
                binary_output,
            ));
        }
        let result = Reply::Evaluation(ObservableResponse {
            observables: include_observables.then_some(observables),
            ..Default::default()
        });
        ensure!(
            values.len() == batch.len() * components.len(),
            "incorrect result size"
        );
        ensure!(
            values.iter().all(|v| v.is_finite()),
            "non-finite evaluation result"
        );
        Ok((
            result,
            values.into_iter().flat_map(f64::to_le_bytes).collect(),
        ))
    }
}

fn serve(writer: &mut impl Write, binary_enabled: bool) -> Result<()> {
    let mut reader = std::io::stdin().lock();
    let mut worker: Option<Worker> = None;
    while let Some(request) = wire::read_request(&mut reader).map_err(anyhow::Error::msg)? {
        let result = (|| -> Result<(Reply, Vec<u8>)> {
            match request.method.as_str() {
                "initialize" => {
                    ensure!(worker.is_none(), "worker is already initialized");
                    ensure!(request.binary.is_empty(), "initialize takes no binary data");
                    let initialized = Worker::initialize(
                        serde_json::from_value(request.params)?,
                        binary_enabled,
                    )?;
                    let metadata = initialized.evaluator.metadata();
                    let observable_encodings = if binary_enabled
                        && matches!(initialized.output, ProcessOutput::Gammaloop { .. })
                    {
                        vec![ENCODING]
                    } else {
                        vec![]
                    };
                    let descriptor = initialized.evaluator.descriptor();
                    worker = Some(initialized);
                    Ok((
                        Reply::Initialized {
                            ok: true,
                            metadata,
                            observable_encodings,
                            evaluation_protocol:
                                gammaboard::evaluation::contract::EVALUATION_PROTOCOL,
                            descriptor,
                            evaluation_encodings: if binary_enabled {
                                vec![gammaboard::evaluation::wire::ENCODING, "json"]
                            } else {
                                vec!["json"]
                            },
                        },
                        vec![],
                    ))
                }
                "evaluate" => worker
                    .as_mut()
                    .context("worker not initialized")?
                    .evaluate_selected(request.params, &request.binary),
                "eval_batch" => worker
                    .as_mut()
                    .context("worker not initialized")?
                    .evaluate(request.params, &request.binary),
                "diagnostics" => Ok((
                    Reply::Diagnostics {
                        diagnostics: worker
                            .as_mut()
                            .context("worker not initialized")?
                            .evaluator
                            .diagnostics(),
                    },
                    vec![],
                )),
                other => anyhow::bail!("unknown method: {other}"),
            }
        })();
        match result {
            Ok((value, binary)) => wire::send_result(writer, request.id, value, &binary),
            Err(error) => wire::send_error(writer, request.id, &format!("{error:#}")),
        }
        .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

// Native libraries may print during initialization, compilation or evaluation.
// Keep a private protocol descriptor and redirect all ordinary stdout to stderr.
#[cfg(unix)]
fn protocol_stdout() -> std::io::Result<std::fs::File> {
    use std::os::fd::FromRawFd;
    std::io::stdout().flush()?;
    // SAFETY: duplicate an existing descriptor; File owns the new descriptor.
    let fd = unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let output = unsafe { std::fs::File::from_raw_fd(fd) };
    // SAFETY: both standard descriptors are open; no Rust references are invalidated.
    if unsafe { libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(output)
}

#[cfg(not(unix))]
fn protocol_stdout() -> std::io::Result<std::fs::File> {
    Err(std::io::Error::other(
        "this example requires Unix stdout redirection",
    ))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut writer = protocol_stdout()?;
    if let Some(path) = args.describe {
        let evaluator = build(serde_json::from_slice(&std::fs::read(path)?)?)?;
        serde_json::to_writer_pretty(
            &mut writer,
            &json!({
                "domain": evaluator.domain(), "metadata": evaluator.metadata(), "descriptor": evaluator.descriptor(), "evaluation_protocol": gammaboard::evaluation::contract::EVALUATION_PROTOCOL,
            }),
        )?;
        writeln!(writer)?;
        Ok(())
    } else {
        serve(
            &mut writer,
            matches!(args.observable_format, ObservableFormat::Binary),
        )
    }
}
