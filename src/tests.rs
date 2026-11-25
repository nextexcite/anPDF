use crate::{contract, error::Error, filesystem::Workspace};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

#[test]
fn rung_choice_prefers_gentlest_fit() {
    use crate::resample::choose_rung;
    // Measured per-rung document bytes, gentlest → hardest; all usable.
    let usable = [true; 4];
    // Gentle-most rung under target wins over smaller harder rungs.
    assert_eq!(
        choose_rung(
            &[9_000_000, 2_900_000, 1_800_000, 900_000],
            &usable,
            3_000_000
        ),
        1
    );
    // No rung fits: smallest measured rung is the best-effort pick.
    assert_eq!(
        choose_rung(&[9_000_000, 2_900_000, 1_800_000], &usable, 1_000),
        2
    );
    // Rungs left unmeasured by a budget stop are not eligible.
    assert_eq!(
        choose_rung(
            &[9_000_000, 1_800_000, 900_000],
            &[true, true, false],
            3_000_000
        ),
        1
    );
    // Exact-fit boundary counts as meeting the target.
    assert_eq!(choose_rung(&[3_000_000, 2_000_000], &usable, 3_000_000), 0);
}

mod content_operations {
    use crate::{
        BackendRegistry, Dispatcher,
        backend::{Backend, CompressMode, Control, ExtractTextBackend, RenderPageBackend},
        error::{Error, Result},
    };
    use serde_json::{Value, json};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{Arc, atomic::Ordering},
    };

    struct Inspect;
    impl Backend for Inspect {
        fn inspect(
            &self,
            path: &Path,
            _: Option<&str>,
            _: &Control,
            _: bool,
        ) -> Result<(Value, Vec<Value>)> {
            let encrypted = fs::read(path)? == b"encrypted";
            Ok((
                json!({
                    "pageCount": 2, "pdfVersion": "1.7", "structuralStatus": "valid",
                    "encrypted": encrypted, "forms": "present", "signatures": "present",
                    "signatureValidity": "notChecked", "actions": "absent",
                    "hasOutline": false, "hasAttachments": false, "metadata": {},
                    "pages": ([1, 2].map(|n| json!({"pageNumber": n, "rotation": 0,
                        "widthPoints": 100, "heightPoints": 100})))
                }),
                vec![],
            ))
        }

        fn transform(
            &self,
            _: &[PathBuf],
            _: Option<&[u64]>,
            _: &Path,
            _: &Control,
        ) -> Result<Vec<Value>> {
            unreachable!()
        }

        fn compress(&self, _: &Path, _: CompressMode, _: &Path, _: &Control) -> Result<Vec<Value>> {
            unreachable!()
        }
        fn encrypt(&self, _: &Path, _: &Path, _: &str, _: &str, _: &Control) -> Result<Vec<Value>> {
            unreachable!()
        }
        fn decrypt(&self, _: &Path, _: &Path, _: &str, _: &Control) -> Result<Vec<Value>> {
            unreachable!()
        }
    }

    struct Text(&'static str);
    impl ExtractTextBackend for Text {
        fn extract_text(
            &self,
            path: &Path,
            pages: &[u64],
            _: &Control,
        ) -> Result<(Vec<Value>, Vec<Value>)> {
            assert_eq!(path.file_name().unwrap(), "input-0.pdf");
            if self.0 == "fails" {
                return Err(Error("INTERNAL", None));
            }
            let pages = pages.iter().map(|page| {
                let right = if self.0 == "wrong-bounds" { 0 } else { 3 };
                let number = if self.0 == "wrong-page" { page + 1 } else { *page };
                let text = if self.0 == "too-big" { "x".repeat(crate::backend::MAX_TEXT_RESPONSE_BYTES) } else { self.0.to_owned() };
                json!({"page":number,"text":text,"spans":[{"text":self.0,"bounds":{"left":1,"right":right,"bottom":2,"top":4}}]})
            }).collect();
            Ok((pages, vec![]))
        }
    }

    #[test]
    fn text_selection_and_output_validation() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("doc.pdf"), b"original").unwrap();
        fs::write(root.path().join("encrypted.pdf"), b"encrypted").unwrap();
        let mut registry = BackendRegistry::with_structural(Arc::new(Inspect));
        for id in [
            "first",
            "second",
            "wrong-page",
            "wrong-bounds",
            "fails",
            "too-big",
        ] {
            registry.register_text(id, Arc::new(Text(id))).unwrap();
        }
        assert!(
            registry
                .register_text("second", Arc::new(Text("duplicate")))
                .is_err()
        );
        let dispatcher = Dispatcher::with_registry(root.path(), registry).unwrap();
        let discovery = dispatcher.discovery();
        let text = discovery["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|op| op["operationId"] == "extractText")
            .unwrap();
        assert_eq!(text["engines"][0], "first");
        assert_eq!(text["engines"][1], "second");
        let mut request = json!({"input":{"path":"doc.pdf"},"pages":[1,2.0]});
        let call = |request: &Value| {
            dispatcher.dispatch("extractText", request, false, &Control::default())
        };
        let result = call(&request).unwrap();
        assert_eq!(result["result"]["engine"], "first");
        assert_eq!(result["result"]["pages"][1]["page"], 2);
        assert_eq!(result["result"]["pages"][1]["text"], "first");
        request["engine"] = json!("second");
        assert_eq!(
            call(&request).unwrap()["result"]["pages"][0]["text"],
            "second"
        );
        for (engine, code) in [
            ("missing", "BACKEND_UNAVAILABLE"),
            ("fails", "INTERNAL"),
            ("wrong-page", "INTERNAL"),
            ("wrong-bounds", "INTERNAL"),
            ("too-big", "LIMIT_EXCEEDED"),
        ] {
            request["engine"] = json!(engine);
            request["pages"] = json!([1]);
            assert_eq!(call(&request).unwrap_err().0, code);
        }
        request["engine"] = json!("first");
        request["pages"] = json!([3]);
        assert_eq!(call(&request).unwrap_err().0, "INVALID_REQUEST");
        request["input"]["path"] = json!("encrypted.pdf");
        request["pages"] = json!([1]);
        assert_eq!(call(&request).unwrap_err().0, "ENCRYPTED_UNSUPPORTED");
        assert_eq!(fs::read(root.path().join("doc.pdf")).unwrap(), b"original");
    }

    const WHITE_PIXEL: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x02\0\0\0\x90wS\xde\0\0\0\x0cIDATx\x9cc\xf8\xff\xff?\0\x05\xfe\x02\xfe\r\xefF\xb8\0\0\0\0IEND\xaeB\x60\x82";

    struct Render(&'static str);
    impl RenderPageBackend for Render {
        fn render_page(
            &self,
            input: &Path,
            page: u64,
            dpi: u32,
            output: &Path,
            control: &Control,
        ) -> Result<Vec<Value>> {
            assert_eq!(page, 2);
            assert_eq!(dpi, 144);
            assert_eq!(input.parent(), output.parent());
            match self.0 {
                "invalid" => fs::write(output, b"not an image")?,
                "oversized" => {
                    let mut data = WHITE_PIXEL.to_vec();
                    data[16..20].copy_from_slice(&8193u32.to_be_bytes());
                    fs::write(output, data)?;
                }
                "cancelled" => {
                    fs::write(output, WHITE_PIXEL)?;
                    control.cancelled.store(true, Ordering::Relaxed);
                }
                _ => fs::write(output, WHITE_PIXEL)?,
            }
            Ok(vec![])
        }
    }

    #[test]
    fn render_checks_staging_and_publishes_only_validated_output() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("doc.pdf");
        let output = root.path().join("page.png");
        fs::write(&input, b"original").unwrap();
        let mut registry = BackendRegistry::with_structural(Arc::new(Inspect));
        for id in ["default", "invalid", "oversized", "cancelled"] {
            registry
                .register_renderer(id, Arc::new(Render(id)))
                .unwrap();
        }
        let dispatcher = Dispatcher::with_registry(root.path(), registry).unwrap();
        let mut request = json!({"input":{"path":"doc.pdf"},"page":2,"output":{"path":"page.png"}});
        let run = |request: &Value, dry| {
            dispatcher.dispatch("renderPage", request, dry, &Control::default())
        };
        assert_eq!(run(&request, true).unwrap()["dryRun"], true);
        assert!(!output.exists());
        let result = run(&request, false).unwrap();
        assert_eq!(result["result"]["engine"], "default");
        assert_eq!(result["result"]["width"], 1);
        assert_eq!(result["result"]["height"], 1);
        assert_eq!(result["result"]["bytes"], WHITE_PIXEL.len());
        assert_eq!(result["result"]["mediaType"], "image/png");
        assert_eq!(fs::read(&output).unwrap(), WHITE_PIXEL);
        assert_eq!(run(&request, false).unwrap_err().0, "OUTPUT_EXISTS");
        fs::hard_link(&input, root.path().join("alias.png")).unwrap();
        request["output"] = json!({"path":"alias.png","replaceExisting":true});
        assert_eq!(run(&request, false).unwrap_err().0, "ACCESS_DENIED");

        fs::write(&output, b"keep me").unwrap();
        request["output"] = json!({"path":"page.png","replaceExisting":true});
        for (engine, error) in [
            ("invalid", "INTERNAL"),
            ("oversized", "LIMIT_EXCEEDED"),
            ("cancelled", "TIMEOUT"),
        ] {
            request["engine"] = json!(engine);
            assert_eq!(run(&request, false).unwrap_err().0, error);
            assert_eq!(fs::read(&output).unwrap(), b"keep me");
        }
        assert_eq!(fs::read(&input).unwrap(), b"original");
    }
}

