use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize_i64_as_string<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

pub fn serialize_option_i64_as_string<S>(
    value: &Option<i64>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(value) => serializer.serialize_some(&value.to_string()),
        None => serializer.serialize_none(),
    }
}

pub fn deserialize_i64_from_string_or_number<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(number) => number
            .as_i64()
            .ok_or_else(|| D::Error::custom("expected signed 64-bit integer")),
        serde_json::Value::String(text) => text
            .parse::<i64>()
            .map_err(|_| D::Error::custom("expected signed 64-bit integer string")),
        _ => Err(D::Error::custom("expected integer as number or string")),
    }
}
