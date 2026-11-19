use sha2::{Digest, Sha256};
fn main() {
    println!("cargo:rerun-if-changed=contracts/v1/openapi.json");
    println!("cargo:rerun-if-changed=generated");
    println!("cargo:rerun-if-changed=build/forge-pin.json");
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read("generated/provenance.json")
            .expect("mandatory Forge provenance missing; run build/generate.py"),
    )
    .unwrap();
    let hash = |p: &str| format!("{:x}", Sha256::digest(std::fs::read(p).unwrap()));
    assert_eq!(
        m["contractSha256"],
        hash("contracts/v1/openapi.json"),
        "contract changed; regenerate through pinned Forge"
    );
    let pin: serde_json::Value =
        serde_json::from_slice(&std::fs::read("build/forge-pin.json").unwrap()).unwrap();
    assert_eq!(m["generator"], pin, "generator pin drift");
    for (path, expected) in m["files"].as_object().unwrap() {
        assert_eq!(
            *expected,
            hash(&format!("generated/{path}")),
            "generated content changed"
        );
    }
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read("contracts/v1/openapi.json").unwrap()).unwrap();
    let mut ids = doc["paths"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v["post"]["operationId"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    ids.sort();
    let generated: Vec<String> =
        serde_json::from_slice(&std::fs::read("generated/operation-index.json").unwrap()).unwrap();
    assert_eq!(ids, generated, "Forge operation coverage drift");
}
