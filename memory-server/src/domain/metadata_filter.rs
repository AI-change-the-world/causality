//! Profile-defined metadata filter DSL.
//!
//! The filter is intentionally small: it is used to narrow retrieval
//! candidates before scoring, not to become a general query language.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use utoipa::ToSchema;

use crate::error::{AppError, AppResult};

const MAX_FILTER_CLAUSES: usize = 16;
const MAX_FILTER_FIELDS: usize = 32;
const MAX_IN_VALUES: usize = 64;

/// Metadata filter passed by callers after reading the profile schema.
///
/// Semantics:
/// - `where` clauses are combined with AND.
/// - Fields inside each clause are also combined with AND.
/// - Field names must be declared by the profile metadata schema as filterable.
///
/// Example:
/// `{ "where": [{ "memory_type": { "in": ["preference", "fact"] } }] }`
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct MetadataFilter {
    #[serde(default, rename = "where")]
    pub clauses: Vec<MetadataFilterClause>,
}

/// One AND clause in a metadata filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ToSchema)]
#[serde(transparent)]
pub struct MetadataFilterClause(pub BTreeMap<String, MetadataFilterPredicate>);

/// Predicate for one metadata field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
#[serde(untagged)]
pub enum MetadataFilterPredicate {
    Operators(MetadataFilterOperators),
    Eq(Value),
}

/// Supported metadata field operators.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct MetadataFilterOperators {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ne: Option<Value>,
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub in_values: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
}

impl MetadataFilter {
    pub fn is_empty(&self) -> bool {
        self.clauses.is_empty()
    }

    pub fn field_count(&self) -> usize {
        self.clauses.iter().map(|clause| clause.0.len()).sum()
    }

    pub fn validate_against_schema(&self, metadata_schema: &Value) -> AppResult<()> {
        if self.clauses.len() > MAX_FILTER_CLAUSES {
            return Err(AppError::Validation(format!(
                "metadata_filter.where supports at most {} clauses",
                MAX_FILTER_CLAUSES
            )));
        }

        if self.field_count() > MAX_FILTER_FIELDS {
            return Err(AppError::Validation(format!(
                "metadata_filter supports at most {} field predicates",
                MAX_FILTER_FIELDS
            )));
        }

        if self.is_empty() {
            return Ok(());
        }

        let filterable_fields = filterable_fields_from_schema(metadata_schema);
        if filterable_fields.is_empty() {
            return Err(AppError::Validation(
                "metadata_filter requires profile metadata_schema.filterable_fields".to_string(),
            ));
        }

        for clause in &self.clauses {
            if clause.0.is_empty() {
                return Err(AppError::Validation(
                    "metadata_filter.where cannot contain empty clauses".to_string(),
                ));
            }

            for (field, predicate) in &clause.0 {
                if !filterable_fields.contains(field) {
                    return Err(AppError::Validation(format!(
                        "metadata_filter field '{}' is not declared filterable by profile schema",
                        field
                    )));
                }
                predicate.validate(field)?;
            }
        }

        Ok(())
    }

    pub fn matches(&self, metadata: &Value) -> bool {
        self.clauses.iter().all(|clause| {
            clause.0.iter().all(|(field, predicate)| {
                let actual = lookup_metadata_value(metadata, field);
                predicate.matches(actual)
            })
        })
    }
}

impl MetadataFilterPredicate {
    fn validate(&self, field: &str) -> AppResult<()> {
        match self {
            MetadataFilterPredicate::Eq(_) => Ok(()),
            MetadataFilterPredicate::Operators(operators) => {
                let operator_count = usize::from(operators.eq.is_some())
                    + usize::from(operators.ne.is_some())
                    + usize::from(operators.in_values.is_some())
                    + usize::from(operators.exists.is_some());

                if operator_count == 0 {
                    return Err(AppError::Validation(format!(
                        "metadata_filter field '{}' has no operator",
                        field
                    )));
                }

                if let Some(values) = &operators.in_values {
                    if values.is_empty() {
                        return Err(AppError::Validation(format!(
                            "metadata_filter field '{}'.in cannot be empty",
                            field
                        )));
                    }
                    if values.len() > MAX_IN_VALUES {
                        return Err(AppError::Validation(format!(
                            "metadata_filter field '{}'.in supports at most {} values",
                            field, MAX_IN_VALUES
                        )));
                    }
                }

                Ok(())
            }
        }
    }

