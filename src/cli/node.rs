use super::shared::with_cli_store;
use anyhow::{Result, bail};
use clap::Args;
use gammaboard::{
    config::RuntimeConfig,
    core::NodeCapabilities,
    runners::{NodeRunner, NodeRunnerConfig},
};
#[derive(Debug, Args)]
pub struct NodeRunArgs {
    #[arg(long)]
    name: String,
    #[arg(long, default_value_t = 3)]
    max_start_failures: u32,
    #[arg(long = "capability", value_parser = parse_capability, value_name = "KEY=VALUE")]
    capabilities: Vec<(String, u64)>,
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

pub async fn run_node(args: NodeRunArgs, config: &RuntimeConfig, quiet: bool) -> Result<()> {
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
    // One connection holds the controller guard; another serves heartbeats and queries.
    with_cli_store(config, 2, quiet, span, |store| async move {
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
