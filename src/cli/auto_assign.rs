use super::shared::{resolve_run_ref, with_cli_store};
use anyhow::Result;
use clap::Args;
use gammaboard::config::RuntimeConfig;
use gammaboard::core::{ControlPlaneStore, WorkerRole};

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
        let nodes = store.list_nodes(None).await?;
        let free_nodes = nodes
            .iter()
            .filter(|node| node.desired_assignment.is_none())
            .map(|node| node.name.clone())
            .collect::<Vec<_>>();
        let sampler_already_assigned = nodes.iter().any(|node| {
            node.desired_assignment.as_ref().is_some_and(|assignment| {
                assignment.run_id == run.run_id && assignment.role == WorkerRole::SamplerAggregator
            })
        });

        let evaluator_limit = args.max_evaluators.unwrap_or(usize::MAX);
        let mut assigned_sampler = None;
        let mut assigned_evaluators = Vec::new();
        let mut free_iter = free_nodes.into_iter();

        if !sampler_already_assigned {
            if let Some(node_name) = free_iter.next() {
                store
                    .upsert_desired_assignment(
                        &node_name,
                        WorkerRole::SamplerAggregator,
                        run.run_id,
                    )
                    .await?;
                assigned_sampler = Some(node_name);
            }
        }

        for node_name in free_iter.take(evaluator_limit) {
            store
                .upsert_desired_assignment(&node_name, WorkerRole::Evaluator, run.run_id)
                .await?;
            assigned_evaluators.push(node_name);
        }

        tracing::info!(
            run_id = run.run_id,
            run_name = run.run_name,
            sampler_already_assigned,
            assigned_sampler = assigned_sampler.as_deref().unwrap_or("none"),
            assigned_evaluators = assigned_evaluators.len(),
            requested_evaluator_limit = args.max_evaluators,
            "auto-assign completed"
        );
        Ok(())
    })
    .await
}
