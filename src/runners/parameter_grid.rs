use crate::core::StoreError;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct ParameterGridItem {
    pub name: String,
    pub values: Vec<toml::Value>,
}

pub fn cartesian_grid_len(items: &[ParameterGridItem]) -> Result<usize, StoreError> {
    items.iter().try_fold(1usize, |size, item| {
        size.checked_mul(item.values.len())
            .ok_or_else(|| StoreError::store("parameter grid size overflow"))
    })
}

pub fn cartesian_grid_point(
    items: &[ParameterGridItem],
    mut index: usize,
) -> Result<BTreeMap<String, toml::Value>, StoreError> {
    let mut selected = BTreeMap::new();
    for item in items.iter().rev() {
        if item.values.is_empty() {
            return Err(StoreError::store(format!(
                "parameter grid item '{}' has no values",
                item.name
            )));
        }
        let value_index = index % item.values.len();
        index /= item.values.len();
        selected.insert(item.name.clone(), item.values[value_index].clone());
    }
    if index != 0 {
        return Err(StoreError::store("parameter grid index exceeds grid size"));
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cartesian_grid_point_enumerates_last_parameter_fastest() {
        let items = vec![
            ParameterGridItem {
                name: "a".to_string(),
                values: vec![toml::Value::Integer(1), toml::Value::Integer(2)],
            },
            ParameterGridItem {
                name: "b".to_string(),
                values: vec![
                    toml::Value::String("x".to_string()),
                    toml::Value::String("y".to_string()),
                ],
            },
        ];

        assert_eq!(cartesian_grid_len(&items).unwrap(), 4);
        assert_eq!(
            cartesian_grid_point(&items, 0).unwrap(),
            BTreeMap::from([
                ("a".to_string(), toml::Value::Integer(1)),
                ("b".to_string(), toml::Value::String("x".to_string())),
            ])
        );
        assert_eq!(
            cartesian_grid_point(&items, 1).unwrap(),
            BTreeMap::from([
                ("a".to_string(), toml::Value::Integer(1)),
                ("b".to_string(), toml::Value::String("y".to_string())),
            ])
        );
        assert_eq!(
            cartesian_grid_point(&items, 2).unwrap(),
            BTreeMap::from([
                ("a".to_string(), toml::Value::Integer(2)),
                ("b".to_string(), toml::Value::String("x".to_string())),
            ])
        );
    }
}
