use super::auto_assign::{AutoAssignArgs, run_auto_assign_command};
use super::shared::{NodeSelection, RoleArg, resolve_run_ref, with_cli_store, with_control_store};
use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use comfy_table::{Cell, CellAlignment, ContentArrangement, Table};
use gammaboard::PgStore;
use gammaboard::api::nodes as node_api;
use gammaboard::config::RuntimeConfig;
use gammaboard::core::{ControlPlaneStore, NodeCapabilities, RegisteredNode, WorkerRole};
use gammaboard::runners::{NodeRunner, NodeRunnerConfig};
use gammaboard::runtime_context::RuntimeContext;
use std::{fs::File, process::Stdio};

#[derive(Debug, Args)]
pub struct NodeArgs {
    #[command(subcommand)]
    pub command: NodeCommand,
}

#[derive(Debug, Subcommand)]
pub enum NodeCommand {
    /// Run one worker node in the foreground
    Run(NodeRunArgs),
    /// Spawn local worker node processes
    StartLocal(AutoRunArgs),
    /// Assign free nodes to a run automatically
    AutoAssign(AutoAssignArgs),
    /// Assign a node role for a run
    Assign {
        node_name: String,
        role: RoleArg,
        run: String,
    },
    /// Clear a node's desired assignment
    Unassign { node_name: String },
    /// List registered nodes
    List { node_name: Option<String> },
    /// Request shutdown for one node or all nodes
    Stop(NodeSelection),
}

#[derive(Debug, Args)]
pub struct NodeRunArgs {
    #[arg(long)]
    name: String,
    #[arg(long, default_value_t = 3)]
    max_start_failures: u32,
    #[arg(long = "capability", value_parser = parse_capability, value_name = "KEY=VALUE")]
    capabilities: Vec<(String, u64)>,
}

#[derive(Debug, Args)]
pub struct AutoRunArgs {
    count: usize,
    #[arg(long, default_value_t = 3)]
    max_start_failures: u32,
}

fn parse_capability(raw: &str) -> Result<(String, u64), String> {
    let (key, value) = raw
        .split_once('=')
        .ok_or_else(|| "capability must use KEY=VALUE".to_string())?;
    let key = key.trim();
    if key.is_empty() {
        return Err("capability key must not be empty".to_string());
    }
    if !key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(
            "capability key may only contain ASCII letters, numbers, '_' and '-'".to_string(),
        );
    }
    let value = value
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("capability value must be an unsigned integer: {err}"))?;
    Ok((key.to_string(), value))
}

fn capabilities_from_pairs(pairs: Vec<(String, u64)>) -> Result<NodeCapabilities> {
    let mut capabilities = NodeCapabilities::new();
    for (key, value) in pairs {
        if capabilities.insert(key.clone(), value).is_some() {
            bail!("duplicate capability key: {key}");
        }
    }
    Ok(capabilities)
}

pub async fn run_node_commands(
    command: NodeCommand,
    runtime: &RuntimeContext,
    quiet: bool,
) -> Result<()> {
    let config = runtime.runtime_config();
    if let NodeCommand::Run(args) = command {
        return run_node(args, config, quiet).await;
    }

    if let NodeCommand::StartLocal(args) = command {
        return run_auto_run_command(args, runtime, quiet).await;
    }

    if let NodeCommand::AutoAssign(args) = command {
        return run_auto_assign_command(args, config, quiet).await;
    }

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
                    println!(
                        "assigned node={} role={} run_id={} run_name={}",
                        assigned.node_name, assigned.role, assigned.run_id, assigned.run_name
                    );
                }
                NodeCommand::Unassign { node_name } => {
                    node_api::unassign_node(&store, &node_name).await?;
                    println!("unassigned node={}", node_name);
                }
                NodeCommand::List { node_name } => {
                    let nodes = store.list_nodes(node_name.as_deref()).await?;
                    print_node_table(build_node_rows(nodes));
                }
                NodeCommand::Stop(selection) => stop_nodes(&store, selection).await?,
                NodeCommand::Run(_) | NodeCommand::StartLocal(_) | NodeCommand::AutoAssign(_) => {
                    unreachable!()
                }
            }
            Ok(())
        },
    )
    .await
}

