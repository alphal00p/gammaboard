use std::collections::{BTreeMap, BTreeSet};

use serde::de::DeserializeOwned;

use crate::api::ApiError;

#[derive(Debug, Clone)]
pub struct ExpandedTomlTemplate {
    pub value: toml::Value,
    pub replacements: BTreeMap<String, toml::Value>,
    pub used_replacements: BTreeSet<String>,
}

#[derive(Debug)]
struct Placeholder {
    name: String,
    default_value: toml::Value,
}

pub fn expand_toml_template(mut value: toml::Value) -> Result<ExpandedTomlTemplate, ApiError> {
    let replacements = extract_replacements(&value)?;
    let mut used_replacements = BTreeSet::new();
    expand_value(&mut value, &replacements, &mut used_replacements, "$")?;
    if let Some(table) = value.as_table_mut() {
        table.remove("replacements");
    }
    Ok(ExpandedTomlTemplate {
        value,
        replacements,
        used_replacements,
    })
}

pub fn parse_templated_toml<T>(raw: &str, label: &str) -> Result<T, ApiError>
where
    T: DeserializeOwned,
{
    let value = toml::from_str(raw)
        .map_err(|err| ApiError::BadRequest(format!("failed parsing {label}: {err}")))?;
    let expanded = expand_toml_template(value)?;
    expanded
        .value
        .try_into()
        .map_err(|err| ApiError::BadRequest(format!("invalid {label}: {err}")))
}

