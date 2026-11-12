use super::*;
use crate::evaluation::GammaLoopDiagnostics;
use crate::server::panels::{key_value, key_value_panel, table_panel_with_payload};
use crate::server::task_panels::sample::{
    gammaloop_diagnostics_entries, gammaloop_timing_panel, point_value,
};
use std::collections::BTreeMap;

#[derive(sqlx::FromRow)]
struct ChildData {
    label: String,
    run_id: i32,
    task_name: Option<String>,
    domain: Value,
    evaluator: Option<Value>,
    accumulator: Option<Value>,
}
struct Child {
    label: String,
    run: i32,
    task: String,
    coefficient: f64,
    accumulator: AccumulatorState,
    has_diagnostics: bool,
}

fn decode(value: &Value) -> Result<AccumulatorState, crate::core::EngineError> {
    if value.get("kind").is_some() {
        AccumulatorState::from_json(value)
    } else if value.get("estimate").is_some() {
        AccumulatorState::from_gammaloop_persistent_json(value)
    } else {
        AccumulatorState::from_vector_persistent_json(value)
    }
}
fn components(accumulator: &AccumulatorState) -> Vec<String> {
    if let AccumulatorState::Stratified(s) = accumulator {
        s.plan.components.clone()
    } else {
        accumulator
            .pooled_vector()
            .map(|v| v.components.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default()
    }
}

pub(super) async fn load(
    store: &PgStore,
    run: i32,
    task: &RunTask,
    group: Option<&str>,
) -> Result<Value, ApiError> {
    // One statement selects a coherent source per child. No histogram histories are loaded.
    let rows: Vec<ChildData>=sqlx::query_as(r#"
        SELECT r.spawn_label AS label,r.id AS run_id,r.point_spec AS domain,t.name AS task_name,
            COALESCE(s.evaluator,r.integration_params->'evaluator') AS evaluator,
            gammaboard_diagnostic_value(CASE WHEN t.state='active' THEN COALESCE(r.current_observable,s.observable_state,p.persisted_observable)
                ELSE COALESCE(s.observable_state,p.persisted_observable) END) AS accumulator
        FROM runs r LEFT JOIN LATERAL (
            SELECT id,name,state,sequence_nr FROM run_tasks WHERE run_id=r.id AND task->>'kind'='sample'
                AND COALESCE((task->>'publish_result')::bool,true) AND (state='completed' OR (state='active' AND nr_completed_samples>0))
            ORDER BY sequence_nr DESC LIMIT 1
        ) t ON true LEFT JOIN LATERAL (
            SELECT observable_state,evaluator FROM run_stage_snapshots WHERE run_id=r.id AND task_id=t.id ORDER BY id DESC LIMIT 1
        ) s ON true LEFT JOIN LATERAL (
            SELECT persisted_observable FROM persisted_observable_snapshots WHERE run_id=r.id AND task_id=t.id ORDER BY id DESC LIMIT 1
        ) p ON true WHERE r.parent_run_id=$1 AND r.parent_task_id=$2 AND r.spawn_kind='integration_campaign' ORDER BY r.id
    "#).bind(run).bind(task.id).fetch_all(store.pool()).await.map_err(crate::core::StoreError::from)?;
    let RunTaskSpec::IntegrationCampaign {
        children: definitions,
        ..
    } = &task.task
    else {
        unreachable!()
    };
    let mut groups: BTreeMap<String, Vec<Child>> = BTreeMap::new();
    let mut unavailable = vec![];
    for definition in definitions {
        let Some(row) = rows.iter().find(|r| r.label == definition.name) else {
            unavailable.push(format!("{}: not created", definition.name));
            continue;
        };
        let Some(value) = &row.accumulator else {
            unavailable.push(format!("{}: no published result yet", row.label));
            continue;
        };
        let accumulator = match decode(value) {
            Ok(a) if a.sample_count() > 0 && !components(&a).is_empty() => a,
            _ => {
                unavailable.push(format!("{}: diagnostic data unavailable", row.label));
                continue;
            }
        };
        // GammaLoop components/timing counters have shared units and semantics.
        // Other evaluators require identical recorded configurations as well.
        let evaluator = if accumulator.gammaloop().is_some() {
            json!({"kind":"gammaloop"})
        } else {
            row.evaluator
                .clone()
                .unwrap_or(json!({"unknown_child":row.run_id}))
        };
        let key = serde_json::to_string(&json!([components(&accumulator), row.domain, evaluator]))
            .unwrap();
        let has_diagnostics =
            value.get("diagnostics").is_some() || value.pointer("/pooled/diagnostics").is_some();
        groups.entry(key).or_default().push(Child {
            label: row.label.clone(),
            run: row.run_id,
            task: row.task_name.clone().unwrap_or_default(),
            coefficient: definition.coefficient,
            accumulator,
            has_diagnostics,
        });
    }
    // Stable group IDs preserve the selected evaluator family as other children publish.
    fn group_id(key: &str) -> String {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hash);
        format!("{:016x}", hash.finish())
    }
    let choices=groups.iter().map(|(key,g)|json!({"id":group_id(key),"label":g.iter().map(|c|c.label.as_str()).collect::<Vec<_>>().join(", ")})).collect::<Vec<_>>();
    let chosen = groups
        .iter()
        .find(|(key, _)| Some(group_id(key)).as_deref() == group)
        .or_else(|| groups.iter().next());
    let group = chosen.map(|(key, _)| group_id(key)).unwrap_or_default();
    let selected = chosen.map(|(_, v)| v.as_slice()).unwrap_or(&[]);
    let mut panels = vec![(
        "Diagnostic coverage",
        key_value_panel(
            "campaign_diagnostic_coverage",
            vec![
                key_value(
                    "scope",
                    "Source",
                    "Latest usable publishing stage per child",
                ),
                key_value(
                    "coverage",
                    "Children in this group / campaign",
                    format!("{} / {}", selected.len(), definitions.len()),
                ),
                key_value(
                    "missing",
                    "Unavailable children",
                    if unavailable.is_empty() {
                        "None".into()
                    } else {
                        unavailable.join("; ")
                    },
                ),
            ],
        ),
    )];
    if !selected.is_empty() {
        let mut diagnostics = GammaLoopDiagnostics::default();
        let mut diagnostic_count = 0;
        for child in selected {
            if child.has_diagnostics
                && let Some(g) = child.accumulator.gammaloop()
            {
                diagnostics.merge_in_place(g.diagnostics.clone());
                diagnostic_count += 1;
            }
        }
        if diagnostic_count > 0 {
            let mut entries = gammaloop_diagnostics_entries(&diagnostics);
            entries.insert(
                0,
                key_value(
                    "coverage",
                    "Children with evaluation diagnostics",
                    format!("{diagnostic_count} / {}", selected.len()),
                ),
            );
            panels.push((
                "Evaluation Diagnostics",
                key_value_panel("gammaloop_evaluation_diagnostics", entries),
            ));
            if let Some(panel) = gammaloop_timing_panel(&diagnostics) {
                panels.push(("Evaluation Timing Breakdown", panel));
            }
        }
        let (impact, points) = weight_panels(selected, unavailable.is_empty());
        panels.push(("Max Weight Impact", impact));
        panels.push(("Max Weight Points", points));
    }
    let mut result = response(
        format!("campaign:{run}:{}:diagnostics:{group}", task.id),
        panels,
    );
    result["groups"] = json!(choices);
    result["selected_group"] = json!(group);
    Ok(result)
}

struct Extreme {
    child: String,
    task: String,
    run: i32,
    component: String,
    stratum: Value,
    raw: f64,
    contribution: f64,
    point: Value,
}
fn weight_panels(children: &[Child], complete: bool) -> (panels::PanelState, panels::PanelState) {
    let mut means: BTreeMap<String, f64> = BTreeMap::new();
    let mut points = vec![];
    let mut valid = complete;
    for child in children {
        if let AccumulatorState::Stratified(s) = &child.accumulator {
            for (k, stratum) in s.strata.iter().enumerate() {
                if stratum.attempts == 0 {
                    valid = false;
                    continue;
                }
                for (j, name) in s.plan.components.iter().enumerate() {
                    *means.entry(name.clone()).or_default() += child.coefficient * stratum.mean[j];
                    for (raw, point) in [&stratum.extrema[j].positive, &stratum.extrema[j].negative]
                        .into_iter()
                        .flatten()
                    {
                        points.push(Extreme {
                            child: child.label.clone(),
                            task: child.task.clone(),
                            run: child.run,
                            component: name.clone(),
                            stratum: json!(s.plan.partition.prefix(k)),
                            raw: *raw,
                            contribution: child.coefficient * raw / stratum.attempts as f64,
                            point: point_value(Some(point)),
                        });
                    }
                }
            }
        } else if let Some(v) = child.accumulator.pooled_vector() {
            for c in &v.components {
                if c.state.count <= 0 {
                    valid = false;
                    continue;
                }
                *means.entry(c.name.clone()).or_default() += child.coefficient * c.state.mean();
                for (raw, point) in [
                    (
                        c.state.max_weighted_positive,
                        &c.state.max_weighted_positive_sample,
                    ),
                    (
                        c.state.max_weighted_negative,
                        &c.state.max_weighted_negative_sample,
                    ),
                ] {
                    if point.is_none() && raw == 0.0 {
                        continue;
                    }
                    points.push(Extreme {
                        child: child.label.clone(),
                        task: child.task.clone(),
                        run: child.run,
                        component: c.name.clone(),
                        stratum: Value::Null,
                        raw,
                        contribution: child.coefficient * raw / c.state.count as f64,
                        point: point_value(point.as_ref()),
                    });
                }
            }
        }
    }
    let impact = |p: &Extreme| {
        means
            .get(&p.component)
            .filter(|m| valid && **m != 0.0)
            .map(|m| p.contribution.abs() / m.abs())
    };
    points.sort_by(|a, b| b.contribution.abs().total_cmp(&a.contribution.abs()));
    let rows = means
        .iter()
        .map(|(component, mean)| {
            let p = points.iter().find(|p| p.component == *component);
            vec![
                json!(component),
                json!(valid.then_some(mean)),
                json!(p.and_then(impact)),
                json!(p.map(|p| p.child.as_str())),
            ]
        })
        .collect();
    let summary = table_panel_with_payload(
        "max_weight_summary",
        [
            "Component",
            "Group estimate",
            "Largest point impact",
            "Child",
        ]
        .map(String::from)
        .to_vec(),
        rows,
        Some(
            json!({"column_help":{"Largest point impact":"Absolute contribution to this group's estimate divided by its magnitude. Child coefficients and per-stratum attempt counts are included. Unavailable when child data is missing or the estimate is zero."}}),
        ),
    );
    let rows = points
        .iter()
        .take(100)
        .map(|p| {
            vec![
                json!(p.child),
                json!(p.run),
                json!(p.task),
                p.stratum.clone(),
                json!(p.component),
                json!(if p.contribution < 0.0 { "−" } else { "+" }),
                json!(p.raw),
                json!(p.contribution),
                json!(impact(p)),
                p.point.clone(),
            ]
        })
        .collect();
    let table = table_panel_with_payload(
        "max_weight_points",
        [
            "Child",
            "Run",
            "Task",
            "Stratum",
            "Component",
            "Sign",
            "Max Weighted Value",
            "Campaign contribution",
            "Impact",
            "Point",
        ]
        .map(String::from)
        .to_vec(),
        rows,
        Some(
            json!({"description":"Largest retained extrema (up to 100), ranked by absolute contribution to the selected group's estimate. These are retained diagnostic points, not every sample.",
            "column_formats":{"Max Weighted Value":"scientific","Campaign contribution":"scientific","Impact":"scientific"},
            "column_help":{"Impact":"Absolute contribution to the selected group estimate divided by its magnitude, including the child coefficient and stratum attempt count. Missing child data or a zero estimate leaves this unavailable.","Campaign contribution":"Coefficient × weighted value / sample count. Stratified values use the conditional contribution and that stratum's attempt count."}}),
        ),
    );
    (summary, table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{AccumulatorMomentConfig, TrainingProjection};
    use crate::evaluation::{Point, VectorAccumulatorState};
    fn child(label: &str, coefficient: f64, values: &[f64]) -> Child {
        let mut state = VectorAccumulatorState::from_config(
            vec!["real".into()],
            TrainingProjection::component("real"),
            None,
            AccumulatorMomentConfig::MaxOrder2,
        );
        for value in values {
            state
                .ingest_vector(&[*value], &Point::new(vec![0.5], vec![], 1.0))
                .unwrap();
        }
        Child {
            label: label.into(),
            run: 1,
            task: "infer".into(),
            coefficient,
            accumulator: AccumulatorState::Vector(state),
            has_diagnostics: false,
        }
    }
    #[test]
    fn extrema_use_child_coefficients_sample_counts_and_signed_campaign_mean() {
        let (summary, table) = weight_panels(
            &[
                child("A", 2.0, &[2., 4.]),
                child("B", -1.0, &[1., 1., 1., 5.]),
            ],
            true,
        );
        let summary = serde_json::to_value(summary).unwrap();
        let table = serde_json::to_value(table).unwrap();
        assert_eq!(summary["rows"][0][1], 4.0); // 2*3 - 2
        assert_eq!(table["rows"][0][0], "A");
        assert_eq!(table["rows"][0][7], 4.0); // 2*4 / 2
        assert_eq!(table["rows"][0][8], 1.0);
        let b = table["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r[0] == "B" && r[6] == 5.0)
            .unwrap();
        assert_eq!(b[7], -1.25);
        assert_eq!(b[5], "−");
    }
    #[test]
    fn missing_children_or_zero_estimate_leave_impact_unavailable() {
        for (children, complete) in [
            (vec![child("A", 1.0, &[1., 3.])], false),
            (vec![child("A", 1.0, &[-1., 1.])], true),
        ] {
            let (_, points) = weight_panels(&children, complete);
            let value = serde_json::to_value(points).unwrap();
            assert!(
                value["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r[8].is_null())
            );
        }
    }
}

#[cfg(test)]
mod stratified_tests {
    use super::*;
    use crate::Point;
    use crate::core::{
        AccumulatorConfig, StratificationConfig, StratificationPlan, TrainingProjection,
    };
    use crate::evaluation::accumulator::StratifiedAccumulatorState;
    #[test]
    fn campaign_extrema_respect_each_stratum_attempt_count() {
        let plan = StratificationPlan {
            proposal_id: "test".into(),
            partition: StratificationConfig::Voronoi {
                domain: crate::Domain::strata(vec![2]),
                centers: [(vec![0], vec![0., 0.]), (vec![1], vec![1., 1.])]
                    .into_iter()
                    .collect(),
                scales: None,
            },
            probabilities: Some(vec![0.25, 0.75]),
            components: vec!["x".into(), "y".into()],
        };
        let mut state = StratifiedAccumulatorState::new(
            plan,
            AccumulatorConfig::vector(
                vec!["x".into(), "y".into()],
                TrainingProjection::component("x"),
            ),
        )
        .unwrap();
        for (k, values, weight) in [(0, vec![2., 4.], 4.), (1, vec![1., 1., 1., 5.], 4. / 3.)] {
            let label = [k as i64];
            let point = Point::new(vec![0.5, 0.5], vec![k as i64], weight);
            for value in values {
                state
                    .push_with_diagnostics(
                        k,
                        &[value, 0.0],
                        weight,
                        true,
                        Some((&point, (&label, &label))),
                    )
                    .unwrap();
            }
        }
        let child = Child {
            label: "stratified".into(),
            run: 1,
            task: "infer".into(),
            coefficient: 2.,
            accumulator: AccumulatorState::Stratified(Box::new(state)),
            has_diagnostics: false,
        };
        let (summary, points) = weight_panels(&[child], true);
        let summary = serde_json::to_value(summary).unwrap();
        let points = serde_json::to_value(points).unwrap();
        assert_eq!(summary["rows"][0][1], 10.);
        assert_eq!(points["rows"][0][3], json!([0]));
        assert_eq!(points["rows"][0][6], 4.);
        assert_eq!(points["rows"][0][7], 4.);
        assert_eq!(points["rows"][0][8], 0.4);
    }
}
