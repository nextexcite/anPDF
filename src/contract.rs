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
pub fn valid_output(id: &str, value: &Value) -> bool {
    operation(id).is_some_and(|op| {
        valid(
            &resolve(&op["responses"]["200"]["content"]["application/json"]["schema"]),
            value,
        )
    })
}
pub fn discovery() -> Value {
    let ops: Vec<Value> = operations().into_iter().map(|(path, op)| json!({
        "operationId": op["operationId"], "description": op["summary"], "path":path,
        "status":"implemented; backend qualification required", "inputSchema":input(op["operationId"].as_str().unwrap()),
        "outputSchema":resolve(&op["responses"]["200"]["content"]["application/json"]["schema"]),
        "errorSchema":resolve(&document()["components"]["schemas"]["Error"])
    })).collect();
    json!({"contractVersion":VERSION,"operations":ops,
        "limits":{"inputBytes":104857600,"totalInputBytes":262144000,"outputBytes":262144000,"pages":2000,"files":20,"wallSeconds":60,"concurrency":2},
        "unsupported":["removePages","reorderPages","compress","fillForms","visualSigning","certificateSigning"],
        "excluded":["ocr","fullTextReplacement"]})
}