fn preservation_report() -> serde_json::Value {
    json!({"sha256":"0".repeat(64),"bytes":1,"pageCount":1,"pdfVersion":"1.7",
        "structuralStatus":"valid","encrypted":false,"forms":"absent","signatures":"absent",
        "signatureValidity":"notChecked","actions":"absent","hasOutline":false,"hasAttachments":false,
        "metadata":{},"pages":[{"pageNumber":1,"rotation":0,"annotationCount":0}]})
}

struct PreservationBackend {
    input: serde_json::Value,
    output: serde_json::Value,
    bytes: Vec<u8>,
}
impl crate::backend::Backend for PreservationBackend {
    fn inspect(
        &self,
        path: &std::path::Path,
        _: Option<&str>,
        _: &crate::backend::Control,
        _: bool,
    ) -> crate::error::Result<(serde_json::Value, Vec<serde_json::Value>)> {
        let input = path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("input-");
        Ok((
            if input {
                self.input.clone()
            } else {
                self.output.clone()
            },
            vec![json!({"code":"BACKEND_VERSION_UNTESTED","message":"Test backend version."})],
        ))
    }
    fn transform(
        &self,
        _: &[std::path::PathBuf],
        _: Option<&[u64]>,
        out: &std::path::Path,
        _: &crate::backend::Control,
    ) -> crate::error::Result<Vec<serde_json::Value>> {
        fs::write(out, &self.bytes)?;
        Ok(vec![])
    }
    fn compress(
        &self,
        _: &std::path::Path,
        _: crate::backend::CompressMode,
        out: &std::path::Path,
        _: &crate::backend::Control,
    ) -> crate::error::Result<Vec<serde_json::Value>> {
        fs::write(out, &self.bytes)?;
        Ok(vec![])
    }
    fn encrypt(
        &self,
        _: &std::path::Path,
        _: &std::path::Path,
        _: &str,
        _: &str,
        _: &crate::backend::Control,
    ) -> crate::error::Result<Vec<serde_json::Value>> {
        unreachable!()
    }
    fn decrypt(
        &self,
        _: &std::path::Path,
        _: &std::path::Path,
        _: &str,
        _: &crate::backend::Control,
    ) -> crate::error::Result<Vec<serde_json::Value>> {
        unreachable!()
    }
}

