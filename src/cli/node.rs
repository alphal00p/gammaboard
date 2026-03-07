use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use gammaboard::core::{ControlPlaneStore, DesiredAssignment, WorkerRole};
use gammaboard::init_pg_store;
use tracing::Instrument;

use super::shared::{NodeSelection, RoleArg, init_cli_tracing};

#[derive(Debug, Args)]
pub struct NodeArgs {
    #[command(subcommand)]
    pub command: NodeCommand,
}

#[derive(Debug, Subcommand)]
pub enum NodeCommand {
    Assign {
        node_id: String,
        role: RoleArg,
        run_id: i32,
    },
    Unassign {
        node_id: String,
        role: RoleArg,
    },
    ListAssignments {
        node_id: Option<String>,
    },
    Stop(NodeSelection),
}

pub async fn run_node_commands(command: NodeCommand, quiet: bool) -> Result<()> {
    let store = init_pg_store(10)
        .await
        .context("failed to initialize postgres store")?;
    init_cli_tracing(&store, quiet)?;
    let command_name = node_command_name(&command);
    let span = tracing::span!(
        tracing::Level::TRACE,
        "control_node_command",
        source = "control",
        command = command_name
    );

    async move {
        match command {
            NodeCommand::Assign {
                node_id,
                role,
                run_id,
            } => {
                store
                    .upsert_desired_assignment(&node_id, role.into(), run_id)
                    .await?;
                tracing::info!(
                    "assigned node={} role={} run_id={}",
                    node_id,
                    WorkerRole::from(role),
                    run_id
                );
            }
            NodeCommand::Unassign { node_id, role } => {
                store
                    .clear_desired_assignment(&node_id, WorkerRole::from(role))
                    .await?;
                tracing::info!(
                    "unassigned node={} role={}",
                    node_id,
                    WorkerRole::from(role)
                );
            }
            NodeCommand::ListAssignments { node_id } => {
                let assignments = store.list_desired_assignments(node_id.as_deref()).await?;
                if assignments.is_empty() {
                    tracing::info!("no desired assignments");
                } else {
                    for assignment in &assignments {
                        print_assignment(assignment);
                    }
                }
            }
            NodeCommand::Stop(selection) => {
                if selection.all {
                    let rows = store.request_all_nodes_shutdown().await?;
                    tracing::info!("requested shutdown for all nodes: rows_updated={rows}");
                } else {
                    for node_id in selection.node_ids {
                        let rows = store.request_node_shutdown(&node_id).await?;
                        tracing::info!(
                            "requested shutdown for node={node_id}: rows_updated={rows}"
                        );
                    }
                }
            }
        }
        Ok(())
    }
    .instrument(span)
    .await
}

fn node_command_name(command: &NodeCommand) -> &'static str {
    match command {
        NodeCommand::Assign { .. } => "node_assign",
        NodeCommand::Unassign { .. } => "node_unassign",
        NodeCommand::ListAssignments { .. } => "node_list_assignments",
        NodeCommand::Stop(_) => "node_stop",
    }
}

fn print_assignment(assignment: &DesiredAssignment) {
    tracing::info!(
        "node={} role={} run_id={}",
        assignment.node_id,
        assignment.role,
        assignment.run_id
    );
}