fn extract_replacements(value: &toml::Value) -> Result<BTreeMap<String, toml::Value>, ApiError> {
    let Some(replacements) = value.as_table().and_then(|table| table.get("replacements")) else {
        return Ok(BTreeMap::new());
    };
    let table = replacements.as_table().ok_or_else(|| {
        ApiError::BadRequest("top-level replacements must be a TOML table".to_string())
    })?;
    Ok(table
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

fn expand_value(
    value: &mut toml::Value,
    replacements: &BTreeMap<String, toml::Value>,
    used_replacements: &mut BTreeSet<String>,
    path: &str,
) -> Result<(), ApiError> {
    match value {
        toml::Value::String(raw) => {
            if let Some(placeholder) = parse_full_placeholder(raw, path)? {
                let replacement = replacements
                    .get(&placeholder.name)
                    .cloned()
                    .unwrap_or(placeholder.default_value);
                used_replacements.insert(placeholder.name);
                *value = replacement;
            } else if raw.contains("$(") {
                let interpolated =
                    interpolate_placeholders(raw, replacements, used_replacements, path)?;
                *value = toml::Value::String(interpolated);
            }
        }
        toml::Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                expand_value(
                    item,
                    replacements,
                    used_replacements,
                    &format!("{path}[{index}]"),
                )?;
            }
        }
        toml::Value::Table(table) => {
            for (key, item) in table.iter_mut() {
                if path == "$" && key == "replacements" {
                    continue;
                }
                expand_value(
                    item,
                    replacements,
                    used_replacements,
                    &format!("{path}.{key}"),
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn parse_full_placeholder(raw: &str, path: &str) -> Result<Option<Placeholder>, ApiError> {
    if !raw.contains("$(") {
        return Ok(None);
    }
    if raw.starts_with("$(") && raw.find(')') == Some(raw.len() - 1) {
        return parse_placeholder(raw, path).map(Some);
    }
    Ok(None)
}

fn interpolate_placeholders(
    raw: &str,
    replacements: &BTreeMap<String, toml::Value>,
    used_replacements: &mut BTreeSet<String>,
    path: &str,
) -> Result<String, ApiError> {
    let mut output = String::with_capacity(raw.len());
    let mut remaining = raw;
    while let Some(start) = remaining.find("$(") {
        output.push_str(&remaining[..start]);
        let after_start = &remaining[start..];
        let Some(end) = after_start.find(')') else {
            return Err(ApiError::BadRequest(format!(
                "placeholder at {path} is missing closing ')'"
            )));
        };
        let placeholder_raw = &after_start[..=end];
        let placeholder = parse_placeholder(placeholder_raw, path)?;
        let replacement = replacements
            .get(&placeholder.name)
            .cloned()
            .unwrap_or(placeholder.default_value);
        used_replacements.insert(placeholder.name);
        output.push_str(&toml_value_to_interpolated_string(&replacement));
        remaining = &after_start[end + 1..];
    }
    output.push_str(remaining);
    Ok(output)
}

fn parse_placeholder(raw: &str, path: &str) -> Result<Placeholder, ApiError> {
    let body = raw
        .strip_prefix("$(")
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| {
            ApiError::BadRequest(format!("placeholder at {path} must use $(name:default)"))
        })?;
    let Some((name, default_raw)) = body.split_once(':') else {
        return Err(ApiError::BadRequest(format!(
            "placeholder at {path} must use $(name:default)"
        )));
    };
    validate_replacement_name(name, path)?;
    let default_value = parse_standalone_toml_value(default_raw.trim())
        .unwrap_or_else(|_| toml::Value::String(default_raw.trim().to_string()));
    Ok(Placeholder {
        name: name.to_string(),
        default_value,
    })
}

fn toml_value_to_interpolated_string(value: &toml::Value) -> String {
    match value {
        toml::Value::String(value) => value.clone(),
        toml::Value::Integer(value) => value.to_string(),
        toml::Value::Float(value) => value.to_string(),
        toml::Value::Boolean(value) => value.to_string(),
        toml::Value::Datetime(value) => value.to_string(),
        toml::Value::Array(_) | toml::Value::Table(_) => value.to_string(),
    }
}

fn validate_replacement_name(name: &str, path: &str) -> Result<(), ApiError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(ApiError::BadRequest(format!(
            "placeholder at {path} has an empty replacement name"
        )));
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(ApiError::BadRequest(format!(
            "placeholder at {path} replacement name must start with an ASCII letter or '_'"
        )));
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return Err(ApiError::BadRequest(format!(
            "placeholder at {path} replacement name may only contain ASCII letters, numbers, and '_'"
        )));
    }
    Ok(())
}

fn parse_standalone_toml_value(raw: &str) -> Result<toml::Value, toml::de::Error> {
    #[derive(serde::Deserialize)]
    struct Wrapper {
        value: toml::Value,
    }

    let wrapper: Wrapper = toml::from_str(&format!("value = {raw}"))?;
    Ok(wrapper.value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(raw: &str) -> toml::Value {
        let value = toml::from_str(raw).expect("valid toml");
        expand_toml_template(value).expect("expand").value
    }

    #[test]
    fn replaces_string_placeholders_with_typed_values() {
        let expanded = expand(
            r#"
replacements = { mu = 0.25, mode = "auto" }

[section]
mu = "$(mu:1.0)"
count = "$(count:5)"
enabled = "$(enabled:true)"
mode = '$(mode:"none")'
"#,
        );

        let section = expanded.get("section").unwrap();
        assert_eq!(
            section.get("mu").and_then(toml::Value::as_float),
            Some(0.25)
        );
        assert_eq!(
            section.get("count").and_then(toml::Value::as_integer),
            Some(5)
        );
        assert_eq!(
            section.get("enabled").and_then(toml::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            section.get("mode").and_then(toml::Value::as_str),
            Some("auto")
        );
        assert!(expanded.get("replacements").is_none());
    }

    #[test]
    fn uses_defaults_when_replacement_is_missing() {
        let expanded = expand(r#"value = "$(missing:[1, 2, 3])""#);
        assert_eq!(
            expanded
                .get("value")
                .and_then(toml::Value::as_array)
                .map(Vec::len),
            Some(3)
        );
    }

    #[test]
    fn interpolates_embedded_placeholders_as_strings() {
        let expanded = expand(
            r#"
replacements = { x = 5, mode = "auto" }

value = "{x:$(x:1), mode:$(mode:none), enabled:$(enabled:true)}"
"#,
        );
        assert_eq!(
            expanded.get("value").and_then(toml::Value::as_str),
            Some("{x:5, mode:auto, enabled:true}")
        );
    }

    #[test]
    fn treats_full_placeholder_as_typed_only_on_exact_match() {
        let expanded = expand(
            r#"
typed = "$(x:5)"
interpolated = " $(x:5)"
bare_string = "$(x:bare string)"
"#,
        );
        assert_eq!(
            expanded.get("typed").and_then(toml::Value::as_integer),
            Some(5)
        );
        assert_eq!(
            expanded.get("interpolated").and_then(toml::Value::as_str),
            Some(" 5")
        );
        assert_eq!(
            expanded.get("bare_string").and_then(toml::Value::as_str),
            Some("bare string")
        );
    }

    #[test]
    fn interpolates_multiple_placeholders_even_when_string_starts_with_placeholder() {
        let expanded = expand(r#"value = "$(x:1) $(y:2)""#);
        assert_eq!(
            expanded.get("value").and_then(toml::Value::as_str),
            Some("1 2")
        );
    }

    #[test]
    fn does_not_expand_inside_replacements_table() {
        let expanded = expand(
            r#"
replacements = { x = "$(other:5)" }

value = "$(x:1)"
"#,
        );
        assert_eq!(
            expanded.get("value").and_then(toml::Value::as_str),
            Some("$(other:5)")
        );
    }
}