#[test]
fn preservation_statuses_and_required_failure_do_not_publish() {
    use crate::{Dispatcher, backend::Control, preservation_status};
    let empty = preservation_report();
    assert_eq!(
        preservation_status(std::slice::from_ref(&empty), &empty, None),
        json!({"metadata":"absent","bookmarks":"absent","attachments":"absent","annotations":"absent"})
    );
    let mut rich = empty.clone();
    rich["metadata"] = json!({"title":"Synthetic"});
    rich["hasOutline"] = json!(true);
    rich["hasAttachments"] = json!(true);
    rich["pages"][0]["annotationCount"] = json!(2);
    rich["preservationEvidence"] =
        json!({"metadata":"a".repeat(64),"attachments":"b".repeat(64),"bookmarks":"[]"});
    let preserved = preservation_status(std::slice::from_ref(&rich), &rich, None);
    for aspect in ["metadata", "bookmarks", "attachments", "annotations"] {
        assert_eq!(preserved[aspect], "preserved");
    }
    let merged = preservation_status(&[rich.clone(), rich.clone()], &rich, None);
    assert_eq!(merged["metadata"], "preserved");
    for aspect in ["bookmarks", "attachments", "annotations"] {
        assert_eq!(merged[aspect], "omitted");
    }
    let mut unknown = rich.clone();
    unknown["pages"][0]
        .as_object_mut()
        .unwrap()
        .remove("annotationCount");
    assert_eq!(
        preservation_status(&[unknown], &rich, None)["annotations"],
        "omitted"
    );
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("in.pdf"), b"original").unwrap();
    fs::write(t.path().join("existing.pdf"), b"existing").unwrap();
    let d = Dispatcher::with_backend(
        t.path(),
        std::sync::Arc::new(PreservationBackend {
            input: rich,
            output: empty,
            bytes: b"transformed".to_vec(),
        }),
    )
    .unwrap();
    let best = d
        .dispatch(
            "extractPdf",
            &json!({"input":{"path":"in.pdf"},"pages":[1],"output":{"path":"best.pdf"}}),
            false,
            &Control::default(),
        )
        .unwrap();
    assert_eq!(best["result"]["preservation"]["metadata"], "omitted");
    assert_eq!(
        best["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|w| w["code"] == "PRESERVATION_OMITTED")
            .count(),
        1
    );
    assert_eq!(
        best["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|w| w["code"] == "BACKEND_VERSION_UNTESTED")
            .count(),
        1
    );
    for operation in ["extractPdf", "reorderPages"] {
        for aspect in ["metadata", "bookmarks", "attachments", "annotations"] {
            let request = json!({"input":{"path":"in.pdf"},"pages":[1],"output":{"path":"existing.pdf","replaceExisting":true},"preserve":{aspect:true}});
            assert_eq!(
                d.dispatch(operation, &request, false, &Control::default())
                    .unwrap_err()
                    .0,
                "PRESERVATION_UNMET"
            );
            assert_eq!(
                fs::read(t.path().join("existing.pdf")).unwrap(),
                b"existing"
            );
        }
    }
    let error = Error("PRESERVATION_UNMET", None);
    assert_eq!((error.http(), error.exit()), (422, 5));
    assert_eq!(error.envelope("extractPdf")["error"]["retryable"], false);
    assert!(!contract::valid_input(
        "extractPdf",
        &json!({"input":{"path":"in.pdf"},"pages":[1],"output":{"path":"out.pdf"},"preserve":{"unknown":true}})
    ));
}

#[test]
fn compression_fallback_and_opt_out_describe_published_bytes() {
    use crate::{Dispatcher, backend::Control};
    for (input, compressed, allow, expected) in [
        (
            b"input".as_slice(),
            b"larger output".as_slice(),
            false,
            "original",
        ),
        (b"input", b"larger output", true, "compressed"),
        (b"larger input", b"small", false, "compressed"),
        (b"input", b"equal", false, "original"),
        (b"", b"", false, "original"),
    ] {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("in.pdf"), input).unwrap();
        let d = Dispatcher::with_backend(
            t.path(),
            std::sync::Arc::new(PreservationBackend {
                input: preservation_report(),
                output: preservation_report(),
                bytes: compressed.to_vec(),
            }),
        )
        .unwrap();
        let response = d.dispatch("compressPdf", &json!({"input":{"path":"in.pdf"},"output":{"path":"out.pdf"},"allowLargerOutput":allow}), false, &Control::default()).unwrap();
        let result = &response["result"];
        let written = fs::read(t.path().join("out.pdf")).unwrap();
        assert_eq!(
            written,
            if expected == "original" {
                input
            } else {
                compressed
            }
        );
        assert_eq!(result["outputSource"], expected);
        assert_eq!(result["bytes"], written.len());
        assert_eq!(
            result["sha256"],
            crate::filesystem::hash(&mut fs::File::open(t.path().join("out.pdf")).unwrap())
                .unwrap()
        );
        if expected == "original" {
            assert_eq!(result["savedBytes"], 0);
            assert_eq!(result["compressionRatio"], 1.0);
        }
    }
}

#[test]
#[cfg(feature = "lossless-only")]
fn compression_policy_rejects_explicit_and_inferred_lossy_modes() {
    use crate::{Dispatcher, backend::Control};
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("in.pdf"), b"original bytes").unwrap();
    fs::write(root.path().join("out.pdf"), b"existing destination").unwrap();
    let dispatcher = Dispatcher::with_backend(
        root.path(),
        std::sync::Arc::new(PreservationBackend {
            input: preservation_report(),
            output: preservation_report(),
            bytes: b"lossless".to_vec(),
        }),
    )
    .unwrap();
    for options in [
        json!({"mode":"lossy"}),
        json!({"mode":"layered"}),
        json!({"maxOutputBytes":1}),
        json!({"compressionLevel":2}),
    ] {
        let mut request = options;
        request["input"] = json!({"path":"in.pdf"});
        request["output"] = json!({"path":"out.pdf","replaceExisting":true});
        let error = dispatcher
            .dispatch("compressPdf", &request, false, &Control::default())
            .unwrap_err();
        assert_eq!(error.0, "INVALID_REQUEST");
        assert!(error.message().contains("lossless compression only"));
        assert_eq!(
            fs::read(root.path().join("out.pdf")).unwrap(),
            b"existing destination"
        );
    }
}