    fn matches(&self, actual: Option<&Value>) -> bool {
        match self {
            MetadataFilterPredicate::Eq(expected) => actual
                .map(|actual| metadata_value_eq(actual, expected))
                .unwrap_or(false),
            MetadataFilterPredicate::Operators(operators) => operators.matches(actual),
        }
    }
}

impl MetadataFilterOperators {
    fn matches(&self, actual: Option<&Value>) -> bool {
        if let Some(exists) = self.exists {
            let actual_exists = actual.is_some_and(|value| !value.is_null());
            if actual_exists != exists {
                return false;
            }
        }

        if let Some(expected) = &self.eq {
            if !actual
                .map(|actual| metadata_value_eq(actual, expected))
                .unwrap_or(false)
            {
                return false;
            }
        }

        if let Some(expected) = &self.ne {
            if !actual
                .map(|actual| !metadata_value_eq(actual, expected))
                .unwrap_or(false)
            {
                return false;
            }
        }

        if let Some(values) = &self.in_values {
            if !actual
                .map(|actual| metadata_value_in(actual, values))
                .unwrap_or(false)
            {
                return false;
            }
        }

        true
    }
}

fn filterable_fields_from_schema(metadata_schema: &Value) -> BTreeSet<String> {
    let mut fields = BTreeSet::new();

    if let Some(values) = metadata_schema
        .get("filterable_fields")
        .and_then(|value| value.as_array())
    {
        for value in values {
            if let Some(field) = value.as_str() {
                fields.insert(field.to_string());
            }
        }
    }

    if let Some(entity_types) = metadata_schema
        .get("entity_types")
        .and_then(|value| value.as_object())
    {
        for entity in entity_types.values() {
            if let Some(entity_fields) = entity.get("fields").and_then(|value| value.as_object()) {
                for (field_name, field_schema) in entity_fields {
                    let is_filterable = field_schema
                        .get("filterable")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                    if is_filterable {
                        fields.insert(field_name.clone());
                    }
                }
            }
        }
    }

    fields
}

fn lookup_metadata_value<'a>(metadata: &'a Value, field: &str) -> Option<&'a Value> {
    let object = metadata.as_object()?;
    if let Some(value) = object.get(field) {
        return Some(value);
    }

    let mut current = metadata;
    for part in field.split('.') {
        current = current.get(part)?;
    }

    Some(current)
}

fn metadata_value_eq(actual: &Value, expected: &Value) -> bool {
    match actual {
        Value::Array(values) => values.iter().any(|value| value == expected),
        _ => actual == expected,
    }
}

fn metadata_value_in(actual: &Value, expected_values: &[Value]) -> bool {
    match actual {
        Value::Array(values) => values
            .iter()
            .any(|value| expected_values.iter().any(|expected| expected == value)),
        _ => expected_values.iter().any(|expected| expected == actual),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "filterable_fields": ["memory_type", "subject", "topics"],
            "entity_types": {}
        })
    }

    #[test]
    fn validates_filterable_fields() {
        let filter: MetadataFilter = serde_json::from_value(json!({
            "where": [
                {"memory_type": {"eq": "preference"}},
                {"subject": {"in": ["rust", "cpp"]}}
            ]
        }))
        .unwrap();

        assert!(filter.validate_against_schema(&schema()).is_ok());
    }

    #[test]
    fn rejects_non_filterable_fields() {
        let filter: MetadataFilter = serde_json::from_value(json!({
            "where": [{"private_note": {"eq": "x"}}]
        }))
        .unwrap();

        assert!(matches!(
            filter.validate_against_schema(&schema()),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn matches_eq_in_and_array_values() {
        let filter: MetadataFilter = serde_json::from_value(json!({
            "where": [
                {"memory_type": "preference"},
                {"subject": {"in": ["rust", "cpp"]}},
                {"topics": {"in": ["systems"]}}
            ]
        }))
        .unwrap();
        let metadata = json!({
            "memory_type": "preference",
            "subject": "rust",
            "topics": ["systems", "memory"]
        });

        assert!(filter.matches(&metadata));
    }

    #[test]
    fn supports_exists_false() {
        let filter: MetadataFilter = serde_json::from_value(json!({
            "where": [{"subject": {"exists": false}}]
        }))
        .unwrap();

        assert!(filter.matches(&json!({"memory_type": "fact"})));
        assert!(!filter.matches(&json!({"subject": "rust"})));
    }
}
