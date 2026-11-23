use crate::BackendRegistry;
use serde_json::{Value, json};
use std::sync::OnceLock;

pub const VERSION: &str = "1.0.0";
pub fn document() -> &'static Value {
    static DOC: OnceLock<Value> = OnceLock::new();
    DOC.get_or_init(|| {
        serde_json::from_str(include_str!("../contracts/v1/openapi.json"))
            .expect("embedded contract")
    })
}
pub fn operations() -> Vec<(&'static str, &'static Value)> {
    document()["paths"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(path, v)| (path.as_str(), &v["post"]))
        .collect()
}
pub fn operation(id: &str) -> Option<&'static Value> {
    operations()
        .into_iter()
        .find(|(_, op)| op["operationId"] == id)
        .map(|(_, op)| op)
}
pub fn resolve(schema: &Value) -> Value {
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        return resolve(
            document()
                .pointer(r.strip_prefix('#').expect("local reference"))
                .expect("resolved reference"),
        );
    }
    match schema {
        Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), resolve(v))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(resolve).collect()),
        other => other.clone(),
    }
}
pub fn input(id: &str) -> Option<Value> {
    operation(id).map(|op| resolve(&op["requestBody"]["content"]["application/json"]["schema"]))
}
pub fn valid(schema: &Value, value: &Value) -> bool {
    jsonschema::validator_for(schema).is_ok_and(|v| v.is_valid(value))
}
pub fn valid_input(id: &str, value: &Value) -> bool {
    input(id).is_some_and(|s| valid(&s, value))
}
/// First schema violation for a request, rendered as `/<path>: <reason>`
/// so error envelopes can name the offending field.
pub fn invalid_detail(id: &str, value: &Value) -> Option<String> {
    let schema = input(id)?;
    if let Some(e) = jsonschema::validator_for(&schema)
        .ok()?
        .iter_errors(value)
        .next()
    {
        let path = e.instance_path.as_str();
        // Validation errors render the rejected value; secret fields must
        // report their constraint without echoing the credential.
        let rendered = if matches!(path, "/password" | "/ownerPassword") {
            e.masked_with("redacted").to_string()
        } else {
            e.to_string()
        };
        return Some(if path.is_empty() {
            format!("/: {rendered}")
        } else {
            format!("{path}: {rendered}")
        });
    }
    // JSON Schema `maxLength` counts characters, but the PDF specification
    // binds passwords to 127 UTF-8 *bytes*: a multi-byte password under the
    // character limit can exceed it, and qpdf would truncate it silently.
    // A NUL byte is worse: the backend's C-string handling reads only the
    // prefix, so a `\0` open password silently encrypts to an empty one.
    for field in ["password", "ownerPassword"] {
        if let Some(v) = value.get(field).and_then(Value::as_str) {
            if v.contains('\0') {
                return Some(format!(
                    "/{field}: NUL bytes cannot be represented by the password transport"
                ));
            }
            if v.len() > 127 {
                return Some(format!("/{field}: exceeds the 127-byte PDF password bound"));
            }
        }
    }
    None
}
pub fn valid_output(id: &str, value: &Value) -> bool {
    operation(id).is_some_and(|op| {
        valid(
            &resolve(&op["responses"]["200"]["content"]["application/json"]["schema"]),
            value,
        )
    })
}
pub fn discovery() -> Value {
    discovery_with(&BackendRegistry::with_qpdf(Default::default()))
}
pub fn discovery_with(registry: &BackendRegistry) -> Value {
    let ops: Vec<Value> = operations().into_iter().map(|(path, op)| {
        let id = op["operationId"].as_str().unwrap();
        let engines = registry.engines(id);
        let status = if engines.is_empty() {
            "unavailable; no engine configured"
        } else if matches!(id, "extractText" | "renderPage") {
            "implemented; configured engine required"
        } else {
            "implemented; qpdf 12.x >=12.4.2 required; only 12.4.2 qualified; strict mode requires exact"
        };
        json!({
            "operationId": id, "description": op["summary"], "path":path,
            "status":status, "engines":engines, "inputSchema":input(id),
            "outputSchema":resolve(&op["responses"]["200"]["content"]["application/json"]["schema"]),
            "errorSchema":resolve(&document()["components"]["schemas"]["Error"])
        })
    }).collect();
    json!({"contractVersion":VERSION,"operations":ops,
        "limits":{"inputBytes":104857600,"totalInputBytes":262144000,"outputBytes":262144000,"pages":2000,"files":20,"wallSeconds":600,"concurrency":2},
        "unsupported":["reorderPages","fillForms","visualSigning","certificateSigning"],
        "excluded":["ocr","fullTextReplacement"]})
}
