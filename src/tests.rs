use crate::{contract, error::Error, filesystem::Workspace};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

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
        fn inspect(&self, path: &Path, _: &Control, _: bool) -> Result<(Value, Vec<Value>)> {
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
    for aspect in ["metadata", "bookmarks", "attachments", "annotations"] {
        let request = json!({"input":{"path":"in.pdf"},"pages":[1],"output":{"path":"existing.pdf","replaceExisting":true},"preserve":{aspect:true}});
        assert_eq!(
            d.dispatch("extractPdf", &request, false, &Control::default())
                .unwrap_err()
                .0,
            "PRESERVATION_UNMET"
        );
        assert_eq!(
            fs::read(t.path().join("existing.pdf")).unwrap(),
            b"existing"
        );
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
    };
    let c = Control {
        deadline: Instant::now() + Duration::from_millis(40),
        ..Control::default()
    };
    let start = Instant::now();
    assert_eq!(q.inspect(&exe, &c, true).unwrap_err().0, "TIMEOUT");
    assert!(start.elapsed() < Duration::from_secs(2));
    fs::write(
        &exe,
        "#!/usr/bin/python3\nimport sys\nsys.stdout.buffer.write(b'x'*(40*1024*1024))\n",
    )
    .unwrap();
    assert_eq!(
        q.inspect(&exe, &Control::default(), true).unwrap_err().0,
        "LIMIT_EXCEEDED"
    );
    fs::write(
        &exe,
        "#!/usr/bin/python3\nimport sys\nsys.stderr.buffer.write(b'e'*100000)\n",
    )
    .unwrap();
    assert_eq!(
        q.inspect(&exe, &Control::default(), true).unwrap_err().0,
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
            _: &crate::backend::Control,
            _: bool,
        ) -> crate::error::Result<(serde_json::Value, Vec<serde_json::Value>)> {
            let pages = if path.file_name().and_then(|s| s.to_str()) == Some("input-0.pdf") {
                3
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
