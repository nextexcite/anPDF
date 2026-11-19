use crate::{contract, error::Error, filesystem::Workspace};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
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
    assert!(!contract::valid_output(
        "inspectPdf",
        &json!({"result":{"pageCount":-1}})
    ));
    for code in
        contract::document()["components"]["schemas"]["Error"]["properties"]["error"]["properties"]
            ["code"]["enum"]
            .as_array()
            .unwrap()
    {
        let error = Error(Box::leak(
            code.as_str().unwrap().to_string().into_boxed_str(),
        ));
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
    };
    let c = Control {
        deadline: Instant::now() + Duration::from_millis(40),
        ..Control::default()
    };
    let start = Instant::now();
    assert_eq!(q.inspect(&exe, &c).unwrap_err().0, "TIMEOUT");
    assert!(start.elapsed() < Duration::from_secs(2));
    fs::write(
        &exe,
        "#!/usr/bin/python3\nimport sys\nsys.stdout.buffer.write(b'x'*(40*1024*1024))\n",
    )
    .unwrap();
    assert_eq!(
        q.inspect(&exe, &Control::default()).unwrap_err().0,
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
