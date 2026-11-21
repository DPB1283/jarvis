//! Tool framework: each tool is a plain definition (name, description, JSON schema, risk, summary, run).
//! Inputs are validated against the schema before anything runs (with eager input streaming the API
//! doesn't validate them for us).

pub mod install;
pub mod memory;
pub mod browser;
pub mod activity;
pub mod utilities;
pub mod screens;
pub mod clipboard;
pub mod messaging;
pub mod ps;
pub mod system;
pub mod timers;
pub mod web;

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use anyhow::Result;
use jarvis_core::events::Risk;
use serde_json::{Value, json};

use crate::Ctx;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What a tool returns: plain text, or content blocks (e.g. an image for a screenshot).
pub enum ToolOutput {
    Text(String),
    Blocks(Vec<Value>),
}

impl From<String> for ToolOutput {
    fn from(s: String) -> Self {
        ToolOutput::Text(s)
    }
}
impl From<&str> for ToolOutput {
    fn from(s: &str) -> Self {
        ToolOutput::Text(s.to_string())
    }
}

impl ToolOutput {
    pub fn to_content(&self) -> Value {
        match self {
            ToolOutput::Text(t) => json!(t),
            ToolOutput::Blocks(b) => json!(b),
        }
    }
    pub fn log_text(&self) -> String {
        match self {
            ToolOutput::Text(t) => t.clone(),
            ToolOutput::Blocks(_) => "[content]".into(),
        }
    }
}

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Value,
    pub risk: fn(&Ctx, &Value) -> Risk,
    pub summary: fn(&Value) -> String,
    pub run: fn(Ctx, Value) -> BoxFuture<Result<ToolOutput>>,
}

pub fn safe(_: &Ctx, _: &Value) -> Risk {
    Risk::Safe
}
pub fn unsafe_(_: &Ctx, _: &Value) -> Risk {
    Risk::Unsafe
}

/// All tools, in a fixed order (keeps the prompt cache stable).
pub fn all() -> &'static [ToolDef] {
    static ALL: OnceLock<Vec<ToolDef>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut v = Vec::new();
        v.extend(system::tools());
        v.extend(memory::tools());
        v.extend(timers::tools());
        v.extend(install::tools());
        v.extend(web::tools());
        v.extend(browser::tools());
        v.extend(messaging::tools());
        v.extend(screens::tools());
        v.extend(clipboard::tools());
        v.extend(activity::tools());
        v.extend(utilities::tools());
        v
    })
}

pub fn find(name: &str) -> Option<&'static ToolDef> {
    all().iter().find(|t| t.name == name)
}

/// Tool definitions for the API request.
pub fn api_defs() -> Vec<Value> {
    all()
        .iter()
        .map(|t| json!({ "name": t.name, "description": t.description, "input_schema": (t.schema)(), "eager_input_streaming": true }))
        .collect()
}

// --- Input helpers ---------------------------------------------------------------------------------

pub fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(|x| x.as_str()).filter(|x| !x.is_empty())
}
pub fn n(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(|x| x.as_f64())
}
pub fn b(v: &Value, k: &str) -> Option<bool> {
    v.get(k).and_then(|x| x.as_bool())
}

/// Minimal JSON-schema check for the shapes our tools use: object with properties/required;
/// property types string/number/integer/boolean/array/object, enum, minimum/maximum, minLength.
pub fn validate(schema: &Value, input: &Value) -> Result<(), String> {
    let Some(obj) = input.as_object() else { return Err("input must be an object".into()) };
    for req in schema["required"].as_array().into_iter().flatten().filter_map(|r| r.as_str()) {
        if obj.get(req).is_none_or(|v| v.is_null()) {
            return Err(format!("missing required field '{req}'"));
        }
    }
    let Some(props) = schema["properties"].as_object() else { return Ok(()) };
    for (k, v) in obj {
        let Some(p) = props.get(k) else { continue };
        if v.is_null() {
            continue;
        }
        let ty = p["type"].as_str().unwrap_or("");
        let ok = match ty {
            "string" => v.is_string(),
            "number" => v.is_number(),
            "integer" => v.as_f64().is_some_and(|f| f.fract() == 0.0),
            "boolean" => v.is_boolean(),
            "array" => v.is_array(),
            "object" => v.is_object(),
            _ => true,
        };
        if !ok {
            return Err(format!("'{k}' must be a {ty}"));
        }
        if let Some(allowed) = p["enum"].as_array() {
            if !allowed.contains(v) {
                return Err(format!("'{k}' must be one of {}", Value::Array(allowed.clone())));
            }
        }
        if let (Some(min), Some(x)) = (p["minimum"].as_f64(), v.as_f64()) {
            if x < min {
                return Err(format!("'{k}' must be ≥ {min}"));
            }
        }
        if let (Some(max), Some(x)) = (p["maximum"].as_f64(), v.as_f64()) {
            if x > max {
                return Err(format!("'{k}' must be ≤ {max}"));
            }
        }
        if let (Some(min), Some(sv)) = (p["minLength"].as_u64(), v.as_str()) {
            if (sv.chars().count() as u64) < min {
                return Err(format!("'{k}' is too short"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_inputs() {
        let schema = json!({"type":"object","properties":{"level":{"type":"number","minimum":0,"maximum":100},"mode":{"type":"string","enum":["a","b"]}},"required":["mode"]});
        assert!(validate(&schema, &json!({"mode":"a","level":50})).is_ok());
        assert!(validate(&schema, &json!({"level":50})).unwrap_err().contains("mode"));
        assert!(validate(&schema, &json!({"mode":"c"})).is_err());
        assert!(validate(&schema, &json!({"mode":"a","level":101})).is_err());
        assert!(validate(&schema, &json!({"mode":"a","level":"loud"})).is_err());
    }

    #[test]
    fn tool_names_are_unique_and_schemas_are_objects() {
        let mut seen = std::collections::HashSet::new();
        for t in all() {
            assert!(seen.insert(t.name), "duplicate tool {}", t.name);
            assert_eq!((t.schema)()["type"], "object", "{}", t.name);
        }
    }
}







