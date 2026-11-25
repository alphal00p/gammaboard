//! Build-feature boundaries must not change the serialized model or core workflows.
use assert_cmd::Command;
use gammaboard::{Domain, EvaluatorConfig, SamplingConfig};
use serde_json::{Value, json};

#[test]
fn cli_reports_features_and_probes_the_dependency_free_example() {
    let output = Command::cargo_bin("gammaboard")
        .unwrap()
        .arg("build-info")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let info: Value = serde_json::from_slice(&output).unwrap();
    let features = info["features"].as_array().unwrap();
    assert_eq!(
        features.contains(&json!("symbolica")),
        cfg!(feature = "symbolica")
    );
    assert_eq!(
        features.contains(&json!("gammaloop")),
        cfg!(feature = "gammaloop")
    );

    let definition = gammaboard::api::runs::load_run_add_config_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources/templates/runs/installation-smoke.toml")
            .as_path(),
    )
    .unwrap();
    gammaboard::api::runs::validate_run(definition, true).unwrap();
}

#[test]
fn symbolic_configuration_remains_readable_in_every_build() {
    for source in [
        json!({"kind":"expression", "expr":"x"}),
        json!({"kind":"expressions", "components":["x"], "exprs":["x"]}),
        json!({"kind":"compiled", "path":"example.so", "name":"evaluate"}),
    ] {
        let config: EvaluatorConfig = serde_json::from_value(json!({
            "kind":"symbolica", "args":["x"], "source":source,
        }))
        .unwrap();
        assert_eq!(config.resolve_domain().unwrap(), Domain::continuous(1));
        let serialized = serde_json::to_value(&config).unwrap();
        let restored: EvaluatorConfig = serde_json::from_value(serialized.clone()).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), serialized);
        #[cfg(not(feature = "symbolica"))]
        assert!(
            config
                .build()
                .err()
                .unwrap()
                .to_string()
                .contains("\"symbolica\" build feature")
        );
    }
}

#[test]
fn havana_snapshots_remain_readable_without_constructing_a_grid() {
    use gammaboard::SavedSampler;

    for kind in ["havana_training", "havana_inference"] {
        let config: SamplingConfig = serde_json::from_value(json!({"kind":kind})).unwrap();
        let saved = SavedSampler {
            version: gammaboard::compatibility::MODEL_VERSION,
            config: config.clone(),
            state: json!({"grid":{"opaque":"backend-owned"}}),
        };
        let serialized = serde_json::to_value(&saved).unwrap();
        let restored: SavedSampler = serde_json::from_value(serialized.clone()).unwrap();
        assert!(restored.contains_havana_grid());
        assert_eq!(serde_json::to_value(restored).unwrap(), serialized);
        #[cfg(not(feature = "symbolica"))]
        {
            use gammaboard::StageHandoff;
            let domain = Domain::continuous(1);
            let handoff = StageHandoff {
                sampler_snapshot: Some(&saved),
                observable_state: None,
            };
            for error in [
                config
                    .create(domain.clone(), Some(32), json!({}))
                    .err()
                    .unwrap(),
                saved.clone().restore(&domain, json!({})).err().unwrap(),
                config.from_stage(&domain, handoff).err().unwrap(),
                config
                    .build_materializer(&domain, Some(handoff))
                    .err()
                    .unwrap(),
            ] {
                assert!(
                    error.to_string().contains("\"symbolica\" build feature"),
                    "{error}"
                );
            }
        }
    }
}
