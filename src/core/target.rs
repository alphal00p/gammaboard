//! Reference values and independent, one-standard-deviation uncertainties.
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub struct TargetComponent {
    pub value: f64,
    pub error: f64,
}

#[derive(Debug, Clone)]
pub struct RunTarget {
    pub components: BTreeMap<String, TargetComponent>,
}

impl RunTarget {
    pub fn parse(target: Option<&Value>, error: Option<&Value>) -> Result<Option<Self>, String> {
        let Some(target) = target else {
            return if error.is_some() {
                Err("target_error requires a target".into())
            } else {
                Ok(None)
            };
        };
        let values = if target.is_number() {
            BTreeMap::from([("value".to_owned(), target.clone())])
        } else if target.get("value").is_some() {
            BTreeMap::from([("value".to_owned(), target["value"].clone())])
        } else {
            target
                .get("components")
                .or_else(|| target.get("values"))
                .and_then(Value::as_object)
                .ok_or("target needs a scalar value or named components")?
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        };
        if values.is_empty() {
            return Err("target must contain at least one component".into());
        }
        let components = values
            .into_iter()
            .map(|(name, value)| {
                let value = value
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("target.{name} must be finite"))?;
                Ok((name, TargetComponent { value, error: 0.0 }))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let mut result = Self { components };
        if let Some(error) = error {
            let errors = if error.is_number() && result.components.len() == 1 {
                BTreeMap::from([(
                    result.components.keys().next().unwrap().clone(),
                    error.clone(),
                )])
            } else {
                error
                    .as_object()
                    .ok_or("target_error must be a number for a single target or a component map")?
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            };
            for (name, value) in errors {
                let error = value
                    .as_f64()
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .ok_or_else(|| format!("target_error.{name} must be finite and nonnegative"))?;
                let key = result
                    .key(&name)
                    .ok_or_else(|| format!("target_error component '{name}' has no target"))?
                    .to_owned();
                result.components.get_mut(&key).unwrap().error = error;
            }
        }
        Ok(Some(result))
    }

    fn key<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        if self.components.contains_key(name) {
            return Some(name);
        }
        let alias = match name {
            "real" => "value",
            "value" => "real",
            _ => return None,
        };
        self.components.contains_key(alias).then_some(alias)
    }

    pub fn integrand_component(&self, name: &str) -> Option<TargetComponent> {
        self.key(name)
            .and_then(|key| self.components.get(key))
            .copied()
    }

    pub fn component(&self, names: &[&str]) -> Option<TargetComponent> {
        names.iter().find_map(|name| self.integrand_component(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn partial_errors_default_to_zero_and_validate_components() {
        let target = json!({"kind":"vector", "components":{"real":1.0,"imag":2.0}});
        let parsed = RunTarget::parse(Some(&target), Some(&json!({"imag":0.2})))
            .unwrap()
            .unwrap();
        assert_eq!(parsed.integrand_component("value").unwrap().error, 0.0);
        assert_eq!(parsed.integrand_component("imag").unwrap().error, 0.2);
        for error in [
            json!(-1),
            json!({"real":-0.1}),
            json!({"imag":"bad"}),
            json!({"typo":0.1}),
        ] {
            assert!(RunTarget::parse(Some(&target), Some(&error)).is_err());
        }
        assert!(RunTarget::parse(None, Some(&json!({}))).is_err());
        for target in [json!(1.0), json!({"kind":"scalar","value":1.0})] {
            let parsed = RunTarget::parse(Some(&target), Some(&json!(0.3)))
                .unwrap()
                .unwrap();
            assert_eq!(parsed.integrand_component("real").unwrap().error, 0.3);
        }
    }
}
