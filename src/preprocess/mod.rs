use crate::core::{EngineError, IntegrationParams, RunStageSnapshot, RunTaskInput, RunTaskSpec};
use crate::utils::domain::Domain;
use serde::Deserialize;
use serde_json::Value as JsonValue;

/// Submitted definition after template expansion; it contains no execution state.
#[derive(Debug, Clone)]
pub struct RunDefinition {
    pub name: String,
    pub target: Option<JsonValue>,
    pub original_toml: String,
    pub effective_document: toml::Value,
    pub body: RunDefinitionBody,
}

#[derive(Debug, Clone)]
pub enum RunDefinitionBody {
    Integration(Box<IntegrationDefinition>),
    // Reuse the controller's typed specification without creating a task yet.
    Controller(Box<RunTaskSpec>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct IntegrationDefinition {
    #[serde(flatten)]
    pub params: IntegrationParams,
    #[serde(default)]
    pub task_queue: Vec<RunTaskInput>,
}

impl RunDefinition {
    pub fn kind(&self) -> &'static str {
        match &self.body {
            RunDefinitionBody::Integration(_) => "integration",
            RunDefinitionBody::Controller(task) => task.kind_str(),
        }
    }

    pub fn integration(&self) -> Option<&IntegrationDefinition> {
        match &self.body {
            RunDefinitionBody::Integration(definition) => Some(definition),
            RunDefinitionBody::Controller(_) => None,
        }
    }

    pub fn controller(&self) -> Option<&RunTaskSpec> {
        match &self.body {
            RunDefinitionBody::Controller(task) => Some(task),
            RunDefinitionBody::Integration(_) => None,
        }
    }
}

/// Fully resolved creation input. Preparation consumes the submitted definition.
#[derive(Debug)]
pub struct PreparedRun {
    pub name: String,
    pub kind: &'static str,
    pub target: Option<JsonValue>,
    pub original_toml: String,
    pub effective_document: toml::Value,
    pub integration_params: IntegrationParams,
    pub task_queue: Vec<RunTaskInput>,
    pub domain: Domain,
    pub initial_stage: RunStageSnapshot,
}

pub fn prepare_run(config: RunDefinition) -> Result<PreparedRun, EngineError> {
    crate::services::validation::validate_resources(&config)?;
    let kind = config.kind();
    let (integration_params, task_queue) = match config.body {
        RunDefinitionBody::Integration(definition) => (definition.params, definition.task_queue),
        RunDefinitionBody::Controller(task) => {
            let mut params: IntegrationParams =
                toml::from_str(include_str!("../config_defaults/run.toml")).map_err(|e| {
                    EngineError::build(format!("invalid embedded run defaults: {e}"))
                })?;
            params.evaluator = None;
            (
                params,
                vec![RunTaskInput {
                    name: Some(kind.into()),
                    task: *task,
                }],
            )
        }
    };
    for task in &task_queue {
        task.validate().map_err(EngineError::invalid_input)?;
        for child in crate::api::run_definition::preflight_children(&task.task)
            .map_err(|e| EngineError::invalid_input(e.to_string()))?
        {
            crate::api::runs::validate_run(child, false)
                .map_err(|e| EngineError::invalid_input(e.to_string()))?;
        }
    }
    // The first task-local evaluator can establish the domain when the root omits it.
    let evaluator = integration_params.evaluator.clone().or_else(|| {
        task_queue
            .iter()
            .find_map(|task| task.task.evaluator_config())
    });
    let domain = match evaluator {
        Some(evaluator) => evaluator.resolve_domain().map_err(|e| {
            EngineError::build(format!(
                "failed to resolve {} domain: {e}",
                evaluator.kind_str()
            ))
        })?,
        None => Domain::continuous(0),
    };
    let initial_stage = RunStageSnapshot {
        id: None,
        run_id: 0,
        task_id: None,
        name: "root".into(),
        sequence_nr: None,
        queue_empty: true,
        sampler_snapshot: None,
        observable_state: None,
        evaluator: integration_params.evaluator.clone(),
        batch_transforms: Vec::new(),
    };
    Ok(PreparedRun {
        name: config.name,
        kind,
        target: config.target,
        original_toml: config.original_toml,
        effective_document: config.effective_document,
        integration_params,
        task_queue,
        domain,
        initial_stage,
    })
}
