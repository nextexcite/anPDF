#[cfg(not(unix))]
compile_error!("anPDF filesystem/resource policy currently requires Linux or macOS");
pub mod backend;
mod content;
pub mod contract;
pub mod error;
pub mod filesystem;
pub mod http;
pub mod mcp;
#[cfg(feature = "pdfium")]
pub mod pdfium;
#[cfg(feature = "pdfoxide")]
pub mod pdfoxide;
mod planner;
pub mod resample;
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
        Self::new_exact(root, executable, false)
    }
    pub fn new_exact(root: &Path, executable: PathBuf, exact: bool) -> Result<Self> {
        Self::with_registry(root, BackendRegistry::with_qpdf_exact(executable, exact))
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
            return Err(Error("UNSUPPORTED_OPERATION", None));
        }
        if let Some(detail) = contract::invalid_detail(id, request) {
            return Err(Error("INVALID_REQUEST", Some(detail)));
        }
        if self
            .active
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                if n < 2 { Some(n + 1) } else { None }
            })
            .is_err()
        {
            return Err(Error("LIMIT_EXCEEDED", None));
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
            return Err(Error("LIMIT_EXCEEDED", None));
        }
        let mut parts: Option<Vec<(Vec<u64>, String, filesystem::Destination)>> = None;
        let pages = if matches!(id, "extractPdf" | "removePages") {
            Some(parse_pages(&request["pages"])?)
        } else {
            None
        };
        /// Descent ladder for size-targeted compression: (DPI cap, JPEG quality)
        /// pairs from gentlest to hardest. `compressionLevel` bounds how far the
        /// ladder may descend — level 2 floors at the 200dpi rung, level 3 (the
        /// default floor for a bare target) at the 120dpi rung.
        struct CompressSpec {
            mode: CompressMode,
            lossy: bool,
            max_output_bytes: Option<u64>,
            error_on_miss: bool,
            level: Option<u64>,
        }
        const LADDER: &[(u32, u8)] = &[(300, 80), (200, 75), (150, 65), (120, 55)];
        let compress_spec = if matches!(plan.job, Job::Compress) {
            let mode_str = request["mode"].as_str().unwrap_or("lossless");
            let jpeg_quality = request["jpegQuality"].as_u64();
            let max_output = request
                .get("maxOutputBytes")
                .map(request_integer)
                .transpose()?;
            let level = request
                .get("compressionLevel")
                .map(request_integer)
                .transpose()?;
            if request.get("onTargetMiss").is_some() && max_output.is_none() {
                return Err(Error(
                    "INVALID_REQUEST",
                    Some("onTargetMiss requires maxOutputBytes".into()),
                ));
            }
            let lossy_requested = mode_str == "lossy";
            match level {
                Some(1) if lossy_requested => {
                    return Err(Error(
                        "INVALID_REQUEST",
                        Some("compressionLevel 1 is strictly lossless and conflicts with mode \"lossy\"".into()),
                    ));
                }
                Some(2..=3) if mode_str == "lossless" && request.get("mode").is_some() => {
                    return Err(Error(
                        "INVALID_REQUEST",
                        Some("compressionLevel 2..3 performs lossy resampling and conflicts with mode \"lossless\"".into()),
                    ));
                }
                Some(1..=3) | None => {}
                Some(_) => return Err(Error("INVALID_REQUEST", None)),
            }
            // A bare target still implies lossy descent — except at level 1,
            // which stays strictly lossless and simply reports the miss.
            let lossy = lossy_requested
                || level.is_some_and(|l| l >= 2)
                || (max_output.is_some() && level != Some(1));
            if jpeg_quality.is_some() && !lossy_requested {
                return Err(Error(
                    "INVALID_REQUEST",
                    Some("jpegQuality applies only to mode \"lossy\"".into()),
                ));
            }
            if lossy_requested && level == Some(1) {
                return Err(Error(
                    "INVALID_REQUEST",
                    Some("mode \"lossy\" requires compressionLevel 2 or 3".into()),
                ));
            }
            let mode = if lossy {
                CompressMode::Lossy {
                    jpeg_quality: jpeg_quality.unwrap_or(80) as u8,
                }
            } else {
                CompressMode::Lossless
            };
            Some(CompressSpec {
                mode,
                lossy,
                max_output_bytes: max_output,
                error_on_miss: request["onTargetMiss"].as_str() == Some("error"),
                level,
            })
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
                        return Err(Error(
                            "INVALID_REQUEST",
                            Some(format!("parts write to the same output '{out_path}'")),
                        ));
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
            .map_err(|_| Error("INTERNAL", None))?;
        let mut paths = Vec::new();
        let mut warnings = Vec::new();
        let mut reports = Vec::new();
        let mut total_pages = 0u64;
        for (n, input) in inputs.iter_mut().enumerate() {
            control.check()?;
            let path = staging.path().join(format!("input-{n}.pdf"));
            input.stage(&path)?;
            // Operations that write PDFs keep the full check on their inputs:
            // an undecodable stream would otherwise be copied into published
            // output. Only the user-facing inspect op gets the shallow default.
            let deep = request["deepCheck"].as_bool().unwrap_or(false)
                || !matches!(plan.job, Job::Inspect);
            let (mut report, mut w) = plan.structural.inspect(&path, control, deep)?;
            if matches!(plan.job, Job::Inspect) && !deep {
                w.push(json!({"code":"STREAM_DECODE_UNCHECKED","message":"Stream payloads were parsed but not decoded; set deepCheck to true for complete qpdf structural validation."}));
            }
            if !report.is_object() {
                return Err(Error("INTERNAL", None));
            }
            report["sha256"] = json!(input.hash);
            report["bytes"] = json!(input.bytes);
            // Validate every backend report before trusting counts or preservation flags.
            if !contract::valid_output(
                "inspectPdf",
                &json!({"contractVersion":contract::VERSION,"operationId":"inspectPdf","result":report,"warnings":w}),
            ) {
                return Err(Error("INTERNAL", None));
            }
            total_pages += report["pageCount"].as_u64().unwrap();
            if total_pages > 2000 {
                return Err(Error("LIMIT_EXCEEDED", None));
            }
            if !matches!(plan.job, Job::Inspect) {
                if report["encrypted"] == true {
                    return Err(Error("ENCRYPTED_UNSUPPORTED", None));
                }
                if report["pageCount"] == 0 {
                    return Err(Error("INVALID_PDF", None));
                }
            }
            if plan.job.writes_pdf() {
                if report["signatures"] == "present" {
                    return Err(Error("SIGNED_UNSUPPORTED", None));
                }
                if report["forms"] == "present" {
                    return Err(Error("FORM_UNSUPPORTED", None));
                }
                if report["forms"] != "absent" || report["signatures"] != "absent" {
                    return Err(Error("PRESERVATION_UNKNOWN", None));
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
                    return Err(Error("INVALID_REQUEST", None));
                }
                control.check()?;
                let (extracted, provider_warnings) =
                    backend.extract_text(&paths[0], &pages, control)?;
                warnings.extend(provider_warnings);
                dedupe_warnings(&mut warnings);
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
                    return Err(Error("INVALID_REQUEST", None));
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
                dedupe_warnings(&mut warnings);
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
                    return Err(Error("INVALID_REQUEST", None));
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
                    &reports,
                    &request["preserve"],
                )?;
                total_output_bytes += bytes;
                if total_output_bytes > TOTAL_LIMIT {
                    return Err(Error("LIMIT_EXCEEDED", None));
                }
                artifacts.push(artifact);
                staged_outputs.push(out);
            }
            dedupe_warnings(&mut warnings);
            let envelope = json!({"contractVersion":contract::VERSION,"operationId":id,"result":{"outputs":artifacts},"warnings":warnings});
            if !contract::valid_output(id, &envelope) {
                return Err(Error("INTERNAL", None));
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
            if let Some(spec) = compress_spec {
                let out = staging.path().join("output.pdf");
                warnings.extend(
                    plan.structural
                        .compress(&paths[0], spec.mode, &out, control)?,
                );
                let mut techniques: Vec<&'static str> =
                    vec!["flate-recompress", "object-streams", "resource-drop"];
                if spec.lossy {
                    techniques.push("jpeg-recompress");
                }
                let mut candidate = out.clone();
                let mut candidate_bytes = std::fs::metadata(&out)?.len();
                // Rungs the ladder may touch: a bare target descends to the
                // level floor (default 120dpi); a level without a target pins
                // exactly its rung; level 1 never resamples.
                let rungs: &[(u32, u8)] = match (spec.max_output_bytes.is_some(), spec.level) {
                    (_, Some(1)) => &[],
                    (true, Some(2)) => &LADDER[..2],
                    (true, _) => LADDER,
                    (false, Some(2)) => &LADDER[1..2],
                    (false, Some(3)) => &LADDER[3..4],
                    (false, _) => &[],
                };
                if !rungs.is_empty() && spec.max_output_bytes.is_none_or(|t| candidate_bytes > t) {
                    match plan.resampler {
                        None if spec.max_output_bytes.is_none() => {
                            return Err(Error("BACKEND_UNAVAILABLE", None))
                        }
                        None => warnings.push(json!({
                            "code": "SIZE_TARGET_UNMET",
                            "message": "Compressed output could not meet the requested maximum size; no image-resampling backend is configured."
                        })),
                        Some(resampler) => {
                            'ladder: for (rung, (dpi, quality)) in rungs.iter().enumerate() {
                        control.check()?;
                        let raw = staging.path().join(format!("rung{rung}.raw.pdf"));
                        // Each rung resamples the initial qpdf pass output —
                        // already repaired — rather than the previous rung's
                        // pixels, so JPEG artifacts never accumulate.
                        let summary = resampler.resample_images(
                            &out,
                            &raw,
                            *dpi,
                            *quality,
                            control,
                        )?;
                        let skipped = summary["imagesSkipped"].as_array().cloned().unwrap_or_default();
                        if !skipped.is_empty() {
                            warnings.push(json!({
                                "code": "IMAGE_RESAMPLE_SKIPPED",
                                "message": format!("{} images could not be safely resampled and were left unchanged.", skipped.len())
                            }));
                        }
                        let processed = summary["imagesProcessed"].as_u64().unwrap_or(0);
                        if processed == 0 {
                            // No image exceeded this rung's DPI cap; deeper
                            // rungs can still qualify lower-resolution images.
                            continue 'ladder;
                        }
                        let finished = staging.path().join(format!("rung{rung}.pdf"));
                        warnings.extend(plan.structural.compress(
                            &raw,
                            CompressMode::Lossy { jpeg_quality: *quality },
                            &finished,
                            control,
                        )?);
                        let finished_bytes = std::fs::metadata(&finished)?.len();
                        if !techniques.contains(&"image-downsample") {
                            techniques.push("image-downsample");
                        }
                        if finished_bytes < candidate_bytes {
                            candidate = finished;
                            candidate_bytes = finished_bytes;
                        }
                        // Stop only when the target is met; non-image bytes
                        // still shrink under the qpdf finish, so no floor check.
                        if spec.max_output_bytes.is_none_or(|t| candidate_bytes <= t) {
                            break 'ladder;
                        }
                            }
                        }
                    }
                }
                let (report, w) = plan.structural.inspect(&candidate, control, false)?;
                warnings.extend(w);
                if report["pageCount"] != total_pages
                    || report["forms"] != "absent"
                    || report["signatures"] != "absent"
                {
                    return Err(Error("PRESERVATION_UNKNOWN", None));
                }
                let compressed_bytes = candidate_bytes;
                let input_bytes = inputs[0].bytes;
                let keep_original =
                    compressed_bytes >= input_bytes && request["allowLargerOutput"] != true;
                let source = if keep_original { &paths[0] } else { &candidate };
                let mut file = std::fs::File::open(source)?;
                let bytes = file.metadata()?.len();
                if bytes > TOTAL_LIMIT {
                    return Err(Error("LIMIT_EXCEEDED", None));
                }
                if keep_original {
                    techniques.clear();
                }
                let saved_bytes = input_bytes as i64 - bytes as i64;
                let ratio = if keep_original {
                    1.0
                } else if input_bytes > 0 {
                    ((bytes as f64 / input_bytes as f64) * 10000.0).round() / 10000.0
                } else {
                    0.0
                };
                if compressed_bytes >= input_bytes {
                    warnings.push(json!({
                        "code": "NO_SIZE_REDUCTION",
                        "message": if keep_original { "Compressed output is not smaller; original bytes were kept." } else { "Compressed output is not smaller; compressed bytes were kept as requested." }
                    }));
                }
                let target_met = spec.max_output_bytes.map(|t| bytes <= t);
                if target_met == Some(false) {
                    if spec.error_on_miss {
                        return Err(Error("SIZE_TARGET_UNMET", None));
                    }
                    warnings.push(json!({
                        "code": "SIZE_TARGET_UNMET",
                        "message": "Compressed output exceeds maxOutputBytes after descending the full ladder; smallest achieved bytes were published."
                    }));
                }
                let mut result = json!({
                    "path": request["output"]["path"],
                    "sha256": filesystem::hash(&mut file)?,
                    "bytes": bytes,
                    "pageCount": total_pages,
                    "inputBytes": input_bytes,
                    "savedBytes": saved_bytes,
                    "compressionRatio": ratio,
                    "mode": if spec.lossy { "lossy" } else { "lossless" },
                    "outputSource": if keep_original { "original" } else { "compressed" },
                    "techniques": techniques,
                });
                if let Some(target) = spec.max_output_bytes {
                    result["targetBytes"] = json!(target);
                    result["targetMet"] = json!(target_met.unwrap_or(false));
                }
                dedupe_warnings(&mut warnings);
                let envelope = json!({"contractVersion":contract::VERSION,"operationId":id,"result":result,"warnings":warnings});
                if !contract::valid_output(id, &envelope) {
                    return Err(Error("INTERNAL", None));
                }
                self.workspace.unchanged(&mut inputs)?;
                control.check()?;
                destination.commit(source, &inputs)?;
                return Ok(envelope);
            }
            if pages
                .as_ref()
                .is_some_and(|p| p.last().copied().unwrap_or(0) > total_pages)
            {
                return Err(Error("INVALID_REQUEST", None));
            }
            let selected_pages = if id == "removePages" {
                let remove = pages.as_ref().unwrap();
                let kept: Vec<u64> = (1..=total_pages).filter(|p| !remove.contains(p)).collect();
                if kept.is_empty() {
                    return Err(Error("INVALID_REQUEST", None));
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
                &reports,
                &request["preserve"],
            )?;
            dedupe_warnings(&mut warnings);
            let envelope = json!({"contractVersion":contract::VERSION,"operationId":id,"result":result,"warnings":warnings});
            if !contract::valid_output(id, &envelope) {
                return Err(Error("INTERNAL", None));
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
        dedupe_warnings(&mut warnings);
        let value = json!({"contractVersion":contract::VERSION,"operationId":id,"result":result,"warnings":warnings});
        if !contract::valid_output(id, &value) {
            return Err(Error("INTERNAL", None));
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
        .ok_or(Error("INVALID_REQUEST", None))
}

fn parse_pages(value: &Value) -> Result<Vec<u64>> {
    let pages = value
        .as_array()
        .unwrap()
        .iter()
        .map(request_integer)
        .collect::<Result<Vec<_>>>()?;
    if pages.windows(2).any(|w| w[0] >= w[1]) {
        return Err(Error("INVALID_REQUEST", None));
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
    inputs: &[Value],
    preserve: &Value,
) -> Result<(Value, u64)> {
    warnings.extend(backend.transform(paths, pages, out, control)?);
    let (report, w) = backend.inspect(out, control, false)?;
    warnings.extend(w);
    let expected = pages.map_or(total_input_pages, |p| p.len() as u64);
    if report["pageCount"] != expected
        || report["forms"] != "absent"
        || report["signatures"] != "absent"
    {
        return Err(Error("PRESERVATION_UNKNOWN", None));
    }
    let mut file = std::fs::File::open(out)?;
    let bytes = file.metadata()?.len();
    if bytes > TOTAL_LIMIT {
        return Err(Error("LIMIT_EXCEEDED", None));
    }
    let preservation = preservation_status(inputs, &report, pages);
    for aspect in ["metadata", "bookmarks", "annotations", "attachments"] {
        if preserve[aspect] == true && preservation[aspect] == "omitted" {
            return Err(Error("PRESERVATION_UNMET", None));
        }
    }
    let omitted = ["metadata", "bookmarks", "annotations", "attachments"]
        .into_iter()
        .filter(|a| preservation[*a] == "omitted")
        .collect::<Vec<_>>();
    if !omitted.is_empty() {
        warnings.push(json!({"code":"PRESERVATION_OMITTED","message":format!("Document aspects omitted or not verified: {}.", omitted.join(", "))}));
    }
    Ok((
        json!({"path":rel_path,"sha256":filesystem::hash(&mut file)?,"bytes":bytes,"pageCount":expected,"preservation":preservation}),
        bytes,
    ))
}

fn dedupe_warnings(warnings: &mut Vec<Value>) {
    let mut unique = Vec::new();
    let mut omitted = std::collections::BTreeSet::new();
    for warning in warnings.drain(..) {
        if warning["code"] == "PRESERVATION_OMITTED" {
            for aspect in ["metadata", "bookmarks", "annotations", "attachments"] {
                if warning["message"]
                    .as_str()
                    .is_some_and(|m| m.contains(aspect))
                {
                    omitted.insert(aspect);
                }
            }
        } else if !unique.contains(&warning) {
            unique.push(warning);
        }
    }
    if !omitted.is_empty() {
        unique.push(json!({"code":"PRESERVATION_OMITTED","message":format!("Document aspects omitted or not verified: {}.", omitted.into_iter().collect::<Vec<_>>().join(", "))}));
    }
    *warnings = unique;
}

fn annotation_count(report: &Value, pages: Option<&[u64]>) -> Option<u64> {
    let entries = report["pages"].as_array()?;
    if entries.len() as u64 != report["pageCount"].as_u64()? {
        return None;
    }
    entries
        .iter()
        .filter(|p| {
            pages.is_none_or(|selected| {
                p["pageNumber"]
                    .as_u64()
                    .is_some_and(|n| selected.contains(&n))
            })
        })
        .try_fold(0u64, |sum, p| {
            sum.checked_add(p["annotationCount"].as_u64()?)
        })
}

fn preservation_status(inputs: &[Value], output: &Value, pages: Option<&[u64]>) -> Value {
    let status = |exists: bool, carried: bool| {
        if !exists {
            "absent"
        } else if carried {
            "preserved"
        } else {
            "omitted"
        }
    };
    let metadata_present = |i: &Value| {
        i["hasMetadata"] == true || i["metadata"].as_object().is_none_or(|m| !m.is_empty())
    };
    let evidence_matches = |i: &Value, key: &str| {
        i["preservationEvidence"][key]
            .as_str()
            .is_some_and(|hash| output["preservationEvidence"][key] == hash)
    };
    let metadata_exists = inputs.iter().any(metadata_present);
    let metadata_preserved = inputs
        .iter()
        .all(|i| !metadata_present(i) || evidence_matches(i, "metadata"));
    let feature = |key: &str, aspect: &str| {
        let exists = inputs.iter().any(|i| i[key] != false);
        let primary_matches = if aspect == "bookmarks" {
            bookmarks_match(&inputs[0], output, pages)
        } else {
            evidence_matches(&inputs[0], aspect)
        };
        let carried = output[key] == true
            && primary_matches
            && inputs.iter().skip(1).all(|i| i[key] == false);
        status(exists, carried)
    };
    let input_annotations = inputs
        .iter()
        .try_fold(0u64, |sum, i| sum.checked_add(annotation_count(i, pages)?));
    let output_annotations = annotation_count(output, None);
    json!({
        "metadata": status(metadata_exists, metadata_preserved),
        "bookmarks": feature("hasOutline", "bookmarks"),
        "attachments": feature("hasAttachments", "attachments"),
        "annotations": match (input_annotations, output_annotations) {
            (Some(0), _) => "absent",
            (Some(a), Some(b)) if a == b => "preserved",
            _ => "omitted",
        }
    })
}

fn bookmarks_match(input: &Value, output: &Value, pages: Option<&[u64]>) -> bool {
    fn remap(outlines: &mut Value, pages: &[u64]) -> Option<()> {
        for outline in outlines.as_array_mut()? {
            if let Some(n) = outline["destpageposfrom1"].as_u64() {
                let position = pages.iter().position(|p| *p == n)? as u64 + 1;
                outline["destpageposfrom1"] = json!(position);
                if let Some(dest) = outline["dest"].as_array_mut() {
                    *dest.first_mut()? = json!(position);
                }
            }
            remap(&mut outline["kids"], pages)?;
        }
        Some(())
    }
    let Some(source) = input["preservationEvidence"]["bookmarks"].as_str() else {
        return false;
    };
    let Some(target) = output["preservationEvidence"]["bookmarks"].as_str() else {
        return false;
    };
    let (Ok(mut source), Ok(target)) = (
        serde_json::from_str::<Value>(source),
        serde_json::from_str::<Value>(target),
    ) else {
        return false;
    };
    if let Some(pages) = pages
        && remap(&mut source, pages).is_none()
    {
        return false;
    }
    source == target
}

#[cfg(test)]
mod tests;
