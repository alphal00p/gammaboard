use super::shared::with_cli_store;
use anyhow::{Result, anyhow};
use clap::Args;
use gammaboard::core::{ControlPlaneStore, RunReadStore, WorkerRole};

#[derive(Debug, Args)]
pub struct AutoAssignArgs {
    pub run_id: i32,
    pub max_evaluators: Option<usize>,
}

pub async fn run_auto_assign_command(args: AutoAssignArgs, quiet: bool) -> Result<()> {
    let span = tracing::span!(
        tracing::Level::TRACE,
        "control_auto_assign_command",
        source = "control",
        command = "auto_assign",
        run_id = args.run_id
    );

    with_cli_store(10, quiet, span, |store| async move {
        let run = store
            .get_run_progress(args.run_id)
            .await?
            .ok_or_else(|| anyhow!("run {} not found", args.run_id))?;
        let nodes = store.list_nodes(None).await?;
        let free_nodes = nodes
            .iter()
            .filter(|node| node.desired_assignment.is_none())
            .map(|node| node.node_id.clone())
            .collect::<Vec<_>>();
        let sampler_already_assigned = nodes.iter().any(|node| {
            node.desired_assignment.as_ref().is_some_and(|assignment| {
                assignment.run_id == args.run_id && assignment.role == WorkerRole::SamplerAggregator
            })
        });

        let evaluator_limit = args.max_evaluators.unwrap_or(usize::MAX);
        let mut assigned_sampler = None;
        let mut assigned_evaluators = Vec::new();
        let mut free_iter = free_nodes.into_iter();

        if !sampler_already_assigned {
            if let Some(node_id) = free_iter.next() {
                store
                    .upsert_desired_assignment(&node_id, WorkerRole::SamplerAggregator, args.run_id)
                    .await?;
                assigned_sampler = Some(node_id);
            }
        }

        for node_id in free_iter.take(evaluator_limit) {
            store
                .upsert_desired_assignment(&node_id, WorkerRole::Evaluator, args.run_id)
                .await?;
            assigned_evaluators.push(node_id);
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
