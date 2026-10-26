//! Abstract values tracked during JavaScript analysis.
//!
//! The analyzer does not execute code. It tracks a small lattice of facts
//! about expressions that matter for pollution detection: whether an
//! expression refers to the global object, `document`, a builtin, a DOM
//! element created by script, or a finite set of strings (for dependency
//! specifiers). Everything else is `Unknown`.

use std::collections::BTreeMap;

/// The root of a global reference chain.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Root {
    /// `window`, `globalThis`, `self`, top-level `this` in a classic script.
    Global,
    /// `document`.
    Document,
    /// A standard builtin such as `Object`, `Array`, `Promise`.
    Builtin(&'static str),
    /// A host API object or constructor such as `fetch`, `XMLHttpRequest`,
    /// `history`, `navigator`.
    Host(&'static str),
}

impl Root {
    pub fn display_name(&self) -> &str {
        match self {
            Self::Global => "window",
            Self::Document => "document",
            Self::Builtin(name) | Self::Host(name) => name,
        }
    }
}

/// A reference to a global, document, or builtin property path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub root: Root,
    /// Property path below the root, e.g. `["body", "style"]`.
    pub path: Vec<String>,
}

impl Target {
    pub fn new(root: Root) -> Self {
        Self { root, path: vec![] }
    }

    pub fn child(&self, key: &str) -> Self {
        let mut path = self.path.clone();
        path.push(key.to_string());
        Self {
            root: self.root.clone(),
            path,
        }
    }

    /// Dotted display form, e.g. `window.foo.bar`.
    pub fn dotted(&self) -> String {
        let mut s = self.root.display_name().to_string();
        for p in &self.path {
            s.push('.');
            s.push_str(p);
        }
        s
    }

    /// `true` when the path is `prototype` or passes through it.
    pub fn touches_prototype(&self) -> bool {
        self.path
            .iter()
            .any(|p| p == "prototype" || p == "__proto__")
    }
}

/// A DOM element created by `document.createElement(tag)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// Allocation/selection site; aliases retain this identity.
    pub id: u32,
    pub tag: String,
    /// Known `src`/`href` string candidates assigned to the node.
    pub urls: Vec<String>,
    pub rel: String,
    pub module: bool,
}

impl Node {
    pub fn new(id: u32, tag: &str) -> Self {
        Self {
            id,
            tag: tag.into(),
            urls: vec![],
            rel: String::new(),
            module: false,
        }
    }
}

/// Abstract value of an expression.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Unknown,
    /// Finite set of possible string values.
    Strings(Vec<String>),
    Number(f64),
    Bool(bool),
    Null,
    Undefined,
    /// Candidate global targets (usually one; several after a branch merge).
    Targets(Vec<Target>),
    /// A script-created DOM element.
    Node(Node),
    /// An object literal with known constant properties.
    Object(BTreeMap<String, Value>),
    /// An array literal with known constant elements.
    Array(Vec<Value>),
    /// A function expression (opaque).
    Function,
}

impl Value {
    pub fn strings(&self) -> Vec<String> {
        match self {
            Self::Strings(s) => s.clone(),
            Self::Number(n) => vec![format_number(*n)],
            Self::Bool(b) => vec![b.to_string()],
            Self::Null => vec!["null".into()],
            Self::Undefined => vec!["undefined".into()],
            _ => vec![],
        }
    }

    pub fn targets(&self) -> &[Target] {
        match self {
            Self::Targets(t) => t,
            _ => &[],
        }
    }

    pub fn is_tracked(&self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// Human readable type for the `detail` field of findings.
    pub fn type_name(&self) -> Option<&'static str> {
        match self {
            Self::Strings(_) => Some("string"),
            Self::Number(_) => Some("number"),
            Self::Bool(_) => Some("boolean"),
            Self::Null => Some("null"),
            Self::Undefined => Some("undefined"),
            Self::Object(_) => Some("object"),
            Self::Array(_) => Some("array"),
            Self::Function => Some("function"),
            Self::Node(_) => Some("element"),
            Self::Targets(_) | Self::Unknown => None,
        }
    }

    /// Joins two branch values. Equal values survive; string and target sets
    /// union (bounded by `max_candidates`); anything else becomes unknown.
    pub fn join(self, other: Value, max_candidates: usize) -> Value {
        if self == other {
            return self;
        }
        match (self, other) {
            (Value::Strings(mut a), Value::Strings(b)) => {
                for s in b {
                    if !a.contains(&s) {
                        a.push(s);
                    }
                }
                if a.len() <= max_candidates {
                    Value::Strings(a)
                } else {
                    Value::Unknown
                }
            }
            (Value::Targets(mut a), Value::Targets(b)) => {
                for t in b {
                    if !a.contains(&t) {
                        a.push(t);
                    }
                }
                if a.len() <= max_candidates {
                    Value::Targets(a)
                } else {
                    Value::Unknown
                }
            }
            _ => Value::Unknown,
        }
    }

    /// Property access on an abstract value.
    pub fn property(&self, key: &str) -> Value {
        match self {
            Value::Targets(targets) => {
                Value::Targets(targets.iter().map(|t| t.child(key)).collect())
            }
            Value::Object(map) => map.get(key).cloned().unwrap_or(Value::Unknown),
            Value::Array(items) => key
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i))
                .cloned()
                .unwrap_or(Value::Unknown),
            _ => Value::Unknown,
        }
    }
}

/// Formats a JS number the way `String(n)` would for integers.
fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
#[path = "../tests/js_values.rs"]
mod tests;
