use crate::api::ApiError;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

fn bad(message: &str) -> ApiError {
    ApiError::BadRequest(message.into())
}

fn positive_count(raw: &str) -> Result<u64, ApiError> {
    raw.parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| bad("GPU counts must be positive integers"))
}

fn gpu_count(raw: &str, gres: bool) -> Result<u64, ApiError> {
    let mut total = 0u64;
    for segment in raw.split(',').map(str::trim) {
        let parts = segment.split(':').collect::<Vec<_>>();
        let count = if gres {
            if parts.first() != Some(&"gpu") {
                continue;
            }
            match parts.as_slice() {
                ["gpu"] => 1,
                ["gpu", value] => {
                    if value.bytes().all(|b| b.is_ascii_digit()) {
                        positive_count(value)?
                    } else if value.is_empty() {
                        return Err(bad("empty GPU type"));
                    } else {
                        1
                    }
                }
                ["gpu", kind, value] if !kind.is_empty() => positive_count(value)?,
                _ => return Err(bad("invalid GPU gres specification")),
            }
        } else {
            match parts.as_slice() {
                [value] => positive_count(value)?,
                [kind, value] if !kind.is_empty() => positive_count(value)?,
                _ => return Err(bad("gpus must be a count or type:count")),
            }
        };
        total = total
            .checked_add(count)
            .ok_or_else(|| bad("GPU count overflow"))?;
    }
    Ok(total)
}

/// One canonical recipe feeds local/Slurm registration, scheduling and GPU setup.
pub(super) fn normalize(config: &Value) -> Result<(Value, BTreeMap<String, u64>), ApiError> {
    let mut config: Map<String, Value> = config
        .as_object()
        .cloned()
        .ok_or_else(|| bad("group config must be an object"))?;
    let mut cpus = None;
    for key in [
        "cores",
        "nr_cores",
        "cpus",
        "cpus_per_task",
        "cpus-per-task",
    ] {
        if let Some(value) = config.remove(key) {
            let n = value
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or_else(|| bad("CPU allowance must be a positive integer"))?;
            if cpus.is_some_and(|previous| previous != n) {
                return Err(bad("conflicting CPU resource aliases"));
            }
            cpus = Some(n);
        }
    }
    if let Some(n) = cpus {
        config.insert("cpus_per_task".into(), json!(n));
    }
    if config.contains_key("mem")
        && (config.contains_key("mem_per_cpu") || config.contains_key("mem-per-cpu"))
    {
        return Err(bad("mem and mem_per_cpu are mutually exclusive"));
    }
    if let Some(value) = config.remove("mem-per-cpu") {
        if config.get("mem_per_cpu").is_some_and(|old| old != &value) {
            return Err(bad("conflicting memory resource aliases"));
        }
        config.insert("mem_per_cpu".into(), value);
    }
    let shorthand = config
        .remove("gpu")
        .map(|value| match value {
            Value::Bool(false) => Ok(None),
            Value::Bool(true) => Ok(Some("gpu:rtx4090:1".to_string())),
            Value::Number(n) => n
                .as_u64()
                .map(|n| (n > 0).then(|| format!("gpu:rtx4090:{n}")))
                .ok_or_else(|| bad("invalid GPU count")),
            Value::String(s) if !s.trim().is_empty() => Ok(Some(if s.starts_with("gpu:") {
                s
            } else if s.bytes().all(|c| c.is_ascii_digit()) {
                format!("gpu:rtx4090:{s}")
            } else {
                format!("gpu:{s}")
            })),
            _ => Err(bad("gpu must be a positive count or GPU type:count")),
        })
        .transpose()?
        .flatten();
    let gres = config
        .get("gres")
        .map(|v| v.as_str().ok_or_else(|| bad("gres must be a string")))
        .transpose()?
        .unwrap_or("")
        .to_owned();
    let from_gres = gpu_count(&gres, true)?;
    let from_gpus = config
        .get("gpus")
        .map(|value| {
            if let Some(n) = value.as_u64() {
                if n > 0 {
                    Ok(n)
                } else {
                    Err(bad("gpus must be positive"))
                }
            } else {
                gpu_count(
                    value
                        .as_str()
                        .ok_or_else(|| bad("gpus must be count or type:count"))?,
                    false,
                )
            }
        })
        .transpose()?;
    if usize::from(shorthand.is_some())
        + usize::from(from_gres > 0)
        + usize::from(from_gpus.is_some())
        > 1
    {
        return Err(bad(
            "specify GPU allocation with only one of gpu, gpus, or GPU gres",
        ));
    }
    let gpu = if let Some(value) = shorthand {
        let n = gpu_count(&value, true)?;
        config.insert(
            "gres".into(),
            json!(if gres.is_empty() {
                value
            } else {
                format!("{gres},{value}")
            }),
        );
        n
    } else {
        from_gpus.unwrap_or(from_gres)
    };
    if gpu > 0 {
        config.entry("partition").or_insert(json!("gpu"));
    }
    let mut caps = BTreeMap::new();
    for (key, value) in &config {
        if !matches!(key.as_str(), "gpus" | "cpus_per_task")
            && let Some(n) = value.as_u64()
        {
            caps.insert(key.clone(), n);
        }
    }
    if let Some(n) = cpus {
        caps.insert("cpus".into(), n);
    }
    if gpu > 0 {
        caps.insert("gpu".into(), gpu);
    }
    Ok((Value::Object(config), caps))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_resources_agree_for_typed_and_shorthand_gpu_forms() {
        for value in [
            json!({"gpu":"h100:2","cores":4}),
            json!({"gpus":"h100:2","cpus":4}),
            json!({"gres":"gpu:h100:2","cpus_per_task":4}),
        ] {
            let (config, caps) = normalize(&value).unwrap();
            assert_eq!(caps.get("gpu"), Some(&2));
            assert_eq!(caps.get("cpus"), Some(&4));
            assert_eq!(config["partition"], "gpu");
            assert_eq!(config["cpus_per_task"], 4);
        }
        for key in [
            "cores",
            "nr_cores",
            "cpus",
            "cpus_per_task",
            "cpus-per-task",
        ] {
            let (config, caps) = normalize(&json!({key:8})).unwrap();
            assert_eq!(config, json!({"cpus_per_task":8}));
            assert_eq!(caps, BTreeMap::from([("cpus".into(), 8)]));
        }
        assert_eq!(
            normalize(&json!({"gres":"gpu:h100:1,gpu:a100:2"}))
                .unwrap()
                .1["gpu"],
            3
        );
        for value in [
            json!({"cpus":1,"cores":2}),
            json!({"gpus":1,"gpu":1}),
            json!({"gpu":1,"gres":"gpu:2"}),
            json!({"mem":"2G","mem_per_cpu":"1G"}),
            json!({"cpus":true}),
        ] {
            assert!(normalize(&value).is_err(), "{value}");
        }
    }
}
