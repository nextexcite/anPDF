#[cfg(not(unix))]
compile_error!("anPDF filesystem/resource policy currently requires Linux or macOS");
pub mod backend;
mod content;
pub mod contract;
pub mod error;
pub mod filesystem;
pub mod http;
pub mod mcp;
mod planner;
use backend::{Backend, CompressMode, Control};
use error::{Error, Result};
use filesystem::{TOTAL_LIMIT, Workspace};
pub use planner::BackendRegistry;
use planner::Job;
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
    registry: BackendRegistry,
    active: AtomicUsize,
    shutdown: std::sync::atomic::AtomicBool,
    controls: std::sync::Mutex<Vec<std::sync::Weak<std::sync::atomic::AtomicBool>>>,
}
struct Permit<'a>(&'a AtomicUsize);
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Dispatcher {
    pub fn new(root: &Path, executable: PathBuf) -> Result<Self> {
        Self::with_registry(root, BackendRegistry::with_qpdf(executable))
    }
    pub fn with_backend(root: &Path, backend: Arc<dyn Backend>) -> Result<Self> {
        Self::with_registry(root, BackendRegistry::with_structural(backend))
    }
    pub fn with_registry(root: &Path, registry: BackendRegistry) -> Result<Self> {
        Ok(Self {
            workspace: Workspace::new(root)?,
            registry,
            active: AtomicUsize::new(0),
            shutdown: std::sync::atomic::AtomicBool::new(false),
            controls: std::sync::Mutex::new(Vec::new()),
        })
    }
    pub fn discovery(&self) -> Value {
        contract::discovery_with(&self.registry)
    }
    pub fn cancel_all(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let mut controls = self.controls.lock().unwrap();
        controls.retain(|c| {
            if let Some(c) = c.upgrade() {
                c.store(true, Ordering::Relaxed);
                true
            } else {
                false
            }
        });
    }
    pub fn active_count(&self) -> usize {
        self.active.load(Ordering::SeqCst)
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
        {
            let mut controls = self.controls.lock().unwrap();
            controls.retain(|c| c.strong_count() > 0);
            controls.push(Arc::downgrade(&control.cancelled));
        }
        if self.shutdown.load(Ordering::SeqCst) {
            control.cancelled.store(true, Ordering::Relaxed);
        }
        control.check()?;
        let plan = self.registry.plan(id, request)?;
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
        let mut parts: Option<Vec<(Vec<u64>, String, filesystem::Destination)>> = None;
        let pages = if matches!(id, "extractPdf" | "removePages") {
            Some(parse_pages(&request["pages"])?)
        } else {
            None
        };
        let compress_opts = if matches!(plan.job, Job::Compress) {
            let mode_str = request["mode"].as_str().unwrap_or("lossless");
            let jpeg_quality = request["jpegQuality"].as_u64();
            let mode = if mode_str == "lossless" {
                if jpeg_quality.is_some() {
                    return Err(Error("INVALID_REQUEST"));
                }
                CompressMode::Lossless
            } else {
                CompressMode::Lossy {
                    jpeg_quality: jpeg_quality.unwrap_or(80) as u8,
                }
            };
            Some((mode_str.to_string(), mode))
        } else {
            None
        };
        let destination = if id == "splitPdf" {
            let mut parsed = Vec::new();
            for part in request["parts"].as_array().unwrap() {
                let p = parse_pages(&part["pages"])?;
                let out_path = part["output"]["path"].as_str().unwrap().to_string();
                let replace = part["output"]["replaceExisting"].as_bool().unwrap_or(false);
                let dest = self.workspace.destination(&out_path, replace, &inputs)?;
                for (_, _, existing) in &parsed {
                    if dest.same_target(existing)? {
                        return Err(Error("INVALID_REQUEST"));
                    }
                }
                parsed.push((p, out_path, dest));
            }
            parts = Some(parsed);
            None
        } else if plan.job.writes_output() {
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
            let (mut report, mut w) = plan.structural.inspect(&path, control)?;
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
            if !matches!(plan.job, Job::Inspect) {
                if report["encrypted"] == true {
                    return Err(Error("ENCRYPTED_UNSUPPORTED"));
                }
                if report["pageCount"] == 0 {
                    return Err(Error("INVALID_PDF"));
                }
            }
            if plan.job.writes_pdf() {
                if report["signatures"] == "present" {
                    return Err(Error("SIGNED_UNSUPPORTED"));
                }
                if report["forms"] == "present" {
                    return Err(Error("FORM_UNSUPPORTED"));
                }
                if report["forms"] != "absent" || report["signatures"] != "absent" {
                    return Err(Error("PRESERVATION_UNKNOWN"));
                }
            }
            warnings.append(&mut w);
            reports.push(report);
            paths.push(path);
        }
        match plan.job {
            Job::ExtractText {
                engine,
                backend,
                pages,
            } => {
                if pages.last().is_some_and(|&page| page > total_pages) {
                    return Err(Error("INVALID_REQUEST"));
                }
                control.check()?;
                let (extracted, provider_warnings) =
                    backend.extract_text(&paths[0], &pages, control)?;
                warnings.extend(provider_warnings);
                let value = content::text_envelope(engine, &pages, extracted, warnings)?;
                self.workspace.unchanged(&mut inputs)?;
                control.check()?;
                return Ok(value);
            }
            Job::RenderPage {
                engine,
                backend,
                page,
                dpi,
            } => {
                if page > total_pages {
                    return Err(Error("INVALID_REQUEST"));
                }
                control.check()?;
                let out = staging.path().join("output.png");
                warnings.extend(backend.render_page(&paths[0], page, dpi, &out, control)?);
                let (mut png, info) = content::read_png(&out)?;
                let result = json!({
                    "engine":engine, "page":page, "dpi":dpi, "mediaType":"image/png",
                    "path":request["output"]["path"], "sha256":filesystem::hash(&mut png)?,
                    "bytes":info.bytes, "width":info.width, "height":info.height,
                });
                let envelope = json!({"contractVersion":contract::VERSION,"operationId":id,"result":result,"warnings":warnings});
                content::validate_envelope(id, &envelope)?;
                self.workspace.unchanged(&mut inputs)?;
                control.check()?;
                destination
                    .unwrap()
                    .commit_file(&mut png, &inputs, backend::MAX_PNG_BYTES)?;
                return Ok(envelope);
            }
            _ => {}
        }
        if let Some(parts) = parts {
            let mut staged_outputs = Vec::with_capacity(parts.len());
            let mut artifacts = Vec::with_capacity(parts.len());
            let mut total_output_bytes = 0u64;
            for (idx, (part_pages, rel_path, _)) in parts.iter().enumerate() {
                if part_pages.last().copied().unwrap_or(0) > total_pages {
                    return Err(Error("INVALID_REQUEST"));
                }
                let out = staging.path().join(format!("output-{idx}.pdf"));
                let (artifact, bytes) = transform_artifact(
                    plan.structural,
                    &paths,
                    Some(part_pages),
                    total_pages,
                    &out,
                    rel_path,
                    control,
                    &mut warnings,
                )?;
                total_output_bytes += bytes;
                if total_output_bytes > TOTAL_LIMIT {
                    return Err(Error("LIMIT_EXCEEDED"));
                }
                artifacts.push(artifact);
                staged_outputs.push(out);
            }
            let envelope = json!({"contractVersion":contract::VERSION,"operationId":id,"result":{"outputs":artifacts},"warnings":warnings});
            if !contract::valid_output(id, &envelope) {
                return Err(Error("INTERNAL"));
            }
            self.workspace.unchanged(&mut inputs)?;
            control.check()?;
            for (idx, (_, _, dest)) in parts.iter().enumerate() {
                if let Err(e) = dest.commit(&staged_outputs[idx], &inputs) {
                    for (_, _, committed) in &parts[..idx] {
                        committed.rollback_new();
                    }
                    return Err(e);
                }
            }
            return Ok(envelope);
        }
        let result = if let Some(destination) = destination {
            if let Some((mode_str, compress_mode)) = compress_opts {
                let out = staging.path().join("output.pdf");
                warnings.extend(plan.structural.compress(
                    &paths[0],
                    compress_mode,
                    &out,
                    control,
                )?);
                let (report, w) = plan.structural.inspect(&out, control)?;
                warnings.extend(w);
                if report["pageCount"] != total_pages
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
                let input_bytes = inputs[0].bytes;
                let saved_bytes = input_bytes as i64 - bytes as i64;
                let ratio = if input_bytes > 0 {
                    ((bytes as f64 / input_bytes as f64) * 10000.0).round() / 10000.0
                } else {
                    0.0
                };
                if bytes >= input_bytes {
                    warnings.push(json!({
                        "code": "NO_SIZE_REDUCTION",
                        "message": "Output size is not smaller than input; input may already be compact."
                    }));
                }
                let result = json!({
                    "path": request["output"]["path"],
                    "sha256": filesystem::hash(&mut file)?,
                    "bytes": bytes,
                    "pageCount": total_pages,
                    "inputBytes": input_bytes,
                    "savedBytes": saved_bytes,
                    "compressionRatio": ratio,
                    "mode": mode_str,
                });
                let envelope = json!({"contractVersion":contract::VERSION,"operationId":id,"result":result,"warnings":warnings});
                if !contract::valid_output(id, &envelope) {
                    return Err(Error("INTERNAL"));
                }
                self.workspace.unchanged(&mut inputs)?;
                control.check()?;
                destination.commit(&out, &inputs)?;
                return Ok(envelope);
            }
            if pages
                .as_ref()
                .is_some_and(|p| p.last().copied().unwrap_or(0) > total_pages)
            {
                return Err(Error("INVALID_REQUEST"));
            }
            let selected_pages = if id == "removePages" {
                let remove = pages.as_ref().unwrap();
                let kept: Vec<u64> = (1..=total_pages).filter(|p| !remove.contains(p)).collect();
                if kept.is_empty() {
                    return Err(Error("INVALID_REQUEST"));
                }
                Some(kept)
            } else {
                pages
            };
            let out = staging.path().join("output.pdf");
            let (result, _) = transform_artifact(
                plan.structural,
                &paths,
                selected_pages.as_deref(),
                total_pages,
                &out,
                request["output"]["path"].as_str().unwrap(),
                control,
                &mut warnings,
            )?;
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

fn request_integer(value: &Value) -> Result<u64> {
    value
        .as_u64()
        .or_else(|| {
            // JSON Schema permits integral floats, but never round a fractional selection.
            value
                .as_f64()
                .filter(|v| (0.0..=9_007_199_254_740_991.0).contains(v) && v.fract() == 0.0)
                .map(|v| v as u64)
        })
        .ok_or(Error("INVALID_REQUEST"))
}

fn parse_pages(value: &Value) -> Result<Vec<u64>> {
    let pages = value
        .as_array()
        .unwrap()
        .iter()
        .map(request_integer)
        .collect::<Result<Vec<_>>>()?;
    if pages.windows(2).any(|w| w[0] >= w[1]) {
        return Err(Error("INVALID_REQUEST"));
    }
    Ok(pages)
}

#[allow(clippy::too_many_arguments)]
fn transform_artifact(
    backend: &dyn Backend,
    paths: &[PathBuf],
    pages: Option<&[u64]>,
    total_input_pages: u64,
    out: &Path,
    rel_path: &str,
    control: &Control,
    warnings: &mut Vec<Value>,
) -> Result<(Value, u64)> {
    warnings.extend(backend.transform(paths, pages, out, control)?);
    let (report, w) = backend.inspect(out, control)?;
    warnings.extend(w);
    let expected = pages.map_or(total_input_pages, |p| p.len() as u64);
    if report["pageCount"] != expected
        || report["forms"] != "absent"
        || report["signatures"] != "absent"
    {
        return Err(Error("PRESERVATION_UNKNOWN"));
    }
    let mut file = std::fs::File::open(out)?;
    let bytes = file.metadata()?.len();
    if bytes > TOTAL_LIMIT {
        return Err(Error("LIMIT_EXCEEDED"));
    }
    Ok((
        json!({"path":rel_path,"sha256":filesystem::hash(&mut file)?,"bytes":bytes,"pageCount":expected}),
        bytes,
    ))
}

#[cfg(test)]
mod tests;
