//! JSON Schema validation for public specs.

use crate::compile::{CompileError, SpecVersion, ValidationIssue};
use jsonschema::JSONSchema;
use serde_json::Value;
use std::sync::OnceLock;

static SCHEMA_V1ALPHA1: OnceLock<JSONSchema> = OnceLock::new();
static SCHEMA_V1BETA1: OnceLock<JSONSchema> = OnceLock::new();

pub fn schema_v1alpha1() -> &'static JSONSchema {
    SCHEMA_V1ALPHA1
        .get_or_init(|| load_schema(include_str!("../../../spec/sandbox.v1alpha1.schema.json")))
}

pub fn schema_v1beta1() -> &'static JSONSchema {
    SCHEMA_V1BETA1
        .get_or_init(|| load_schema(include_str!("../../../spec/sandbox.v1beta1.schema.json")))
}

pub fn validate_raw(version: SpecVersion, raw: &Value) -> Result<(), CompileError> {
    let schema = match version {
        SpecVersion::V1Alpha1 => schema_v1alpha1(),
        SpecVersion::V1Beta1 => schema_v1beta1(),
    };

    match schema.validate(raw) {
        Ok(_) => Ok(()),
        Err(errors) => Err(CompileError::SchemaValidation {
            version,
            issues: errors.map(ValidationIssue::from).collect(),
        }),
    }
}

fn load_schema(raw_schema: &str) -> JSONSchema {
    let schema: Value = serde_json::from_str(raw_schema).expect("schema JSON valido");
    JSONSchema::options()
        .compile(&schema)
        .expect("schema compilabile")
}
