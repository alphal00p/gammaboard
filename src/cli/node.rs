use super::shared::{NodeSelection, RoleArg, resolve_run_ref, with_control_store};
use anyhow::Result;
use clap::{Args, Subcommand};
use comfy_table::{Cell, CellAlignment, ContentArrangement, Table};
use gammaboard::PgStore;
use gammaboard::api::nodes as node_api;
use gammaboard::config::CliConfig;
use gammaboard::core::{ControlPlaneStore, RegisteredNode, WorkerRole};

#[derive(Debug, Args)]
pub struct NodeArgs {
    #[command(subcommand)]
    pub command: NodeCommand,
}

#[derive(Debug, Subcommand)]
pub enum NodeCommand {
    Assign {
        node_name: String,
        role: RoleArg,
        run: String,
    },
    Unassign {
        node_name: String,
    },
    List {
        node_name: Option<String>,
    },
    Stop(NodeSelection),
}

pub async fn run_node_commands(
    command: NodeCommand,
    config: &CliConfig,
    quiet: bool,
) -> Result<()> {
    with_control_store(
        config,
        10,
        quiet,
        node_command_name(&command),
        |store| async move {
            match command {
                NodeCommand::Assign {
                    node_name,
                    role,
                    run,
                } => {
                    let run = resolve_run_ref(&store, &run).await?;
                    let assigned =
                        node_api::assign_node(&store, &node_name, run.run_id, role.into()).await?;
                    tracing::info!(
                        "assigned node={} role={} run_id={} run_name={}",
                        assigned.node_name,
                        assigned.role,
                        assigned.run_id,
                        assigned.run_name
                    );
                }
                NodeCommand::Unassign { node_name } => {
                    node_api::unassign_node(&store, &node_name).await?;
                    tracing::info!("unassigned node={}", node_name);
                }
                NodeCommand::List { node_name } => {
                    let nodes = store.list_nodes(node_name.as_deref()).await?;
                    print_node_table(build_node_rows(nodes));
                }
                NodeCommand::Stop(selection) => stop_nodes(&store, selection).await?,
            }
            Ok(())
        },
    )
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

async fn stop_nodes(store: &PgStore, selection: NodeSelection) -> Result<()> {
    if selection.all {
        let rows = store.request_all_nodes_shutdown().await?;
        tracing::info!("requested shutdown for all nodes: rows_updated={rows}");
        return Ok(());
    }

    for node_name in selection.node_names {
        let stopped = node_api::stop_node(store, &node_name).await?;
        tracing::info!(
            "requested shutdown for node={}: rows_updated={}",
            stopped.node_name,
            stopped.rows_updated
        );
    }
    Ok(())
}

#[derive(Debug)]
struct NodeRow {
    node_name: String,
    node_uuid: String,
    run: String,
    role: String,
    last_seen: String,
}

fn build_node_rows(nodes: Vec<RegisteredNode>) -> Vec<NodeRow> {
    nodes
        .into_iter()
        .map(|node| NodeRow {
            node_name: node.name,
            node_uuid: node.uuid,
            run: node
                .desired_assignment
                .as_ref()
                .map(|assignment| match assignment.run_name.as_deref() {
                    Some(run_name) => format!("{run_name} (#{})", assignment.run_id),
                    None => assignment.run_id.to_string(),
                })
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
        Cell::new("Name").set_alignment(CellAlignment::Center),
        Cell::new("UUID").set_alignment(CellAlignment::Center),
        Cell::new("Run").set_alignment(CellAlignment::Center),
        Cell::new("Role").set_alignment(CellAlignment::Center),
        Cell::new("Last Seen").set_alignment(CellAlignment::Center),
    ]);

    for row in rows {
        table.add_row(vec![
            row.node_name,
            row.node_uuid,
            row.run,
            row.role,
            row.last_seen,
        ]);
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
