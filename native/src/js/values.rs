use super::*;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Target {
    pub(super) namespace: String,
    pub(super) path_segments: Vec<String>,
    pub(super) key_kind: String,
}
impl Target {
    pub(super) fn new(ns: &str, path: Vec<String>) -> Self {
        Self {
            namespace: ns.into(),
            path_segments: path,
            key_kind: "string".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value {
    Unknown,
    Strings(Vec<String>),
    Targets(Vec<Target>),
    Object(BTreeMap<String, Value>),
    Array(Vec<Value>),
    Node(String),
    Typed(&'static str),
    Number(f64),
    Bool(bool),
    Null,
    SymbolKey(String, String),
    DynamicTarget,
}
impl Value {
    pub(super) fn strings(&self) -> Vec<String> {
        match self {
            Self::Strings(s) => s.clone(),
            Self::Number(n) => vec![n.to_string()],
            Self::Bool(b) => vec![b.to_string()],
            Self::Null => vec!["null".into()],
            _ => vec![],
        }
    }
    pub(super) fn targets(&self) -> Vec<Target> {
        if let Self::Targets(s) = self {
            s.clone()
        } else {
            vec![]
        }
    }
    pub(super) fn kind(&self) -> &str {
        match self {
            Self::Strings(_) => "string",
            Self::Object(_) | Self::Node(_) => "object",
            Self::Array(_) => "array",
            Self::Targets(_) | Self::Unknown => "unknown",
            Self::Typed(s) => s,
            Self::Number(_) => "number",
            Self::Bool(_) => "boolean",
            Self::Null => "null",
            Self::SymbolKey(_, _) => "symbol",
            Self::DynamicTarget => "unknown",
        }
    }
}
pub(super) fn value_type(e: &Expression<'_>) -> &'static str {
    match e {
        Expression::NumericLiteral(_) => "number",
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => "string",
        Expression::BooleanLiteral(_) => "boolean",
        Expression::ObjectExpression(_) => "object",
        Expression::ArrayExpression(_) => "array",
        Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => "function",
        Expression::NullLiteral(_) => "null",
        _ => "unknown",
    }
}
pub(super) fn to_json(v: &Value) -> Option<Json> {
    match v {
        Value::Number(n) => Some(json!(n)),
        Value::Bool(b) => Some(json!(b)),
        Value::Null => Some(Json::Null),
        Value::Strings(s) if s.len() == 1 => Some(json!(s[0])),
        Value::Object(m) => m
            .iter()
            .map(|(k, v)| to_json(v).map(|v| (k.clone(), v)))
            .collect::<Option<serde_json::Map<_, _>>>()
            .map(Json::Object),
        Value::Array(a) => a
            .iter()
            .map(to_json)
            .collect::<Option<Vec<_>>>()
            .map(Json::Array),
        _ => None,
    }
}
pub(super) fn json_value(v: &Json) -> Value {
    match v {
        Json::String(s) => Value::Strings(vec![s.clone()]),
        Json::Number(n) => n.as_f64().map(Value::Number).unwrap_or(Value::Unknown),
        Json::Bool(b) => Value::Bool(*b),
        Json::Null => Value::Null,
        Json::Object(m) => {
            Value::Object(m.iter().map(|(k, v)| (k.clone(), json_value(v))).collect())
        }
        Json::Array(a) => Value::Array(a.iter().map(json_value).collect()),
    }
}