#[test]
#[cfg(not(feature = "lossless-only"))]
fn layered_timeout_keeps_lossless_output_unless_strict_or_cancelled() {
    use crate::{
        BackendRegistry, Dispatcher,
        backend::{Control, PageRasterizer},
        error::Result,
    };
    use serde_json::Value;
    use std::{
        path::Path,
        sync::{Arc, atomic::Ordering},
    };
    struct Deadline(bool);
    impl PageRasterizer for Deadline {
        fn layered_document(&self, _: &Path, _: &Path, control: &Control) -> Result<Value> {
            if self.0 {
                control.cancelled.store(true, Ordering::Relaxed);
            }
            Err(Error("TIMEOUT", None))
        }
        fn rasterize_document(
            &self,
            _: &Path,
            _: &Path,
            _: &[(u32, u8)],
            _: Option<u64>,
            _: &Control,
        ) -> Result<Value> {
            unreachable!()
        }
    }
    for (cancelled, strict, expected) in [
        (false, false, None),
        (false, true, Some("SIZE_TARGET_UNMET")),
        (true, false, Some("TIMEOUT")),
    ] {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("in.pdf"), b"original bytes").unwrap();
        let mut registry = BackendRegistry::with_structural(Arc::new(PreservationBackend {
            input: preservation_report(),
            output: preservation_report(),
            bytes: b"lossless".to_vec(),
        }));
        registry.register_rasterizer(Arc::new(Deadline(cancelled)));
        let d = Dispatcher::with_registry(t.path(), registry).unwrap();
        let request = json!({"mode":"layered", "input":{"path":"in.pdf"}, "output":{"path":"out.pdf"}, "maxOutputBytes":1, "onTargetMiss":if strict { "error" } else { "bestEffort" }});
        let result = d.dispatch("compressPdf", &request, false, &Control::default());
        let out = t.path().join("out.pdf");
        if let Some(code) = expected {
            assert_eq!(result.unwrap_err().0, code);
            assert!(!out.exists());
        } else {
            let result = result.unwrap();
            assert_eq!(fs::read(out).unwrap(), b"lossless");
            assert_eq!(result["result"]["targetMet"], false);
            assert!(
                result["warnings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|w| w["code"] == "PAGE_RASTERIZATION_SKIPPED"
                        && w["message"].as_str().unwrap().contains("deadline"))
            );
        }
    }
}

#[test]
fn compression_invalid_output_never_falls_back_or_publishes() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("in.pdf"), b"original").unwrap();
    let mut output = preservation_report();
    output["pageCount"] = json!(2);
    let d = crate::Dispatcher::with_backend(
        t.path(),
        std::sync::Arc::new(PreservationBackend {
            input: preservation_report(),
            output,
            bytes: b"larger compressed bytes".to_vec(),
        }),
    )
    .unwrap();
    assert_eq!(
        d.dispatch(
            "compressPdf",
            &json!({"input":{"path":"in.pdf"},"output":{"path":"out.pdf"}}),
            false,
            &crate::backend::Control::default()
        )
        .unwrap_err()
        .0,
        "PRESERVATION_UNKNOWN"
    );
    assert!(!t.path().join("out.pdf").exists());
}
#[test]
fn schema_rejects_unknown_properties_and_bad_pages() {
    assert!(contract::valid_input(
        "inspectPdf",
        &json!({"input":{"path":"a.pdf"}})
    ));
    assert!(!contract::valid_input(
        "inspectPdf",
        &json!({"input":{"path":"a.pdf","url":"x"}})
    ));
    assert!(!contract::valid_input(
        "extractPdf",
        &json!({"input":{"path":"a"},"output":{"path":"b"},"pages":[1,1]})
    ));
    assert!(!contract::valid_input(
        "mergePdf",
        &json!({"inputs":[],"output":{"path":"b"}})
    ));
    assert!(contract::valid_input(
        "splitPdf",
        &json!({"input":{"path":"a.pdf"},"parts":[{"pages":[1],"output":{"path":"p1.pdf"}},{"pages":[2],"output":{"path":"p2.pdf"}}]})
    ));
    assert!(!contract::valid_input(
        "splitPdf",
        &json!({"input":{"path":"a.pdf"},"parts":[{"pages":[1],"output":{"path":"p1.pdf"}}]})
    ));
    assert!(!contract::valid_input(
        "splitPdf",
        &json!({"input":{"path":"a.pdf"},"parts":[{"pages":[1],"output":{"path":"p1.pdf","extra":1}},{"pages":[2],"output":{"path":"p2.pdf"}}]})
    ));
    assert!(!contract::valid_output(
        "inspectPdf",
        &json!({"result":{"pageCount":-1}})
    ));
    assert!(!contract::valid_output(
        "splitPdf",
        &json!({"contractVersion":"1.0.0","operationId":"splitPdf","result":{"outputs":[{"path":"p1.pdf","sha256":"0".repeat(64),"bytes":1,"pageCount":1}]},"warnings":[]})
    ));
    assert!(contract::valid_input(
        "removePages",
        &json!({"input":{"path":"a.pdf"},"pages":[2],"output":{"path":"out.pdf"}})
    ));
    assert!(!contract::valid_input(
        "removePages",
        &json!({"input":{"path":"a.pdf"},"pages":[],"output":{"path":"out.pdf"}})
    ));
    assert!(contract::valid_input(
        "compressPdf",
        &json!({"input":{"path":"a.pdf"},"output":{"path":"out.pdf"}})
    ));
    assert!(contract::valid_input(
        "compressPdf",
        &json!({"input":{"path":"a.pdf"},"output":{"path":"out.pdf"},"mode":"lossy","jpegQuality":85})
    ));
    assert!(!contract::valid_input(
        "compressPdf",
        &json!({"input":{"path":"a.pdf"},"output":{"path":"out.pdf"},"mode":"lossy","jpegQuality":101})
    ));
    for code in
        contract::document()["components"]["schemas"]["Error"]["properties"]["error"]["properties"]
            ["code"]["enum"]
            .as_array()
            .unwrap()
    {
        let error = Error(
            Box::leak(code.as_str().unwrap().to_string().into_boxed_str()),
            None,
        );
        assert!(contract::valid(
            &contract::resolve(&contract::document()["components"]["schemas"]["Error"]),
            &error.envelope("unknown")
        ));
    }
}
#[test]
fn confinement_aliases_and_exclusive_commit() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("a"), b"original").unwrap();
    let w = Workspace::new(tmp.path()).unwrap();
    for path in [
        "../a",
        "/etc/passwd",
        "a/../a",
        "https://example.invalid/a",
        "a\\b",
        "./a",
    ] {
        assert!(w.input(path).is_err());
    }
    symlink("a", tmp.path().join("sym")).unwrap();
    assert!(w.input("sym").is_err());
    fs::create_dir(tmp.path().join("writable")).unwrap();
    fs::set_permissions(
        tmp.path().join("writable"),
        fs::Permissions::from_mode(0o777),
    )
    .unwrap();
    assert!(w.input("writable/a").is_err());
    fs::hard_link(tmp.path().join("a"), tmp.path().join("alias")).unwrap();
    let mut inputs = vec![w.input("a").unwrap()];
    assert!(w.destination("alias", true, &inputs).is_err());
    let dest = w.destination("output", false, &inputs).unwrap();
    let source = tmp.path().join("source");
    fs::write(&source, b"result").unwrap();
    dest.commit(&source, &inputs).unwrap();
    assert!(dest.commit(&source, &inputs).is_err());
    assert_eq!(fs::read(tmp.path().join("a")).unwrap(), b"original");
    fs::write(tmp.path().join("a"), b"changed").unwrap();
    assert!(w.unchanged(&mut inputs).is_err());
    assert_eq!(fs::read(tmp.path().join("output")).unwrap(), b"result");
}
#[test]
fn backend_output_is_validated() {
    struct Broken;
    impl crate::backend::Backend for Broken {
        fn inspect(
            &self,
            _: &std::path::Path,
            _: Option<&str>,
            _: &crate::backend::Control,
            _: bool,
        ) -> crate::error::Result<(serde_json::Value, Vec<serde_json::Value>)> {
            Ok((json!({"pageCount":1}), vec![]))
        }
        fn transform(
            &self,
            _: &[std::path::PathBuf],
            _: Option<&[u64]>,
            _: &std::path::Path,
            _: &crate::backend::Control,
        ) -> crate::error::Result<Vec<serde_json::Value>> {
            unreachable!()
        }
        fn compress(
            &self,
            _: &std::path::Path,
            _: crate::backend::CompressMode,
            _: &std::path::Path,
            _: &crate::backend::Control,
        ) -> crate::error::Result<Vec<serde_json::Value>> {
            unreachable!()
        }
        fn encrypt(
            &self,
            _: &std::path::Path,
            _: &std::path::Path,
            _: &str,
            _: &str,
            _: &crate::backend::Control,
        ) -> crate::error::Result<Vec<serde_json::Value>> {
            unreachable!()
        }
        fn decrypt(
            &self,
            _: &std::path::Path,
            _: &std::path::Path,
            _: &str,
            _: &crate::backend::Control,
        ) -> crate::error::Result<Vec<serde_json::Value>> {
            unreachable!()
        }
    }
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("a"), b"pdf").unwrap();
    let d = crate::Dispatcher::with_backend(t.path(), std::sync::Arc::new(Broken)).unwrap();
    assert_eq!(
        d.dispatch(
            "inspectPdf",
            &json!({"input":{"path":"a"}}),
            false,
            &crate::backend::Control::default()
        )
        .unwrap_err()
        .0,
        "INTERNAL"
    );
}

