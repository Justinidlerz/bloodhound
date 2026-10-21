use napi::{
    Env, ValueType,
    bindgen_prelude::{FromNapiValue, Object, ToNapiValue, TypeName, Unknown, ValidateNapiValue},
    sys,
};
use serde_json::{Map, Value};

/// Convert each report record separately. napi-rs still bounds the depth and
/// node count of each record, without applying one node budget to the report.
pub struct ReportInput(pub Value);

impl TypeName for ReportInput {
    fn type_name() -> &'static str {
        "Report"
    }

    fn value_type() -> ValueType {
        ValueType::Object
    }
}

impl ValidateNapiValue for ReportInput {}

impl FromNapiValue for ReportInput {
    unsafe fn from_napi_value(env: sys::napi_env, value: sys::napi_value) -> napi::Result<Self> {
        let object = unsafe { Object::from_napi_value(env, value)? };
        let mut report = Map::new();
        for key in Object::keys(&object)? {
            let value = if crate::engine::COLLECTIONS.contains(&key.as_str()) {
                object.get::<Vec<Value>>(&key)?.map(Value::Array)
            } else {
                object.get::<Value>(&key)?
            };
            if let Some(value) = value {
                report.insert(key, value);
            }
        }
        Ok(Self(Value::Object(report)))
    }
}

/// Checkpoints embed a report, so use the same record conversion there.
pub struct CheckpointInput(pub Value);

impl TypeName for CheckpointInput {
    fn type_name() -> &'static str {
        "Checkpoint"
    }

    fn value_type() -> ValueType {
        ValueType::Object
    }
}

impl ValidateNapiValue for CheckpointInput {}

impl FromNapiValue for CheckpointInput {
    unsafe fn from_napi_value(env: sys::napi_env, value: sys::napi_value) -> napi::Result<Self> {
        let object = unsafe { Object::from_napi_value(env, value)? };
        let mut checkpoint = Map::new();
        for key in Object::keys(&object)? {
            let value = match key.as_str() {
                "report" => object.get::<ReportInput>(&key)?.map(|report| report.0),
                "pending" | "entries" => object.get::<Vec<Value>>(&key)?.map(Value::Array),
                _ => object.get::<Value>(&key)?,
            };
            if let Some(value) = value {
                checkpoint.insert(key, value);
            }
        }
        Ok(Self(Value::Object(checkpoint)))
    }
}

/// Rust report/query values are owned JSON trees. Use the same serializer as
/// scan results, including for report getters and large query result arrays.
pub struct JsonOutput(pub Value);

impl ToNapiValue for JsonOutput {
    unsafe fn to_napi_value(env: sys::napi_env, value: Self) -> napi::Result<sys::napi_value> {
        let js = Env::from(env).to_js_value(&value.0)?;
        unsafe { Unknown::to_napi_value(env, js) }
    }
}
