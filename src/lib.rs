#[cfg(not(unix))]
compile_error!("anPDF filesystem/resource policy currently requires Linux or macOS");
pub mod backend;
pub mod contract;
pub mod error;
pub mod filesystem;
pub mod http;
pub mod mcp;
use backend::{Backend, Control, Qpdf};
use error::{Error, Result};
use filesystem::{TOTAL_LIMIT, Workspace};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub struct Dispatcher {
    workspace: Workspace,
    backend: Arc<dyn Backend>,
    active: AtomicUsize,
}
struct Permit<'a>(&'a AtomicUsize);
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Dispatcher {
    pub fn new(root: &Path, executable: PathBuf) -> Result<Self> {
        Self::with_backend(root, Arc::new(Qpdf { executable }))
    }
    pub fn with_backend(root: &Path, backend: Arc<dyn Backend>) -> Result<Self> {
        Ok(Self {
            workspace: Workspace::new(root)?,
            backend,
            active: AtomicUsize::new(0),
        })
    }
    pub fn dispatch(
        &self,
        id: &str,
        request: &Value,
        dry_run: bool,
        control: &Control,
    ) -> Result<Value> {
        if contract::operation(id).is_none() {
            return Err(Error("UNSUPPORTED_OPERATION"));
        }
        if !contract::valid_input(id, request) {
            return Err(Error("INVALID_REQUEST"));
        }
        if self
            .active
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                if n < 2 { Some(n + 1) } else { None }
            })
            .is_err()
        {
            return Err(Error("LIMIT_EXCEEDED"));
        }
        let _permit = Permit(&self.active);
        control.check()?;
        let specs = if id == "mergePdf" {
            request["inputs"].as_array().unwrap().clone()
        } else {
            vec![request["input"].clone()]
        };
        let mut inputs = specs
            .iter()
            .map(|v| self.workspace.input(v["path"].as_str().unwrap()))
            .collect::<Result<Vec<_>>>()?;
        if inputs.iter().map(|i| i.bytes).sum::<u64>() > TOTAL_LIMIT {
            return Err(Error("LIMIT_EXCEEDED"));
        }
        let pages = if id == "extractPdf" {
            let p = request["pages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect::<Vec<_>>();
            if p.windows(2).any(|w| w[0] >= w[1]) {
                return Err(Error("INVALID_REQUEST"));
            }
            Some(p)
        } else {
            None
        };
        let destination = if id != "inspectPdf" {
            Some(
                self.workspace.destination(
                    request["output"]["path"].as_str().unwrap(),
                    request["output"]["replaceExisting"]
                        .as_bool()
                        .unwrap_or(false),
                    &inputs,
                )?,
            )
        } else {
            None
        };
        if dry_run {
            return Ok(
                json!({"contractVersion":contract::VERSION,"operationId":id,"dryRun":true,"requestValid":true,"destinationPolicyValid":true,"contentValidation":"requires processing"}),
            );
        }
        let staging = tempfile::Builder::new()
            .prefix("anpdf-")
            .tempdir()
            .map_err(|_| Error("INTERNAL"))?;
        let mut paths = Vec::new();
        let mut warnings = Vec::new();
        let mut reports = Vec::new();
        let mut total_pages = 0u64;
        for (n, input) in inputs.iter_mut().enumerate() {
            control.check()?;
            let path = staging.path().join(format!("input-{n}.pdf"));
            input.stage(&path)?;
            let (mut report, mut w) = self.backend.inspect(&path, control)?;
            if !report.is_object() {
                return Err(Error("INTERNAL"));
            }
            report["sha256"] = json!(input.hash);
            report["bytes"] = json!(input.bytes);
            // Validate every backend report before trusting counts or preservation flags.
            if !contract::valid_output(
                "inspectPdf",
                &json!({"contractVersion":contract::VERSION,"operationId":"inspectPdf","result":report,"warnings":w}),
            ) {
                return Err(Error("INTERNAL"));
            }
            total_pages += report["pageCount"].as_u64().unwrap();
            if total_pages > 2000 {
                return Err(Error("LIMIT_EXCEEDED"));
            }
            if id != "inspectPdf" {
                if report["encrypted"] == true {
                    return Err(Error("ENCRYPTED_UNSUPPORTED"));
                }
                if report["signatures"] == "present" {
                    return Err(Error("SIGNED_UNSUPPORTED"));
                }
                if report["forms"] == "present" {
                    return Err(Error("FORM_UNSUPPORTED"));
                }
                if report["forms"] != "absent" || report["signatures"] != "absent" {
                    return Err(Error("PRESERVATION_UNKNOWN"));
                }
                if report["pageCount"] == 0 {
                    return Err(Error("INVALID_PDF"));
                }
            }
            warnings.append(&mut w);
            reports.push(report);
            paths.push(path);
        }
        let result = if let Some(destination) = destination {
            if pages
                .as_ref()
                .is_some_and(|p| p.last().copied().unwrap_or(0) > total_pages)
            {
                return Err(Error("INVALID_REQUEST"));
            }
            let out = staging.path().join("output.pdf");
            warnings.extend(
                self.backend
                    .transform(&paths, pages.as_deref(), &out, control)?,
            );
            let (report, w) = self.backend.inspect(&out, control)?;
            warnings.extend(w);
            let expected = pages.as_ref().map_or(total_pages, |p| p.len() as u64);
            if report["pageCount"] != expected
                || report["forms"] != "absent"
                || report["signatures"] != "absent"
            {
                return Err(Error("PRESERVATION_UNKNOWN"));
            }
            let mut file = std::fs::File::open(&out)?;
            let bytes = file.metadata()?.len();
            if bytes > TOTAL_LIMIT {
                return Err(Error("LIMIT_EXCEEDED"));
            }
            let result = json!({"path":request["output"]["path"],"sha256":filesystem::hash(&mut file)?,"bytes":bytes,"pageCount":expected});
            let envelope = json!({"contractVersion":contract::VERSION,"operationId":id,"result":result,"warnings":warnings});
            if !contract::valid_output(id, &envelope) {
                return Err(Error("INTERNAL"));
            }
            self.workspace.unchanged(&mut inputs)?;
            control.check()?;
            destination.commit(&out, &inputs)?;
            return Ok(envelope);
        } else {
            reports.remove(0)
        };
        self.workspace.unchanged(&mut inputs)?;
        control.check()?;
        let value = json!({"contractVersion":contract::VERSION,"operationId":id,"result":result,"warnings":warnings});
        if !contract::valid_output(id, &value) {
            return Err(Error("INTERNAL"));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests;
