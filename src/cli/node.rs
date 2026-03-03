use clap::{Args, Subcommand};
use gammaboard::core::{ControlPlaneStore, DesiredAssignment, WorkerRole};
use gammaboard::{BinResult, init_pg_store};

use super::shared::{NodeSelection, RoleArg};

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

pub async fn run_node_commands(command: NodeCommand) -> BinResult {
    let store = init_pg_store(10).await?;

    match command {
        NodeCommand::Assign {
            node_id,
            role,
            run_id,
        } => {
            store
                .upsert_desired_assignment(&node_id, role.into(), run_id)
                .await?;
            println!(
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
            println!(
                "unassigned node={} role={}",
                node_id,
                WorkerRole::from(role)
            );
        }
        NodeCommand::ListAssignments { node_id } => {
            let assignments = store.list_desired_assignments(node_id.as_deref()).await?;
            if assignments.is_empty() {
                println!("no desired assignments");
            } else {
                for assignment in &assignments {
                    print_assignment(assignment);
                }
            }
        }
        NodeCommand::Stop(selection) => {
            if selection.all {
                let rows = store.request_all_nodes_shutdown().await?;
                println!("requested shutdown for all nodes: rows_updated={rows}");
            } else {
                for node_id in selection.node_ids {
                    let rows = store.request_node_shutdown(&node_id).await?;
                    println!("requested shutdown for node={node_id}: rows_updated={rows}");
                }
            }
        }
    }

    Ok(())
}

fn print_assignment(assignment: &DesiredAssignment) {
    println!(
        "node={} role={} run_id={}",
        assignment.node_id, assignment.role, assignment.run_id
    );
}
