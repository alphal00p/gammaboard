//! Public run documents. Integration defaults never leak into controller schemas.
//! Child sources are frozen at submission; bindings are applied before expansion.
use super::{ApiError, toml_template};
use crate::core::tasks::ChildRunSource;
use crate::core::{RunTaskInput, RunTaskSpec};
use crate::preprocess::RunAddConfig;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn parse(
    raw: &str,
    bindings: BTreeMap<String, toml::Value>,
    base: Option<&Path>,
) -> Result<RunAddConfig, ApiError> {
    let value: toml::Value =
        toml::from_str(raw).map_err(|e| bad(format!("invalid run TOML: {e}")))?;
    let original: toml::Value = value.clone();
    let retain_original = bindings.is_empty();
    let mut config = instantiate(value, bindings, base)?;
    if retain_original
        && config
            .original_toml
            .as_ref()
            .and_then(|raw| toml::from_str::<toml::Value>(raw).ok())
            .as_ref()
            == Some(&original)
    {
        config.original_toml = Some(raw.to_owned());
    }
    Ok(config)
}

pub fn instantiate(
    mut value: toml::Value,
    bindings: BTreeMap<String, toml::Value>,
    base: Option<&Path>,
) -> Result<RunAddConfig, ApiError> {
    let base = base
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().map_err(|e| bad(e.to_string()))?);
    freeze_sources(&mut value, &base, &mut Vec::new(), 0)?;
    toml_template::merge_replacements(&mut value, bindings)?;
    let frozen = toml::to_string(&value).map_err(|e| bad(e.to_string()))?;
    let value = toml_template::expand_toml_template(value)?.value;
    validate_document(&value)?;
    let kind = kind(&value)?.to_owned();
    let mut table = value
        .as_table()
        .cloned()
        .ok_or_else(|| bad("run must be a table"))?;
    table.remove("kind");
    let name = table
        .get("name")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| bad("run.name must be a string"))?
        .to_owned();
    if name.trim().is_empty() {
        return Err(bad("run.name must not be empty"));
    }
    let effective = value.clone();
    if kind != "integration" {
        table.remove("name");
        table.remove("gammaboard");
        table.insert("kind".into(), toml::Value::String(kind.clone()));
        let controller: RunTaskSpec = toml::Value::Table(table)
            .try_into()
            .map_err(|e| bad(format!("invalid {kind}: {e}")))?;
        controller.validate().map_err(bad)?;
        if let RunTaskSpec::IntegrationCampaign { children, .. } = &controller {
            for child in children {
                let config =
                    instantiate_child(child.run.clone(), child.replacements.clone(), None)?;
                if config.kind != "integration" {
                    return Err(bad(format!(
                        "campaign child '{}' must be an integration run",
                        child.name
                    )));
                }
                if !config.task_queue.as_ref().is_some_and(|tasks| {
                    tasks.iter().any(|task| {
                        matches!(
                            task.task,
                            RunTaskSpec::Sample {
                                publish_result: true,
                                ..
                            }
                        )
                    })
                }) {
                    return Err(bad(format!(
                        "campaign child '{}' needs at least one publishing sample task",
                        child.name
                    )));
                }
            }
        }
        // Share durable execution/checkpoint infrastructure with integration tasks.
        // This single controller record is not part of the public task-queue schema.
        let mut config =
            super::runs::parse_integration_value(toml::Value::Table(toml::map::Map::from_iter([
                ("name".into(), toml::Value::String(name)),
            ])))?;
        config.task_queue = Some(vec![RunTaskInput {
            name: Some(kind.clone()),
            task: controller,
        }]);
        config.kind = kind;
        config.original_toml = Some(frozen);
        config.effective_document = Some(effective);
        Ok(config)
    } else {
        let mut config = super::runs::parse_integration_value(toml::Value::Table(table))?;
        config.original_toml = Some(frozen);
        config.effective_document = Some(effective);
        Ok(config)
    }
}

fn kind(value: &toml::Value) -> Result<&str, ApiError> {
    match value.get("kind") {
        None => Ok("integration"),
        Some(toml::Value::String(kind)) => Ok(kind),
        _ => Err(bad("run.kind must be a string")),
    }
}