#[test]
fn process_timeout_and_output_limits() {
    use crate::backend::{Backend, Control, Qpdf};
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    let exe = root.path().join("backend");
    fs::write(&exe, "#!/usr/bin/python3\nimport time\ntime.sleep(10)\n").unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
    let q = Qpdf {
        executable: exe.clone(),
        exact: false,
        as_limit: 768 * 1024 * 1024,
        cpu_scale: 1,
    };
    let c = Control {
        deadline: Instant::now() + Duration::from_millis(40),
        ..Control::default()
    };
    let start = Instant::now();
    assert_eq!(q.inspect(&exe, None, &c, true).unwrap_err().0, "TIMEOUT");
    assert!(start.elapsed() < Duration::from_secs(2));
    fs::write(
        &exe,
        "#!/usr/bin/python3\nimport sys\nsys.stdout.buffer.write(b'x'*(40*1024*1024))\n",
    )
    .unwrap();
    assert_eq!(
        q.inspect(&exe, None, &Control::default(), true)
            .unwrap_err()
            .0,
        "LIMIT_EXCEEDED"
    );
    fs::write(
        &exe,
        "#!/usr/bin/python3\nimport sys\nsys.stderr.buffer.write(b'e'*100000)\n",
    )
    .unwrap();
    assert_eq!(
        q.inspect(&exe, None, &Control::default(), true)
            .unwrap_err()
            .0,
        "LIMIT_EXCEEDED"
    );
}
#[test]
fn atomic_competing_outputs_and_sparse_limit() {
    use std::sync::Arc;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fs::write(&source, b"complete result").unwrap();
    let w = Arc::new(Workspace::new(root.path()).unwrap());
    let mut workers = Vec::new();
    for _ in 0..2 {
        let w = w.clone();
        let source = source.clone();
        workers.push(std::thread::spawn(move || {
            w.destination("out", false, &[])
                .and_then(|d| d.commit(&source, &[]))
        }));
    }
    let results = workers
        .into_iter()
        .map(|w| w.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        fs::read(root.path().join("out")).unwrap(),
        b"complete result"
    );
    let f = fs::File::create(root.path().join("large")).unwrap();
    f.set_len(crate::filesystem::FILE_LIMIT + 1).unwrap();
    assert_eq!(w.input("large").err().unwrap().0, "LIMIT_EXCEEDED");
    assert!(!fs::read_dir(root.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".anpdf-")
    }));
}

