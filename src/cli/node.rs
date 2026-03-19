use super::shared::{NodeSelection, RoleArg, with_cli_store};
use anyhow::Result;
use clap::{Args, Subcommand};
use comfy_table::{Cell, CellAlignment, ContentArrangement, Table};
use gammaboard::core::{ControlPlaneStore, RegisteredNode, WorkerRole};

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
    },
    List {
        node_id: Option<String>,
    },
    Stop(NodeSelection),
}

pub async fn run_node_commands(command: NodeCommand, quiet: bool) -> Result<()> {
    let command_name = node_command_name(&command);
    let span = tracing::span!(
        tracing::Level::TRACE,
        "control_node_command",
        source = "control",
        command = command_name
    );

    with_cli_store(10, quiet, span, |store| async move {
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
            NodeCommand::Unassign { node_id } => {
                store.clear_desired_assignment(&node_id).await?;
                tracing::info!("unassigned node={}", node_id);
            }
            NodeCommand::List { node_id } => {
                let nodes = store.list_nodes(node_id.as_deref()).await?;
                print_node_table(build_node_rows(nodes));
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
    })
    .await
}

fn node_command_name(command: &NodeCommand) -> &'static str {
    match command {
        NodeCommand::Assign { .. } => "node_assign",
        NodeCommand::Unassign { .. } => "node_unassign",
        NodeCommand::List { .. } => "node_list",
        NodeCommand::Stop(_) => "node_stop",
    }
}

#[derive(Debug)]
struct NodeRow {
    node_id: String,
    run: String,
    role: String,
    last_seen: String,
}

fn build_node_rows(nodes: Vec<RegisteredNode>) -> Vec<NodeRow> {
    nodes
        .into_iter()
        .map(|node| NodeRow {
            node_id: node.node_id,
            run: node
                .desired_assignment
                .as_ref()
                .map(|assignment| assignment.run_id.to_string())
                .unwrap_or_else(|| "N/A".to_string()),
            role: node
                .desired_assignment
                .as_ref()
                .map(|assignment| format_role(Some(assignment.role)))
                .unwrap_or_else(|| "None".to_string()),
            last_seen: format_last_seen(node.last_seen),
        })
        .collect()
}

fn print_node_table(rows: Vec<NodeRow>) {
    if rows.is_empty() {
        println!("no nodes found");
        return;
    }

    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec![
        Cell::new("ID").set_alignment(CellAlignment::Center),
        Cell::new("Run").set_alignment(CellAlignment::Center),
        Cell::new("Role").set_alignment(CellAlignment::Center),
        Cell::new("Last Seen").set_alignment(CellAlignment::Center),
    ]);

    for row in rows {
        table.add_row(vec![row.node_id, row.run, row.role, row.last_seen]);
    }

    println!("{table}");
}

fn format_role(role: Option<WorkerRole>) -> String {
    match role {
        Some(WorkerRole::Evaluator) => "Evaluator".to_string(),
        Some(WorkerRole::SamplerAggregator) => "Sampler Aggregator".to_string(),
        None => "None".to_string(),
    }
}

fn format_last_seen(last_seen: Option<chrono::DateTime<chrono::Utc>>) -> String {
    last_seen
        .map(|ts| ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| "-".to_string())
}
