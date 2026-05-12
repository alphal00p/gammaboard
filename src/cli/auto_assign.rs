use super::shared::{resolve_run_ref, with_cli_store};
use anyhow::Result;
use clap::Args;
use gammaboard::api::nodes as node_api;
use gammaboard::config::RuntimeConfig;

#[derive(Debug, Args)]
pub struct AutoAssignArgs {
    pub run: String,
    pub max_evaluators: Option<usize>,
}

pub async fn run_auto_assign_command(
    args: AutoAssignArgs,
    config: &RuntimeConfig,
    quiet: bool,
) -> Result<()> {
    let span = tracing::span!(
        tracing::Level::TRACE,
        "control_auto_assign_command",
        source = "control",
        command = "auto_assign",
        run = args.run
    );

    with_cli_store(config, 10, quiet, span, |store| async move {
        let run = resolve_run_ref(&store, &args.run).await?;
        let assigned = node_api::auto_assign_run(&store, run.run_id, args.max_evaluators).await?;

        println!(
            "auto-assign completed: run_id={} run_name={} sampler_already_assigned={} assigned_sampler={} assigned_evaluators={} requested_evaluator_limit={}",
            assigned.run_id,
            assigned.run_name,
            assigned.sampler_already_assigned,
            assigned.assigned_sampler.as_deref().unwrap_or("none"),
            assigned.assigned_evaluators.len(),
            args.max_evaluators
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string())
        );
        Ok(())
    })
    .await
}