fn node_command_name(command: &NodeCommand) -> &'static str {
    match command {
        NodeCommand::Run(_) => "node_run",
        NodeCommand::StartLocal(_) => "node_start_local",
        NodeCommand::AutoAssign(_) => "node_auto_assign",
        NodeCommand::Assign { .. } => "node_assign",
        NodeCommand::Unassign { .. } => "node_unassign",
        NodeCommand::List { .. } => "node_list",
        NodeCommand::Stop(_) => "node_stop",
    }
}

async fn run_node(args: NodeRunArgs, config: &RuntimeConfig, quiet: bool) -> Result<()> {
    let node_name = args.name.clone();
    let capabilities = capabilities_from_pairs(args.capabilities)?;
    println!(
        "starting node runner: name={} max_start_failures={} capabilities={:?}",
        node_name, args.max_start_failures, capabilities
    );
    let span = tracing::span!(
        tracing::Level::TRACE,
        "node-run",
        source = "worker",
        node_name = %node_name
    );
    with_cli_store(config, 1, quiet, span, |store| async move {
        let node_runner = NodeRunner::new(
            store,
            config.database.url.clone(),
            node_name,
            NodeRunnerConfig {
                max_consecutive_start_failures: args.max_start_failures,
                capabilities,
                ..NodeRunnerConfig::default()
            },
        );
        node_runner.run().await?;
        Ok(())
    })
    .await
}

async fn run_auto_run_command(
    args: AutoRunArgs,
    runtime: &RuntimeContext,
    quiet: bool,
) -> Result<()> {
    let config = runtime.runtime_config();
    let planned = with_control_store(config, 10, quiet, "node_auto_run", |store| async move {
        Ok(node_api::plan_auto_run_nodes(&store, args.count).await?)
    })
    .await?;

    let binary = std::env::current_exe()?;
    for node_name in &planned.node_names {
        let (stdout_log_path, stderr_log_path) = runtime.node_log_paths(node_name)?;
        let stdout_log = File::create(&stdout_log_path)?;
        let stderr_log = File::create(&stderr_log_path)?;
        let mut command = std::process::Command::new(&binary);
        command
            .args(runtime.runtime_cli_args())
            .args(node_api::node_run_cli_args(
                node_name,
                args.max_start_failures,
                &NodeCapabilities::default(),
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_log))
            .stderr(Stdio::from(stderr_log));
        let mut child = command.spawn()?;
        let name = node_name.clone();
        std::thread::spawn(move || match child.wait() {
            Ok(status) if !status.success() => {
                eprintln!(
                    "spawned node process exited unsuccessfully: node_name={} exit_status={} stdout_log={} stderr_log={}",
                    name,
                    status,
                    stdout_log_path.display(),
                    stderr_log_path.display()
                );
            }
            Ok(_) => {}
            Err(err) => {
                eprintln!(
                    "spawned node process wait failed: node_name={} error={} stdout_log={} stderr_log={}",
                    name,
                    err,
                    stdout_log_path.display(),
                    stderr_log_path.display()
                );
            }
        });
    }

    println!(
        "started node processes: requested={} started={} node_names={}",
        planned.requested_count,
        planned.node_names.len(),
        planned.node_names.join(",")
    );
    Ok(())
}

async fn stop_nodes(store: &PgStore, selection: NodeSelection) -> Result<()> {
    if selection.all {
        let rows = store.request_all_nodes_shutdown().await?;
        println!("requested shutdown for all nodes: rows_updated={rows}");
        return Ok(());
    }

    for node_name in selection.node_names {
        let stopped = node_api::stop_node(store, &node_name).await?;
        println!(
            "requested shutdown for node={}: rows_updated={}",
            stopped.node_name, stopped.rows_updated
        );
    }
    Ok(())
}

#[derive(Debug)]
struct NodeRow {
    node_name: String,
    node_uuid: String,
    capabilities: String,
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
            capabilities: format_capabilities(&node.capabilities),
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
        Cell::new("Capabilities").set_alignment(CellAlignment::Center),
        Cell::new("Run").set_alignment(CellAlignment::Center),
        Cell::new("Role").set_alignment(CellAlignment::Center),
        Cell::new("Last Seen").set_alignment(CellAlignment::Center),
    ]);

    for row in rows {
        table.add_row(vec![
            row.node_name,
            row.node_uuid,
            row.capabilities,
            row.run,
            row.role,
            row.last_seen,
        ]);
    }

    println!("{table}");
}

fn format_capabilities(capabilities: &NodeCapabilities) -> String {
    if capabilities.is_empty() {
        return "-".to_string();
    }
    capabilities
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",")
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