#[test]
fn split_destination_collision_and_rollback() {
    struct MockSplit;
    impl crate::backend::Backend for MockSplit {
        fn inspect(
            &self,
            path: &std::path::Path,
            _: Option<&str>,
            _: &crate::backend::Control,
            _: bool,
        ) -> crate::error::Result<(serde_json::Value, Vec<serde_json::Value>)> {
            let pages = if path.file_name().and_then(|s| s.to_str()) == Some("input-0.pdf") {
                3
            } else if let Ok(written) = fs::read_to_string(path)
                && let Some(list) = written
                    .strip_prefix("pages:Some([")
                    .and_then(|v| v.strip_suffix("])"))
                && !list.is_empty()
            {
                list.split(',').count() as u64
            } else {
                1
            };
            let page_infos = (1..=pages)
                .map(|n| json!({"pageNumber": n, "rotation": 0, "widthPoints": 300.0, "heightPoints": 200.0}))
                .collect::<Vec<_>>();
            Ok((
                json!({
                    "sha256": "0".repeat(64),
                    "bytes": 1,
                    "pageCount": pages,
                    "pdfVersion": "1.7",
                    "structuralStatus": "valid",
                    "encrypted": false,
                    "forms": "absent",
                    "signatures": "absent",
                    "signatureValidity": "notChecked",
                    "actions": "absent",
                    "hasOutline": false,
                    "hasAttachments": false,
                    "metadata": {},
                    "pages": page_infos
                }),
                vec![],
            ))
        }
        fn transform(
            &self,
            _: &[std::path::PathBuf],
            pages: Option<&[u64]>,
            out: &std::path::Path,
            _: &crate::backend::Control,
        ) -> crate::error::Result<Vec<serde_json::Value>> {
            fs::write(out, format!("pages:{pages:?}")).unwrap();
            Ok(vec![])
        }
        fn compress(
            &self,
            _: &std::path::Path,
            _: crate::backend::CompressMode,
            _: &std::path::Path,
            _: &crate::backend::Control,
        ) -> crate::error::Result<Vec<serde_json::Value>> {
            unreachable!()
        }
        fn encrypt(
            &self,
            _: &std::path::Path,
            _: &std::path::Path,
            _: &str,
            _: &str,
            _: &crate::backend::Control,
        ) -> crate::error::Result<Vec<serde_json::Value>> {
            unreachable!()
        }
        fn decrypt(
            &self,
            _: &std::path::Path,
            _: &std::path::Path,
            _: &str,
            _: &crate::backend::Control,
        ) -> crate::error::Result<Vec<serde_json::Value>> {
            unreachable!()
        }
    }

    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("in.pdf"), b"pdf").unwrap();
    fs::write(t.path().join("existing-a.pdf"), b"a").unwrap();
    fs::hard_link(
        t.path().join("existing-a.pdf"),
        t.path().join("existing-b.pdf"),
    )
    .unwrap();
    let d = crate::Dispatcher::with_backend(t.path(), std::sync::Arc::new(MockSplit)).unwrap();

    // Same destination path in two parts is rejected even on dry-run.
    assert_eq!(
        d.dispatch(
            "splitPdf",
            &json!({
                "input": {"path": "in.pdf"},
                "parts": [
                    {"pages": [1], "output": {"path": "out.pdf"}},
                    {"pages": [2], "output": {"path": "out.pdf"}}
                ]
            }),
            true,
            &crate::backend::Control::default()
        )
        .unwrap_err()
        .0,
        "INVALID_REQUEST"
    );

    // Distinct paths that are hardlinked to the same target file are rejected.
    assert_eq!(
        d.dispatch(
            "splitPdf",
            &json!({
                "input": {"path": "in.pdf"},
                "parts": [
                    {"pages": [1], "output": {"path": "existing-a.pdf", "replaceExisting": true}},
                    {"pages": [2], "output": {"path": "existing-b.pdf", "replaceExisting": true}}
                ]
            }),
            true,
            &crate::backend::Control::default()
        )
        .unwrap_err()
        .0,
        "INVALID_REQUEST"
    );

    // Multi-output commit succeeds and preserves input.
    let ok = d
        .dispatch(
            "splitPdf",
            &json!({
                "input": {"path": "in.pdf"},
                "parts": [
                    {"pages": [1], "output": {"path": "p1.pdf"}},
                    {"pages": [2], "output": {"path": "p2.pdf"}}
                ]
            }),
            false,
            &crate::backend::Control::default(),
        )
        .unwrap();
    assert_eq!(ok["result"]["outputs"].as_array().unwrap().len(), 2);
    assert!(t.path().join("p1.pdf").exists());
    assert!(t.path().join("p2.pdf").exists());

    // Rollback unlinks newly created files but keeps existing replaceable ones.
    let w = Workspace::new(t.path()).unwrap();
    let src = t.path().join("src.pdf");
    fs::write(&src, b"new").unwrap();
    let new_dest = w.destination("rollback-new.pdf", false, &[]).unwrap();
    new_dest.commit(&src, &[]).unwrap();
    assert!(t.path().join("rollback-new.pdf").exists());
    new_dest.rollback_new();
    assert!(!t.path().join("rollback-new.pdf").exists());

    // reorderPages passes its order through without sorting, requires a
    // complete cover after document inspection, and treats dry-run as the
    // documented preflight boundary.
    let reordered = d
        .dispatch(
            "reorderPages",
            &json!({"input": {"path": "in.pdf"}, "pages": [3, 1, 2], "output": {"path": "reordered.pdf"}}),
            false,
            &crate::backend::Control::default(),
        )
        .unwrap();
    assert_eq!(reordered["result"]["pageCount"], 3);
    assert_eq!(
        fs::read(t.path().join("reordered.pdf")).unwrap(),
        b"pages:Some([3, 1, 2])"
    );
    let identity = d
        .dispatch(
            "reorderPages",
            &json!({"input": {"path": "in.pdf"}, "pages": [1, 2, 3], "output": {"path": "identity.pdf"}}),
            false,
            &crate::backend::Control::default(),
        )
        .unwrap();
    assert_eq!(identity["result"]["pageCount"], 3);
    assert!(t.path().join("identity.pdf").exists());
    assert!(
        d.dispatch(
            "reorderPages",
            &json!({"input": {"path": "in.pdf"}, "pages": [1, 2], "output": {"path": "dry-reorder.pdf"}}),
            true,
            &crate::backend::Control::default()
        )
        .unwrap()["dryRun"]
            == true
    );
    assert!(!t.path().join("dry-reorder.pdf").exists());
    for pages in [
        serde_json::json!([1, 2]),
        serde_json::json!([3, 1]),
        serde_json::json!([1, 4, 3]),
        serde_json::json!([0, 1, 2]),
        serde_json::json!([1, 1, 3]),
    ] {
        assert_eq!(
            d.dispatch(
                "reorderPages",
                &json!({"input": {"path": "in.pdf"}, "pages": pages, "output": {"path": "bad-reorder.pdf"}}),
                false,
                &crate::backend::Control::default()
            )
            .unwrap_err()
            .0,
            "INVALID_REQUEST"
        );
        assert!(!t.path().join("bad-reorder.pdf").exists());
    }
    assert!(crate::contract::valid_input(
        "reorderPages",
        &json!({"input":{"path":"in.pdf"},"pages":[3,1,2],"output":{"path":"out.pdf"}})
    ));
    assert!(!crate::contract::valid_input(
        "reorderPages",
        &json!({"input":{"path":"in.pdf"},"pages":[1,1,3],"output":{"path":"out.pdf"}})
    ));
    assert!(!crate::contract::valid_input(
        "reorderPages",
        &json!({"input":{"path":"in.pdf"},"pages":[0,1,2],"output":{"path":"out.pdf"}})
    ));

    // removePages rejects removing every page of the input.
    assert_eq!(
        d.dispatch(
            "removePages",
            &json!({"input": {"path": "in.pdf"}, "pages": [1, 2, 3], "output": {"path": "none.pdf"}}),
            false,
            &crate::backend::Control::default()
        )
        .unwrap_err()
        .0,
        "INVALID_REQUEST"
    );

    // compressPdf rejects jpegQuality in lossless mode.
    assert_eq!(
        d.dispatch(
            "compressPdf",
            &json!({"input": {"path": "in.pdf"}, "output": {"path": "c.pdf"}, "mode": "lossless", "jpegQuality": 80}),
            true,
            &crate::backend::Control::default()
        )
        .unwrap_err()
        .0,
        "INVALID_REQUEST"
    );
}