pub(crate) fn validate_document(value: &toml::Value) -> Result<(), ApiError> {
    let table = value.as_table().ok_or_else(|| bad("run must be a table"))?;
    let kind = kind(value)?;
    let fields: &[&str] = match kind {
        "integration" => &[
            "target",
            "evaluator",
            "evaluator_requirements",
            "sampler_requirements",
            "evaluator_runner_params",
            "sampler_aggregator_runner_params",
            "task_queue",
        ],
        "integration_campaign" => &["children", "measurement", "stop_condition", "allocation"],
        "parameter_scan" => &["child", "parameters", "measurement", "max_concurrent_runs"],
        "hyperparameter_tuning" => &[
            "child",
            "parameters",
            "objective",
            "optimizer",
            "max_concurrent_trials",
        ],
        _ => return Err(bad(format!("unknown run kind '{kind}'"))),
    };
    for key in table.keys() {
        if !["kind", "name", "replacements", "gammaboard"].contains(&key.as_str())
            && !fields.contains(&key.as_str())
        {
            return Err(bad(format!(
                "field '{key}' is not valid for run kind '{kind}'"
            )));
        }
    }
    let selection = match kind {
        "integration_campaign" => Some(("measurement", &["quantity"][..])),
        "parameter_scan" => Some(("measurement", &["source_task", "quantity"][..])),
        "hyperparameter_tuning" => Some(("objective", &["source_task", "quantity", "mode"][..])),
        _ => None,
    };
    if let Some((field, allowed)) = selection
        && let Some(selection) = table.get(field).and_then(toml::Value::as_table)
    {
        for key in selection.keys() {
            if !allowed.contains(&key.as_str()) {
                return Err(bad(format!(
                    "field '{field}.{key}' is not valid for run kind '{kind}'"
                )));
            }
        }
    }
    if let Some(tasks) = table.get("task_queue").and_then(toml::Value::as_array) {
        for task in tasks {
            validate_integration_task(task)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_integration_task(task: &toml::Value) -> Result<(), ApiError> {
    if task
        .get("kind")
        .and_then(toml::Value::as_str)
        .is_some_and(|k| {
            matches!(
                k,
                "integration_campaign" | "parameter_scan" | "hyperparameter_tuning"
            )
        })
    {
        return Err(bad(
            "orchestration is a run kind, not an integration task; move its definition to the document root",
        ));
    }
    Ok(())
}

/// Resolve either child source through the same document pipeline. Submitted
/// controllers already contain frozen inline sources, so spawning never rereads files.
pub(crate) fn instantiate_child(
    source: ChildRunSource,
    bindings: BTreeMap<String, toml::Value>,
    base: Option<&Path>,
) -> Result<RunAddConfig, ApiError> {
    let base = match base {
        Some(base) => base.to_path_buf(),
        None => std::env::current_dir().map_err(|e| bad(e.to_string()))?,
    };
    let value = resolve_child(source, &base, &mut Vec::new(), 1)?;
    instantiate(value, bindings, Some(&base))
}

// Freeze nested sources before expanding the enclosing document's replacements.
fn freeze_sources(
    value: &mut toml::Value,
    base: &Path,
    stack: &mut Vec<PathBuf>,
    depth: usize,
) -> Result<(), ApiError> {
    if depth > 32 {
        return Err(bad("child run reference nesting exceeds 32"));
    }
    let table = value
        .as_table_mut()
        .ok_or_else(|| bad("run definition must be a table"))?;
    if let Some(child) = table.get_mut("child")
        && let Some(source) = child.get_mut("run")
    {
        freeze_source(source, base, stack, depth + 1)?;
    }
    if let Some(children) = table
        .get_mut("children")
        .and_then(toml::Value::as_array_mut)
    {
        for child in children {
            if let Some(source) = child.get_mut("run") {
                freeze_source(source, base, stack, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn freeze_source(
    source: &mut toml::Value,
    base: &Path,
    stack: &mut Vec<PathBuf>,
    depth: usize,
) -> Result<(), ApiError> {
    let child: ChildRunSource = source.clone().try_into().map_err(|_| {
        bad("child.run must be a TOML string or a file reference (run = { file = \"...\" }); nested run-definition tables are not supported")
    })?;
    let value = resolve_child(child, base, stack, depth)?;
    *source = toml::Value::String(toml::to_string(&value).map_err(|e| bad(e.to_string()))?);
    Ok(())
}

fn resolve_child(
    source: ChildRunSource,
    base: &Path,
    stack: &mut Vec<PathBuf>,
    depth: usize,
) -> Result<toml::Value, ApiError> {
    source.validate().map_err(bad)?;
    let (raw, file) = match source {
        ChildRunSource::Inline(raw) => (raw, None),
        ChildRunSource::File(source) => {
            let path = base.join(&source.file).canonicalize().map_err(|e| {
                bad(format!(
                    "child run file {}: {e}",
                    base.join(&source.file).display()
                ))
            })?;
            if stack.contains(&path) {
                return Err(bad(format!(
                    "cyclic child run reference: {}",
                    path.display()
                )));
            }
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| bad(format!("{}: {e}", path.display())))?;
            (raw, Some(path))
        }
    };
    let mut value: toml::Value = toml::from_str(&raw).map_err(|e| {
        bad(format!(
            "invalid child run TOML{}: {e}",
            file.as_ref()
                .map(|path| format!(" in {}", path.display()))
                .unwrap_or_default()
        ))
    })?;
    let child_base = file.as_ref().and_then(|path| path.parent()).unwrap_or(base);
    if let Some(path) = &file {
        stack.push(path.clone());
    }
    let result = freeze_sources(&mut value, child_base, stack, depth);
    if file.is_some() {
        stack.pop();
    }
    result?;
    validate_document(&value)?;
    Ok(value)
}

fn bad(message: impl Into<String>) -> ApiError {
    ApiError::BadRequest(message.into())
}

/// Validate one representative scan/trial binding; every generated instance is
/// validated again at creation. Campaign children are all known at submission.
pub(crate) fn preflight_children(task: &RunTaskSpec) -> Result<Vec<RunAddConfig>, ApiError> {
    use crate::core::HyperparameterTuningParameterDomain as Parameter;
    let sources = match task {
        RunTaskSpec::IntegrationCampaign { children, .. } => children
            .iter()
            .map(|child| (child.run.clone(), child.replacements.clone()))
            .collect(),
        RunTaskSpec::ParameterScan {
            child, parameters, ..
        } => {
            let mut bindings = child.replacements.clone();
            for parameter in parameters {
                if let Some(value) = parameter.values().map_err(bad)?.into_iter().next() {
                    bindings.insert(parameter.name.clone(), value);
                }
            }
            vec![(child.run.clone(), bindings)]
        }
        RunTaskSpec::HyperparameterTuning {
            child, parameters, ..
        } => {
            let mut bindings = child.replacements.clone();
            for (name, parameter) in parameters {
                let value = match parameter {
                    Parameter::Float(domain) => toml::Value::Float(domain.min),
                    Parameter::Integer(domain) => toml::Value::Integer(domain.min),
                    Parameter::Categorical(domain) => domain
                        .source
                        .values(name)
                        .map_err(bad)?
                        .into_iter()
                        .next()
                        .ok_or_else(|| bad("empty parameter domain"))?,
                };
                bindings.insert(name.clone(), value);
            }
            vec![(child.run.clone(), bindings)]
        }
        _ => Vec::new(),
    };
    sources
        .into_iter()
        .map(|(run, bindings)| instantiate_child(run, bindings, None))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EvaluatorConfig;

    #[test]
    fn integration_is_default_and_all_kinds_reject_foreign_fields() {
        let default = parse("name = 'plain'", BTreeMap::new(), None).unwrap();
        assert_eq!(default.kind, "integration");
        let explicit = parse(
            "name = 'plain'\nkind = 'integration'",
            BTreeMap::new(),
            None,
        )
        .unwrap();
        assert_eq!(default.kind, explicit.kind);
        for kind in [
            "integration_campaign",
            "parameter_scan",
            "hyperparameter_tuning",
        ] {
            for field in [
                "evaluator",
                "task_queue",
                "target",
                "sampler_requirements",
                "evaluator_runner_params",
                "sampler_aggregator_runner_params",
            ] {
                let doc = format!("name = 'bad'\nkind = '{kind}'\n{field} = []");
                let error = parse(&doc, BTreeMap::new(), None).unwrap_err().to_string();
                assert!(error.contains(field) && error.contains(kind), "{error}");
            }
        }
        for field in [
            "children",
            "allocation",
            "optimizer",
            "parameters",
            "child",
            "stop_condition",
        ] {
            assert!(
                parse(
                    &format!("name = 'bad'\n{field} = []"),
                    BTreeMap::new(),
                    None
                )
                .is_err()
            );
        }
        assert!(
            parse(
                "name = 'bad'\n[[task_queue]]\nkind = 'parameter_scan'",
                BTreeMap::new(),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn generated_parameters_override_child_bindings_and_file_defaults() {
        let config = parse(
            r#"
kind = "parameter_scan"
name = "scan"
parameters = [{ name = "dims", values = [3, 4] }]
[child]
replacements = { dims = 2 }
run = '''
name = "child-$(dims:0)"
replacements = { dims = 1 }
[evaluator]
kind = "unit"
continuous_dims = "$(dims:0)"
discrete_dims = 0
'''
"#,
            BTreeMap::new(),
            None,
        )
        .unwrap();
        let instances = preflight_children(&config.task_queue.unwrap()[0].task).unwrap();
        assert_eq!(instances[0].name, "child-3");
    }

    #[test]
    fn rejects_irrelevant_measurement_options_and_unknown_child_fields() {
        for (kind, field) in [
            ("integration_campaign", "source_task"),
            ("integration_campaign", "mode"),
            ("parameter_scan", "mode"),
        ] {
            let raw = format!(
                "name = 'invalid'\nkind = '{kind}'\nmeasurement = {{ {field} = 'unused' }}"
            );
            let error = parse(&raw, BTreeMap::new(), None).unwrap_err().to_string();
            assert!(error.contains(&format!("measurement.{field}")), "{error}");
        }
        let error = parse(
            r#"
kind = "integration_campaign"
name = "invalid"
stop_condition = { max_total_samples = 10 }
[[children]]
name = "a"
coefficent = 2
run = 'name = "child"'
"#,
            BTreeMap::new(),
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("coefficent"));
    }

    #[test]
    fn file_defaults_are_overridden_before_expansion_and_frozen() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("integration.toml");
        std::fs::write(&file, "name = 'child-$(dims:1)'\nreplacements = { dims = 2 }\n[evaluator]\nkind = 'unit'\ncontinuous_dims = '$(dims:1)'\ndiscrete_dims = 0\n[[task_queue]]\nkind = 'sample'\nstop_condition = {max_samples = 10}\naccumulator = {config = 'scalar'}\nsampler_aggregator = {config = {kind = 'naive_monte_carlo'}}").unwrap();
        let raw = "kind = 'integration_campaign'\nname = 'parent'\nreplacements = { dims = 99, selected = 3 }\nstop_condition = { max_total_samples = 10 }\n[[children]]\nname = 'a'\nreplacements = { dims = '$(selected:4)' }\nrun = { file = 'integration.toml' }\n[[children]]\nname = 'b'\nrun = { file = 'integration.toml' }";
        let config = parse(raw, BTreeMap::new(), Some(dir.path())).unwrap();
        let RunTaskSpec::IntegrationCampaign { children, .. } =
            &config.task_queue.as_ref().unwrap()[0].task
        else {
            panic!()
        };
        // The enclosing scope must not consume placeholders in child definitions.
        let ChildRunSource::Inline(raw) = &children[0].run else {
            panic!()
        };
        let document: toml::Value = toml::from_str(raw).unwrap();
        assert_eq!(document["name"].as_str(), Some("child-$(dims:1)"));
        for (child, dims) in children.iter().zip([3, 2]) {
            let config =
                instantiate_child(child.run.clone(), child.replacements.clone(), None).unwrap();
            assert_eq!(config.name, format!("child-{dims}"));
            let EvaluatorConfig::Unit { params } = config.integration_params.evaluator.unwrap()
            else {
                panic!()
            };
            assert_eq!(params.continuous_dims, dims);
        }
        std::fs::remove_file(file).unwrap();
        // Persisted document remains self-contained even after its input disappears.
        parse(
            config.original_toml.as_ref().unwrap(),
            BTreeMap::new(),
            None,
        )
        .unwrap();
    }

    #[test]
    fn nested_references_are_relative_to_each_file_and_cycles_fail() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(
            dir.path().join("nested/leaf.toml"),
            "name = 'leaf'\n[[task_queue]]\nkind = 'sample'\nstop_condition = { max_samples = 1 }\naccumulator = { config = 'scalar' }\nsampler_aggregator = { config = { kind = 'naive_monte_carlo' } }",
        )
        .unwrap();
        std::fs::write(dir.path().join("nested/parent.toml"), "kind = 'integration_campaign'\nname = 'inner'\nstop_condition = { max_total_samples = 10 }\n[[children]]\nname = 'leaf'\nrun = { file = 'leaf.toml' }").unwrap();
        let raw = "kind = 'integration_campaign'\nname = 'outer'\nstop_condition = { max_total_samples = 10 }\n[[children]]\nname = 'inner'\nrun = { file = 'nested/parent.toml' }";
        let mut value: toml::Value = toml::from_str(raw).unwrap();
        freeze_sources(&mut value, dir.path(), &mut Vec::new(), 0).unwrap();
        std::fs::write(dir.path().join("nested/leaf.toml"), "kind = 'integration_campaign'\nname = 'cycle'\nstop_condition = { max_total_samples = 10 }\n[[children]]\nname = 'parent'\nrun = { file = 'parent.toml' }").unwrap();
        let mut value: toml::Value = toml::from_str(raw).unwrap();
        assert!(
            freeze_sources(&mut value, dir.path(), &mut Vec::new(), 0)
                .unwrap_err()
                .to_string()
                .contains("cyclic")
        );
    }

    #[test]
    fn typed_arrays_and_quoted_strings_survive_child_bindings() {
        let document: toml::Value = toml::from_str("name = '$(name:fallback)'\nreplacements = { dims = 2 }\n[evaluator]\nkind = 'unit'\ncontinuous_dims = '$(dims:1)'\ndiscrete_cardinalities = '$(channels:[2])'").unwrap();
        let bindings = BTreeMap::from([
            (
                "name".into(),
                toml::Value::String("a \"quoted\" \\ name".into()),
            ),
            ("dims".into(), toml::Value::Integer(3)),
            (
                "channels".into(),
                toml::Value::Array(vec![toml::Value::Integer(2), toml::Value::Integer(4)]),
            ),
        ]);
        let config = instantiate(document, bindings, None).unwrap();
        assert_eq!(config.name, "a \"quoted\" \\ name");
        let effective = config.effective_document.unwrap();
        assert_eq!(
            effective["evaluator"]["discrete_cardinalities"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    const CHILD_DOCUMENT: &str = r#"
name = "child-$(dims:1)-$(label:fallback)"
replacements = { dims = 1, samples = 6, label = "local" }
[evaluator]
kind = "unit"
continuous_dims = "$(dims:1)"
discrete_cardinalities = "$(channels:[2, 4])"
[[task_queue]]
name = "sample"
kind = "sample"
publish_result = "$(publish:true)"
stop_condition = { max_samples = "$(samples:4)" }
measurement = { quantity = "central_value" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#;

    const CONTROLLERS: [&str; 3] = [
        "integration_campaign",
        "parameter_scan",
        "hyperparameter_tuning",
    ];

    fn controller_document(kind: &str, source: toml::Value) -> String {
        let common = "name = 'parent'\nreplacements = { dims = 99, selected = 2, label = 'parent-must-not-leak' }\n";
        let body = match kind {
            "integration_campaign" => {
                "stop_condition = { max_total_samples = 12 }\n[[children]]\nname = 'a'\nreplacements = { dims = '$(selected:4)', samples = 12 }"
            }
            "parameter_scan" => {
                "parameters = [{ name = 'dims', values = [3, 4] }]\n[child]\nreplacements = { dims = '$(selected:4)', samples = 12 }"
            }
            "hyperparameter_tuning" => {
                "optimizer = { algorithm = 'grid_search' }\nobjective = { source_task = 'sample', mode = 'minimize', quantity = 'central_value' }\nparameters = { dims = { kind = 'integer', min = 3, max = 4, step = 1 } }\n[child]\nreplacements = { dims = '$(selected:4)', samples = 12 }"
            }
            _ => unreachable!(),
        };
        let mut document: toml::Value =
            toml::from_str(&format!("kind = '{kind}'\n{common}{body}")).unwrap();
        let child = if kind == "integration_campaign" {
            &mut document["children"].as_array_mut().unwrap()[0]
        } else {
            document.get_mut("child").unwrap()
        };
        child.as_table_mut().unwrap().insert("run".into(), source);
        toml::to_string(&document).unwrap()
    }

    fn file_source(path: &str) -> toml::Value {
        toml::Value::Table(toml::map::Map::from_iter([(
            "file".into(),
            toml::Value::String(path.into()),
        )]))
    }

    #[test]
    fn inline_and_file_children_share_expansion_validation_and_freezing_for_every_controller() {
        for kind in CONTROLLERS {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("child.toml");
            std::fs::write(&path, CHILD_DOCUMENT).unwrap();
            let inline = controller_document(kind, toml::Value::String(CHILD_DOCUMENT.into()));
            let file = controller_document(kind, file_source("child.toml"));
            let inline = parse(&inline, BTreeMap::new(), Some(dir.path())).unwrap();
            let file = parse(&file, BTreeMap::new(), Some(dir.path())).unwrap();
            assert_eq!(inline.effective_document, file.effective_document, "{kind}");
            let inline_child = preflight_children(&inline.task_queue.as_ref().unwrap()[0].task)
                .unwrap()
                .remove(0);
            let file_child = preflight_children(&file.task_queue.as_ref().unwrap()[0].task)
                .unwrap()
                .remove(0);
            assert_eq!(
                inline_child.effective_document, file_child.effective_document,
                "{kind}"
            );
            let dims = if kind == "integration_campaign" { 2 } else { 3 };
            assert_eq!(file_child.name, format!("child-{dims}-local"));
            let document = file_child.effective_document.unwrap();
            assert_eq!(
                document["evaluator"]["continuous_dims"].as_integer(),
                Some(dims)
            );
            assert_eq!(
                document["evaluator"]["discrete_cardinalities"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(
                document["task_queue"][0]["publish_result"].as_bool(),
                Some(true)
            );
            assert_eq!(
                document["task_queue"][0]["stop_condition"]["max_samples"].as_integer(),
                Some(12)
            );
            std::fs::remove_file(path).unwrap();
            for config in [inline, file] {
                let restored = parse(
                    config.original_toml.as_ref().unwrap(),
                    BTreeMap::new(),
                    None,
                )
                .unwrap();
                let child = preflight_children(&restored.task_queue.unwrap()[0].task)
                    .unwrap()
                    .remove(0);
                assert_eq!(child.effective_document.unwrap(), document, "{kind}");
            }
        }
    }

    #[test]
    fn every_controller_rejects_nested_tables_and_invalid_source_shapes() {
        for kind in CONTROLLERS {
            let nested: toml::Value = toml::from_str(CHILD_DOCUMENT).unwrap();
            let mixed: toml::Value =
                toml::from_str("file = 'missing.toml'\nname = 'nested'").unwrap();
            let bad_file: toml::Value = toml::from_str("file = 42").unwrap();
            for source in [
                nested,
                mixed,
                bad_file,
                toml::Value::Integer(3),
                toml::Value::Array(vec![]),
            ] {
                let error = parse(&controller_document(kind, source), BTreeMap::new(), None)
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains("TOML string or a file reference"),
                    "{kind}: {error}"
                );
            }
            for raw in [
                "",
                "name = [",
                "kind = 'unknown'\nname = 'bad'",
                "name = 'bad'\nchildren = []",
            ] {
                let error = parse(
                    &controller_document(kind, toml::Value::String(raw.into())),
                    BTreeMap::new(),
                    None,
                );
                assert!(error.is_err(), "{kind}: {raw}");
            }
        }
    }

    #[test]
    fn nested_inline_and_file_sources_preserve_relative_bases_and_freeze_recursively() {
        for kind in CONTROLLERS {
            let dir = tempfile::tempdir().unwrap();
            let nested = dir.path().join("nested");
            std::fs::create_dir(&nested).unwrap();
            std::fs::write(nested.join("leaf.toml"), CHILD_DOCUMENT).unwrap();
            let inner = controller_document(kind, file_source("leaf.toml"));
            let middle = controller_document("parameter_scan", toml::Value::String(inner));
            std::fs::write(nested.join("middle.toml"), middle).unwrap();
            let outer = controller_document("parameter_scan", file_source("nested/middle.toml"));
            let frozen = parse(&outer, BTreeMap::new(), Some(dir.path()))
                .unwrap()
                .original_toml
                .unwrap();
            std::fs::remove_dir_all(nested).unwrap();
            let mut config = parse(&frozen, BTreeMap::new(), None).unwrap();
            for _ in 0..3 {
                config = preflight_children(&config.task_queue.as_ref().unwrap()[0].task)
                    .unwrap()
                    .remove(0);
            }
            assert_eq!(config.kind, "integration");
            assert_eq!(
                config.name,
                if kind == "integration_campaign" {
                    "child-2-local"
                } else {
                    "child-3-local"
                }
            );
        }
    }

    #[test]
    fn cycles_through_inline_sources_are_rejected_for_every_controller() {
        for kind in CONTROLLERS {
            let dir = tempfile::tempdir().unwrap();
            let inner = controller_document(kind, file_source("loop.toml"));
            let raw = controller_document(kind, toml::Value::String(inner));
            std::fs::write(dir.path().join("loop.toml"), &raw).unwrap();
            let error = parse(&raw, BTreeMap::new(), Some(dir.path()))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("cyclic child run reference"),
                "{kind}: {error}"
            );
        }
    }

    #[test]
    fn quoted_bindings_are_applied_after_parsing_both_source_forms() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("child.toml"), CHILD_DOCUMENT).unwrap();
        let bindings = BTreeMap::from([
            (
                "label".into(),
                toml::Value::String("quoted \\\"label\\\" and \\ path".into()),
            ),
            (
                "channels".into(),
                toml::Value::Array(vec![toml::Value::Integer(3)]),
            ),
        ]);
        let inline = instantiate_child(
            ChildRunSource::Inline(CHILD_DOCUMENT.into()),
            bindings.clone(),
            Some(dir.path()),
        )
        .unwrap();
        let file = instantiate_child(
            ChildRunSource::File(crate::core::tasks::ChildRunFile {
                file: "child.toml".into(),
            }),
            bindings,
            Some(dir.path()),
        )
        .unwrap();
        assert_eq!(inline.effective_document, file.effective_document);
        assert_eq!(inline.name, "child-1-quoted \\\"label\\\" and \\ path");
    }

    #[test]
    fn invalid_children_fail_the_same_submission_validation_for_both_forms() {
        let invalid = CHILD_DOCUMENT.replace("max_samples = \"$(samples:4)\"", "max_samples = -1");
        for kind in CONTROLLERS {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("invalid.toml"), &invalid).unwrap();
            let mut errors = Vec::new();
            for source in [
                toml::Value::String(invalid.clone()),
                file_source("invalid.toml"),
            ] {
                let error = parse(
                    &controller_document(kind, source),
                    BTreeMap::new(),
                    Some(dir.path()),
                )
                .and_then(|config| super::super::runs::validate_run(config, false))
                .unwrap_err()
                .to_string();
                assert!(error.contains("max_samples"), "{kind}: {error}");
                errors.push(error);
            }
            assert_eq!(errors[0], errors[1], "{kind}");
        }
    }

    #[test]
    fn deep_file_chains_fail_before_freezing_unbounded_documents() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..35 {
            let raw = controller_document(
                "parameter_scan",
                file_source(&format!("{}.toml", index + 1)),
            );
            std::fs::write(dir.path().join(format!("{index}.toml")), raw).unwrap();
        }
        let root = controller_document("parameter_scan", file_source("0.toml"));
        let error = parse(&root, BTreeMap::new(), Some(dir.path()))
            .unwrap_err()
            .to_string();
        assert!(error.contains("nesting exceeds 32"), "{error}");
    }
}