#[test]
fn protect_dispatch_paths() {
    use crate::backend::{Backend, Control};
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;
    struct Protect {
        secrets: Mutex<Vec<(String, String)>>,
        seen_passwords: Mutex<Vec<Option<String>>>,
    }
    impl Protect {
        // Inputs are staged under generated names, so marker bytes identify
        // the document: "locked-marker"/"sealed-output" read as encrypted.
        fn report(&self, path: &Path) -> (Value, bool) {
            let data = fs::read(path).unwrap_or_default();
            let sealed = data == b"sealed-output" || data == b"locked-marker";
            let mut r = json!({
                "sha256":"0".repeat(64),"bytes":1,"pageCount":1,
                "pdfVersion":"1.7","structuralStatus":"valid",
                "encrypted":sealed,"forms":"absent","signatures":"absent",
                "signatureValidity":"notChecked","actions":"absent",
                "hasOutline":false,"hasAttachments":false,
                "hasMetadata":false,"metadata":{},
                "pages":[{"pageNumber":1,"rotation":0,"annotationCount":0}]
            });
            if data == b"signed-marker" {
                r["signatures"] = "present".into();
            }
            (r, sealed)
        }
    }
    impl Backend for Protect {
        fn inspect(
            &self,
            path: &Path,
            password: Option<&str>,
            _: &Control,
            _: bool,
        ) -> crate::error::Result<(Value, Vec<Value>)> {
            self.seen_passwords
                .lock()
                .unwrap()
                .push(password.map(str::to_string));
            let (report, sealed) = self.report(path);
            if sealed && password.is_none() {
                return Err(crate::error::Error("ENCRYPTED_UNSUPPORTED", None));
            }
            Ok((report, vec![]))
        }
        fn transform(
            &self,
            _: &[PathBuf],
            _: Option<&[u64]>,
            _: &Path,
            _: &Control,
        ) -> crate::error::Result<Vec<Value>> {
            unreachable!()
        }
        fn compress(
            &self,
            _: &Path,
            _: crate::backend::CompressMode,
            _: &Path,
            _: &Control,
        ) -> crate::error::Result<Vec<Value>> {
            unreachable!()
        }
        fn encrypt(
            &self,
            _: &Path,
            out: &Path,
            user: &str,
            owner: &str,
            _: &Control,
        ) -> crate::error::Result<Vec<Value>> {
            self.secrets
                .lock()
                .unwrap()
                .push((user.to_string(), owner.to_string()));
            fs::write(out, b"sealed-output").unwrap();
            Ok(vec![])
        }
        fn decrypt(
            &self,
            _: &Path,
            out: &Path,
            password: &str,
            _: &Control,
        ) -> crate::error::Result<Vec<Value>> {
            if password != "synthetic-user-secret" {
                return Err(crate::error::Error("INVALID_PASSWORD", None));
            }
            fs::write(out, b"opened-output").unwrap();
            Ok(vec![])
        }
    }
    let root = tempdir().unwrap();
    fs::write(root.path().join("plain.pdf"), b"plain-marker").unwrap();
    fs::write(root.path().join("locked.pdf"), b"locked-marker").unwrap();
    fs::write(root.path().join("signed.pdf"), b"signed-marker").unwrap();
    let backend = Arc::new(Protect {
        secrets: Mutex::new(vec![]),
        seen_passwords: Mutex::new(vec![]),
    });
    let d = crate::Dispatcher::with_backend(root.path(), backend.clone()).unwrap();
    let c = Control::default();
    let encrypt = |extra: Value| {
        let mut r = json!({"input":{"path":"plain.pdf"},"password":"synthetic-user-secret","output":{"path":"sealed.pdf","replaceExisting":true}});
        for (k, v) in extra.as_object().unwrap() {
            r[k] = v.clone();
        }
        r
    };
    // encryptPdf: user password rides to the backend; an omitted owner is
    // generated, never blank.
    let out = d
        .dispatch("encryptPdf", &encrypt(json!({})), false, &c)
        .unwrap();
    assert_eq!(out["result"]["path"], "sealed.pdf");
    let secrets = backend.secrets.lock().unwrap();
    assert_eq!(secrets[0].0, "synthetic-user-secret");
    assert!(!secrets[0].1.is_empty() && secrets[0].1 != secrets[0].0);
    // The post-write re-inspect opens the fresh output with the new password.
    let seen = backend.seen_passwords.lock().unwrap();
    assert_eq!(*seen, vec![None, Some("synthetic-user-secret".to_string())]);
    drop(seen);
    drop(secrets);
    // An explicit owner password is passed through verbatim.
    d.dispatch(
        "encryptPdf",
        &encrypt(json!({"ownerPassword":"synthetic-owner-secret"})),
        false,
        &c,
    )
    .unwrap();
    assert_eq!(
        backend.secrets.lock().unwrap()[1],
        (
            "synthetic-user-secret".into(),
            "synthetic-owner-secret".into()
        )
    );
    // decryptPdf opens the input with the supplied password only.
    let out = d
        .dispatch(
            "decryptPdf",
            &json!({"input":{"path":"locked.pdf"},"password":"synthetic-user-secret","output":{"path":"opened.pdf"}}),
            false,
            &c,
        )
        .unwrap();
    assert_eq!(out["result"]["path"], "opened.pdf");
    // A plain document is not a decrypt input.
    assert_eq!(
        d.dispatch(
            "decryptPdf",
            &json!({"input":{"path":"plain.pdf"},"password":"synthetic-user-secret","output":{"path":"no.pdf"}}),
            false,
            &c,
        )
        .unwrap_err()
        .0,
        "INVALID_REQUEST"
    );
    // An encrypted document is not an encrypt input.
    assert_eq!(
        d.dispatch("encryptPdf", &json!({"input":{"path":"locked.pdf"},"password":"synthetic-user-secret","output":{"path":"twice.pdf"}}), false, &c)
            .unwrap_err()
            .0,
        "ENCRYPTED_UNSUPPORTED"
    );
    // A signed document is never rewritten.
    let mut signed = encrypt(json!({}));
    signed["input"]["path"] = "signed.pdf".into();
    assert_eq!(
        d.dispatch("encryptPdf", &signed, false, &c).unwrap_err().0,
        "SIGNED_UNSUPPORTED"
    );
    // A wrong password surfaces as INVALID_PASSWORD, not a generic failure.
    assert_eq!(
        d.dispatch(
            "decryptPdf",
            &json!({"input":{"path":"locked.pdf"},"password":"not-it","output":{"path":"no2.pdf"}}),
            false,
            &c,
        )
        .unwrap_err()
        .0,
        "INVALID_PASSWORD"
    );
    // An intentionally empty user password is schema-valid: the mock sees
    // it verbatim and the response warns it provides no confidentiality.
    let empty = d
        .dispatch(
            "encryptPdf",
            &encrypt(
                json!({"password":"", "output":{"path":"owner-only.pdf","replaceExisting":true}}),
            ),
            false,
            &c,
        )
        .unwrap();
    assert!(
        empty["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "EMPTY_OPEN_PASSWORD"),
        "{empty}"
    );
}

#[test]
fn password_byte_bound() {
    // The PDF specification binds passwords to 127 UTF-8 *bytes* — JSON
    // Schema's maxLength counts characters, so the dispatcher re-checks in
    // bytes and never lets qpdf silently truncate.
    let base = |password: &str| json!({"input":{"path":"a.pdf"},"password":password,"output":{"path":"b.pdf"}});
    assert!(contract::invalid_detail("encryptPdf", &base(&"x".repeat(127))).is_none());
    // 128 characters trips the schema's maxLength first — masked, so the
    // rejected secret is not echoed.
    let long = contract::invalid_detail("encryptPdf", &base(&"x".repeat(128))).unwrap();
    assert!(
        long.starts_with("/password: ") && !long.contains('x'),
        "{long}"
    );
    // 127 characters but 128 bytes — multi-byte characters count several.
    assert!(
        contract::invalid_detail("encryptPdf", &base(&format!("{}é", "a".repeat(126)))).is_some()
    );
    // An intentionally empty password is valid (an R6 owner-only file);
    // an omitted one is not — the field stays required.
    assert!(contract::invalid_detail("encryptPdf", &base("")).is_none());
    assert!(
        contract::invalid_detail(
            "encryptPdf",
            &json!({"input":{"path":"a.pdf"},"output":{"path":"b.pdf"}})
        )
        .is_some()
    );
    // NUL would truncate silently at the backend's C-string boundary.
    assert_eq!(
        contract::invalid_detail("encryptPdf", &base("a\0b")).as_deref(),
        Some("/password: NUL bytes cannot be represented by the password transport")
    );
    // Validation errors name the constraint without echoing the secret —
    // direct fields and secrets nested inside a rejected container alike.
    let detail = contract::invalid_detail(
        "encryptPdf",
        &json!({"input":{"path":"a.pdf"},"password":123,"output":{"path":"b.pdf"}}),
    )
    .unwrap();
    assert!(
        detail.starts_with("/password: ") && !detail.contains("123"),
        "{detail}"
    );
    let wrapped = contract::invalid_detail(
        "encryptPdf",
        &json!([{"input":{"path":"a.pdf"},"password":"hidden-secret","output":{"path":"b.pdf"}}]),
    )
    .unwrap();
    assert!(!wrapped.contains("hidden-secret"), "{wrapped}");
    // A supplied blank owner password is refused: qpdf rejects it for
    // AES-256 because the output would open without a password.
    assert!(
        contract::invalid_detail(
            "encryptPdf",
            &json!({"input":{"path":"a.pdf"},"password":"p","ownerPassword":"","output":{"path":"b.pdf"}}),
        )
        .is_some_and(|d| d.starts_with("/ownerPassword: "))
    );
}
