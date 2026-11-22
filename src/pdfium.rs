use crate::{
    backend::{
        Control, ExtractTextBackend, MAX_TEXT_RESPONSE_BYTES, PageRasterizer, Qpdf,
        RenderPageBackend, validate_render_dimensions,
    },
    error::{Error, Result},
    resample::{
        MAX_RUNGS, achromatic_to_gray, choose_rung, encode_jpeg, required_pixels, resampled,
    },
};
use lopdf::{Dictionary, Document, Object};
use pdfium_render::prelude::*;
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Isolated adapter whose worker executable must handle the `pdfium-worker` subcommand.
pub struct PdfiumProvider {
    pub library: PathBuf,
    pub worker_executable: PathBuf,
}

impl PdfiumProvider {
    fn run(&self, request: Value, control: &Control) -> Result<Value> {
        let (code, bytes) = Qpdf {
            executable: self.worker_executable.clone(),
            exact: false,
            as_limit: 768 * 1024 * 1024,
            cpu_scale: 1,
        }
        .run(
            &[
                "pdfium-worker".into(),
                self.library
                    .to_str()
                    .ok_or(Error("BACKEND_UNAVAILABLE", None))?
                    .into(),
                request.to_string(),
            ],
            control,
        )?;
        if bytes.len() > MAX_TEXT_RESPONSE_BYTES {
            return Err(Error("LIMIT_EXCEEDED", None));
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error("INTERNAL", None))?;
        if code != 0 {
            return Err(Error(
                match value["error"].as_str() {
                    Some("BACKEND_UNAVAILABLE") => "BACKEND_UNAVAILABLE",
                    Some("INVALID_PDF") => "INVALID_PDF",
                    Some("INVALID_REQUEST") => "INVALID_REQUEST",
                    Some("LIMIT_EXCEEDED") => "LIMIT_EXCEEDED",
                    Some("TIMEOUT") => "TIMEOUT",
                    _ => "INTERNAL",
                },
                value["error"]["message"].as_str().map(String::from),
            ));
        }
        Ok(value)
    }
}

impl ExtractTextBackend for PdfiumProvider {
    fn extract_text(
        &self,
        input: &Path,
        pages: &[u64],
        control: &Control,
    ) -> Result<(Vec<Value>, Vec<Value>)> {
        let value = self.run(json!({"input":input,"pages":pages}), control)?;
        Ok((
            value.as_array().ok_or(Error("INTERNAL", None))?.clone(),
            Vec::new(),
        ))
    }
}

impl RenderPageBackend for PdfiumProvider {
    fn render_page(
        &self,
        input: &Path,
        page: u64,
        dpi: u32,
        output: &Path,
        control: &Control,
    ) -> Result<Vec<Value>> {
        self.run(
            json!({"input":input,"page":page,"dpi":dpi,"output":output}),
            control,
        )?;
        Ok(Vec::new())
    }
}

impl PageRasterizer for PdfiumProvider {
    fn rasterize_document(
        &self,
        input: &Path,
        output: &Path,
        rungs: &[(u32, u8)],
        target: Option<u64>,
        control: &Control,
    ) -> Result<Value> {
        self.run(
            json!({
                "rasterize": true,
                "input": input,
                "output": output,
                "rungs": rungs
                    .iter()
                    .map(|(d, q)| json!({"dpiCap": d, "jpegQuality": q}))
                    .collect::<Vec<_>>(),
                "targetBytes": target,
                "budgetMillis": control.remaining().as_millis() as u64,
            }),
            control,
        )
    }
}

/// Painted-path operators: `m l c v y` build paths, `f F f* B B* b b* s S`
/// paint them. A dense traced page runs hundreds of these; a simple diagram
/// or text page runs few or none.
const PATH_BUILD_OPS: &[&[u8]] = &[b"m", b"l", b"c", b"v", b"y"];
const PATH_PAINT_OPS: &[&[u8]] = &[
    b"f", b"F", b"f*", b"B", b"B*", b"b", b"b*", b"s", b"S", b"n",
];
const TEXT_OPS: &[&[u8]] = &[b"Tj", b"TJ", b"'", b"\""];
/// Operators whose semantics make page-region isolation uncertain (optional
/// content / layer ranges, blend or alpha state changes).
const UNCLEAR_OPS: &[&[u8]] = &[b"BDC", b"EMC", b"MP", b"DP", b"sh"];
/// Dense traced page: at least this many painted paths drawn on it.
const TRACED_PATH_FLOOR: usize = 64;

/// Positive-evidence check for a traced-graphics page: dense painted paths
/// with no text operators, no annotations, and no layer/blend/clip content
/// that makes region separation uncertain. Returns a skip reason when the
/// page keeps its original objects.
fn traced_evidence(doc: &Document, page: &Dictionary, contents: &[u8]) -> Option<&'static str> {
    if page.get(b"Annots").is_ok() {
        return Some("annotations");
    }
    // The rebuilt page draws the rendered bitmap over the full box at its
    // origin; a rotated page would rotate it twice and a shifted or cropped
    // box would misplace it, so those keep original objects.
    let box_origin = |k: &[u8]| {
        resample_lookup(doc, page, k)
            .and_then(|b| b.as_array().ok().cloned())
            .map(|a| {
                (
                    a.first().and_then(|v| v.as_float().ok()).unwrap_or(0.0),
                    a.get(1).and_then(|v| v.as_float().ok()).unwrap_or(0.0),
                )
            })
    };
    if page
        .get(b"Rotate")
        .ok()
        .is_some_and(|r| r.as_i64().ok().unwrap_or(0) != 0)
        || box_origin(b"MediaBox").is_some_and(|o| o != (0.0, 0.0))
        || box_origin(b"CropBox")
            .zip(box_origin(b"MediaBox"))
            .is_some_and(|(c, m)| c != m)
    {
        return Some("page-geometry");
    }
    let resources =
        resample_lookup(doc, page, b"Resources").and_then(|r| r.as_dict().ok().cloned());
    if resources.as_ref().is_some_and(|r| {
        r.get(b"Properties").is_ok()
            || resample_lookup(doc, r, b"ExtGState").is_some_and(|gs| {
                gs.as_dict().ok().is_some_and(|d| {
                    d.iter().any(|(_, v)| {
                        v.as_dict().ok().is_some_and(|g| {
                            g.get(b"BM").is_ok()
                                || g.get(b"ca").is_ok()
                                || g.get(b"CA").is_ok()
                                || g.get(b"SMask").is_ok()
                        })
                    })
                })
            })
    }) {
        return Some("layers-or-blend");
    }
    let parsed = match lopdf::content::Content::decode(contents) {
        Ok(p) => p,
        // Undecodable content cannot be classified: keep the original
        // objects rather than risk rasterizing live text.
        Err(_) => return Some("unreadable"),
    };
    let mut paths = 0usize;
    for op in parsed.operations {
        if PATH_BUILD_OPS.contains(&op.operator.as_bytes())
            || PATH_PAINT_OPS.contains(&op.operator.as_bytes())
        {
            paths += 1;
        } else if TEXT_OPS.contains(&op.operator.as_bytes()) {
            return Some("text");
        } else if UNCLEAR_OPS.contains(&op.operator.as_bytes()) {
            return Some("layered-content");
        } else if op.operator == "Do" {
            // A Do invocation may paint a Form XObject carrying its own
            // text or layered content — only evidence that it holds none
            // keeps the page eligible.
            let name = op
                .operands
                .first()
                .and_then(|o| o.as_name().ok().map(|n| n.to_vec()));
            let nested = name.and_then(|n| {
                resources
                    .as_ref()
                    .and_then(|r| resample_lookup(doc, r, b"XObject"))
                    .and_then(|x| x.as_dict().ok().cloned())
                    .and_then(|x| resample_lookup(doc, &x, &n))
                    .and_then(|o| match o {
                        Object::Stream(s) => Some(s),
                        _ => None,
                    })
            });
            match nested {
                Some(stream) => {
                    // Only a Form can carry operators; images are already
                    // pixels. Undecodable or layered form content keeps the
                    // page's original objects.
                    if stream
                        .dict
                        .get(b"Subtype")
                        .ok()
                        .is_some_and(|s| s.as_name().ok() == Some(&b"Form"[..]))
                    {
                        let bytes = stream
                            .decompressed_content()
                            .unwrap_or_else(|_| stream.content.clone());
                        match lopdf::content::Content::decode(&bytes) {
                            Err(_) => return Some("unreadable"),
                            Ok(inner) => {
                                for iop in inner.operations {
                                    if TEXT_OPS.contains(&iop.operator.as_bytes()) {
                                        return Some("text");
                                    }
                                    if UNCLEAR_OPS.contains(&iop.operator.as_bytes()) {
                                        return Some("layered-content");
                                    }
                                }
                            }
                        }
                    }
                }
                // Unresolvable XObject: cannot prove it carries no text.
                None => return Some("layered-content"),
            }
        }
    }
    if paths < TRACED_PATH_FLOOR {
        return Some("sparse-paths");
    }
    None
}

fn resample_lookup(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<Object> {
    dict.get_deref(key, doc).ok().cloned()
}

/// Page content stream bytes (decoded for evidence), the objects holding
/// them, and their stored on-disk size for file-size prediction.
fn page_contents(doc: &Document, page: &Dictionary) -> (Vec<u8>, Vec<Object>, u64) {
    let mut bytes = Vec::new();
    let mut refs = Vec::new();
    let mut stored = 0u64;
    // Read Contents unresolved so referenced streams are kept for removal.
    let contents = match page.get(b"Contents").ok().cloned() {
        Some(c) => c,
        None => return (bytes, refs, stored),
    };
    let mut items = Vec::new();
    match &contents {
        Object::Array(list) => items.extend(list.iter().cloned()),
        single => items.push(single.clone()),
    }
    for item in items {
        if let Object::Reference(id) = item {
            refs.push(item.clone());
            if let Ok(Object::Stream(s)) = doc.get_object(id) {
                stored += s.content.len() as u64;
                bytes.extend(
                    s.decompressed_content()
                        .unwrap_or_else(|_| s.content.clone()),
                );
            }
        } else if let Object::Stream(s) = &item {
            stored += s.content.len() as u64;
            bytes.extend(
                s.decompressed_content()
                    .unwrap_or_else(|_| s.content.clone()),
            );
        }
    }
    (bytes, refs, stored)
}

/// Rasterize eligible pages in place: each traced-graphics page becomes one
/// JPEG at the chosen rung while its MediaBox, rotation and page dict survive;
/// every other page and object stays byte-identical — photos, text, links and
/// uncertain content are never discarded. No fallback: render or assembly
/// failures propagate to the caller.
fn rasterize(document: &PdfDocument, request: &Value) -> Result<Value> {
    let input = request["input"]
        .as_str()
        .ok_or(Error("INVALID_REQUEST", None))?;
    let output = request["output"]
        .as_str()
        .ok_or(Error("INVALID_REQUEST", None))?;
    let rungs = request["rungs"]
        .as_array()
        .ok_or(Error("INVALID_REQUEST", None))?;
    if rungs.is_empty() || rungs.len() > MAX_RUNGS {
        return Err(Error("INVALID_REQUEST", None));
    }
    let mut ladder: Vec<(u32, u8)> = rungs
        .iter()
        .map(|r| {
            Ok((
                crate::request_integer(&r["dpiCap"])? as u32,
                crate::request_integer(&r["jpegQuality"])? as u8,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let target = request["targetBytes"]
        .as_u64()
        .ok_or(Error("INVALID_REQUEST", None))?;
    let budget = Duration::from_millis(request["budgetMillis"].as_u64().unwrap_or(0));
    let started = Instant::now();
    // Reserve 20% of the budget for the final apply+save, matching resample.
    let usable = budget.mul_f64(0.80);
    let spent = |now: Instant| now.duration_since(started) >= usable;

    let mut doc = Document::load(input).map_err(|_| Error("INVALID_PDF", None))?;
    let input_bytes = std::fs::metadata(input).map(|m| m.len()).unwrap_or(0);
    // Tagged structure and optional-content groups are document-level
    // protection rasterization would drop: bail entirely. Embedded files are
    // gated by the caller's inspect report.
    let tagged = doc
        .catalog()
        .ok()
        .map(|catalog| {
            catalog
                .get_deref(b"MarkInfo", &doc)
                .ok()
                .and_then(|mark| {
                    mark.as_dict()
                        .ok()
                        .and_then(|m| m.get_deref(b"Marked", &doc).ok())
                })
                .is_some_and(|v| matches!(v, Object::Boolean(true)))
                || catalog.get(b"StructTreeRoot").is_ok()
                || catalog.get(b"OCProperties").is_ok()
        })
        .unwrap_or(false);
    if tagged {
        return Ok(
            json!({"rasterizedPages": [], "skippedPages": [], "reason": "protected-structure"}),
        );
    }

    // Classify every page by positive evidence; each eligible page is
    // rendered once at the top rung and trial-encoded at every rung so rung
    // bytes are exact. Only one bitmap is held at a time — a whole document
    // of rendered pages would exceed the worker's memory cap.
    let page_ids = doc.get_pages();

    let mut eligible: Vec<(lopdf::ObjectId, i32, f64, f64, Vec<Object>, u64)> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut rung_bytes = vec![0u64; ladder.len()];
    let mut usable_rungs = vec![true; ladder.len()];
    let mut measured = true;
    let mut replaced_total = 0u64;
    // Streams referenced by a page that keeps its original objects must
    // survive rebuild; every other content stream is dropped with its page.
    let mut kept_refs: std::collections::HashSet<lopdf::ObjectId> =
        std::collections::HashSet::new();
    for (index, page_id) in &page_ids {
        let page_dict = match doc.get_dictionary(*page_id) {
            Ok(d) => d.clone(),
            Err(_) => {
                skipped.push(json!({"page": index, "reason": "unreadable"}));
                continue;
            }
        };
        let (contents, old_refs, stored) = page_contents(&doc, &page_dict);
        match traced_evidence(&doc, &page_dict, &contents) {
            Some(reason) => {
                skipped.push(json!({"page": index, "reason": reason}));
                for r in old_refs {
                    if let Object::Reference(id) = r {
                        kept_refs.insert(id);
                    }
                }
            }
            None => {
                if spent(Instant::now()) {
                    measured = false;
                    skipped.push(json!({"page": index, "reason": "deadline"}));
                    for r in old_refs {
                        if let Object::Reference(id) = r {
                            kept_refs.insert(id);
                        }
                    }
                    continue;
                }
                let page = document
                    .pages()
                    .get(i32::try_from(index - 1).unwrap_or(i32::MAX))
                    .map_err(|_| Error("INVALID_PDF", None))?;
                let w_pt = f64::from(page.width().value);
                let h_pt = f64::from(page.height().value);
                let pixels = required_pixels((w_pt, h_pt), ladder[0].0);
                let config = PdfRenderConfig::new()
                    .render_annotations(false)
                    .render_form_data(false)
                    .set_fixed_width(pixels.0 as i32)
                    .set_fixed_height(pixels.1 as i32)
                    .clear_before_rendering(true)
                    .set_clear_color(PdfColor::WHITE);
                let image = page
                    .render_with_config(&config)
                    .map_err(|_| Error("RENDER_FAILED", None))?
                    .as_image()
                    .map_err(|_| Error("INTERNAL", None))?;
                for (i, &(_, quality)) in ladder.iter().enumerate() {
                    let pixels = required_pixels((w_pt, h_pt), ladder[i].0);
                    let resized = resampled(&image, pixels.0, pixels.1);
                    let gray = achromatic_to_gray(resized);
                    match encode_jpeg(&gray, quality) {
                        Some(jpeg) => rung_bytes[i] += jpeg.len() as u64,
                        None => usable_rungs[i] = false,
                    }
                }
                replaced_total += stored;
                eligible.push((
                    *page_id,
                    i32::try_from(index - 1).unwrap_or(i32::MAX),
                    w_pt,
                    h_pt,
                    old_refs,
                    stored,
                ));
            }
        }
    }
    if eligible.is_empty() {
        return Ok(json!({
            "rasterizedPages": [],
            "skippedPages": skipped,
        }));
    }
    if !measured {
        return Err(Error("TIMEOUT", Some("measure".into())));
    }
    // Rung bytes count JPEG payloads only; predict assembled size as input
    // minus the content streams being replaced plus each rung's payloads.
    let mut predicted: Vec<u64> = rung_bytes
        .iter()
        .map(|b| input_bytes.saturating_sub(replaced_total) + b)
        .collect();
    let mut chosen = choose_rung(&predicted, &usable_rungs, target);
    // The strided raster ladder is coarser than the descent ladder: when the
    // chosen rung lands well under target, try the midpoint of it and its
    // gentler neighbor first — same surgery, better fidelity when it fits.
    if chosen > 0 && predicted[chosen] + target / 5 < target {
        let mid = (
            (ladder[chosen - 1].0 + ladder[chosen].0) / 2,
            (ladder[chosen - 1].1 + ladder[chosen].1) / 2,
        );
        ladder.insert(chosen, mid);
        usable_rungs.insert(chosen, true);
        rung_bytes.insert(chosen, (rung_bytes[chosen - 1] + rung_bytes[chosen]) / 2);
        predicted.insert(chosen, (predicted[chosen - 1] + predicted[chosen]) / 2);
    }

    // Assemble at the chosen rung, then measure the real file: the estimate
    // covers stream bytes, not object framing, so a rung that predicts near
    // the target can land over it. Descend until it fits or rungs run out.
    let mut bytes;
    let mut rasterized_pages = Vec::with_capacity(eligible.len());
    let mut last_bytes = u64::MAX;
    loop {
        let (dpi, quality) = ladder[chosen];
        rasterized_pages.clear();
        bytes = 0u64;
        for (page_id, index, w_pt, h_pt, old_refs, _) in &eligible {
            if spent(Instant::now()) {
                return Err(Error("TIMEOUT", Some("assemble".into())));
            }
            let page = document
                .pages()
                .get(*index)
                .map_err(|_| Error("INVALID_PDF", None))?;
            // Render at the top rung and downsample to the chosen one, exactly
            // as measurement did — native low-DPI renders of traced strokes
            // produce larger JPEGs than the measured path implies, which
            // would break the rung choice.
            let pixels = required_pixels((*w_pt, *h_pt), ladder[0].0);
            let config = PdfRenderConfig::new()
                .render_annotations(false)
                .render_form_data(false)
                .set_fixed_width(pixels.0 as i32)
                .set_fixed_height(pixels.1 as i32)
                .clear_before_rendering(true)
                .set_clear_color(PdfColor::WHITE);
            let image = page
                .render_with_config(&config)
                .map_err(|_| Error("RENDER_FAILED", None))?
                .as_image()
                .map_err(|_| Error("INTERNAL", None))?;
            let pixels = required_pixels((*w_pt, *h_pt), dpi);
            let resized = resampled(&image, pixels.0, pixels.1);
            let gray = achromatic_to_gray(resized);
            let jpeg = encode_jpeg(&gray, quality).ok_or(Error("INTERNAL_ERROR", None))?;
            bytes += jpeg.len() as u64;
            let (w_px, h_px) = (i64::from(gray.width()), i64::from(gray.height()));
            let colorspace = if matches!(gray, image::DynamicImage::ImageLuma8(_)) {
                b"DeviceGray".to_vec()
            } else {
                b"DeviceRGB".to_vec()
            };
            let image_id = doc.new_object_id();
            doc.objects.insert(
                image_id,
                Object::Stream(lopdf::Stream::new(
                    Dictionary::from_iter(vec![
                        (b"Type".to_vec(), Object::Name(b"XObject".to_vec())),
                        (b"Subtype".to_vec(), Object::Name(b"Image".to_vec())),
                        (b"Width".to_vec(), Object::Integer(w_px)),
                        (b"Height".to_vec(), Object::Integer(h_px)),
                        (b"ColorSpace".to_vec(), Object::Name(colorspace)),
                        (b"BitsPerComponent".to_vec(), Object::Integer(8)),
                        (b"Filter".to_vec(), Object::Name(b"DCTDecode".to_vec())),
                    ]),
                    jpeg,
                )),
            );
            let content = format!("q {w_pt} 0 0 {h_pt} 0 0 cm /Im0 Do Q\n").into_bytes();
            let content_id = doc.new_object_id();
            doc.objects.insert(
                content_id,
                Object::Stream(lopdf::Stream::new(
                    Dictionary::from_iter(vec![(
                        b"Length".to_vec(),
                        Object::Integer(content.len() as i64),
                    )]),
                    content,
                )),
            );
            // Replace only Contents and Resources on the existing page dict:
            // MediaBox, Rotate and outline destinations survive untouched. Old
            // content streams become unreferenced and are dropped on save.
            if let Ok(mut dict) = doc.get_dictionary(*page_id).cloned() {
                dict.set(b"Resources", {
                    Dictionary::from_iter(vec![(
                        b"XObject".to_vec(),
                        Object::Dictionary(Dictionary::from_iter(vec![(
                            b"Im0".to_vec(),
                            Object::Reference(image_id),
                        )])),
                    )])
                });
                dict.set(b"Contents", Object::Reference(content_id));
                doc.objects.insert(*page_id, Object::Dictionary(dict));
            }
            for item in old_refs {
                if let Object::Reference(id) = item {
                    // A stream a skipped page also paints must stay.
                    if !kept_refs.contains(id) {
                        doc.objects.remove(id);
                    }
                }
            }
            rasterized_pages.push(page_id.0);
        }
        let mut file = std::fs::File::create(output).map_err(|_| Error("INTERNAL_ERROR", None))?;
        doc.save_to(&mut file)
            .map_err(|_| Error("INTERNAL_ERROR", None))?;
        file.flush().ok();
        let file_bytes = std::fs::metadata(output).map(|m| m.len()).unwrap_or(bytes);
        if file_bytes <= target {
            return Ok(json!({
                "rasterizedPages": rasterized_pages,
                "skippedPages": skipped,
                "bytes": file_bytes,
                "rasterizedImageBytes": bytes,
                "chosenRung": chosen,
                "rungBytes": rung_bytes,
                "usableRungs": usable_rungs,
            }));
        }
        // Over target: step to the next usable harder rung. Each retry
        // reloads the document because the assemble pass mutates it.
        let next =
            (chosen + 1..ladder.len()).find(|&r| usable_rungs[r] && predicted[r] < last_bytes);
        match next {
            Some(r) if !spent(Instant::now()) => {
                chosen = r;
                last_bytes = file_bytes;
                doc = Document::load(input).map_err(|_| Error("INVALID_PDF", None))?;
            }
            _ => {
                return Ok(json!({
                    "rasterizedPages": rasterized_pages,
                    "skippedPages": skipped,
                    "bytes": file_bytes,
                    "rasterizedImageBytes": bytes,
                    "chosenRung": chosen,
                    "rungBytes": rung_bytes,
                    "usableRungs": usable_rungs,
                }));
            }
        }
    }
}

fn dimensions(width: f32, height: f32, dpi: u32) -> Result<(u32, u32)> {
    let scale = |points: f32| -> Result<u32> {
        let pixels = (f64::from(points) * f64::from(dpi) / 72.0).ceil();
        if !pixels.is_finite() || pixels <= 0.0 || pixels > f64::from(u32::MAX) {
            return Err(Error("LIMIT_EXCEEDED", None));
        }
        Ok(pixels as u32)
    };
    let (width, height) = (scale(width)?, scale(height)?);
    validate_render_dimensions(width, height)?;
    Ok((width, height))
}

fn span(text: String, bounds: PdfRect) -> Result<Option<Value>> {
    if text.is_empty() {
        return Ok(None);
    }
    let [left, bottom, right, top] = [
        bounds.left().value,
        bounds.bottom().value,
        bounds.right().value,
        bounds.top().value,
    ];
    if ![left, bottom, right, top].iter().all(|n| n.is_finite()) || left > right || bottom > top {
        return Err(Error("INVALID_PDF", None));
    }
    Ok(Some(
        json!({"text":text,"bounds":{"left":left,"bottom":bottom,"right":right,"top":top}}),
    ))
}

fn decode_text(values: impl IntoIterator<Item = u32>) -> String {
    char::decode_utf16(
        values
            .into_iter()
            .map(|value| u16::try_from(value).unwrap_or(0xfffd)),
    )
    .map(|value| value.unwrap_or(char::REPLACEMENT_CHARACTER))
    .collect()
}

fn process(library: &Path, request: &Value) -> Result<Value> {
    if !library.is_absolute() {
        return Err(Error("BACKEND_UNAVAILABLE", None));
    }
    let bindings =
        Pdfium::bind_to_library(library).map_err(|_| Error("BACKEND_UNAVAILABLE", None))?;
    let pdfium = Pdfium::new(bindings);
    let document = pdfium
        .load_pdf_from_file(
            request["input"]
                .as_str()
                .ok_or(Error("INVALID_REQUEST", None))?,
            None,
        )
        .map_err(|_| Error("INVALID_PDF", None))?;
    if request.get("rasterize").and_then(Value::as_bool) == Some(true) {
        return rasterize(&document, request);
    }
    let get_page = |number: u64| {
        let index = number
            .checked_sub(1)
            .and_then(|n| i32::try_from(n).ok())
            .ok_or(Error("INVALID_REQUEST", None))?;
        document
            .pages()
            .get(index)
            .map_err(|_| Error("INVALID_REQUEST", None))
    };
    if let Some(pages) = request["pages"].as_array() {
        let mut result = Vec::new();
        let mut budget = 0;
        for number in pages {
            let page = get_page(number.as_u64().ok_or(Error("INVALID_REQUEST", None))?)?;
            let text = page.text().map_err(|_| Error("INVALID_PDF", None))?;
            let mut spans = Vec::new();
            for segment in text.segments().iter() {
                if let Some(span) = span(segment.text(), segment.bounds())? {
                    budget += span.to_string().len();
                    if budget > MAX_TEXT_RESPONSE_BYTES / 2 || spans.len() >= 20_000 {
                        return Err(Error("LIMIT_EXCEEDED", None));
                    }
                    spans.push(span);
                }
            }
            let text = decode_text(text.chars().iter().map(|c| c.unicode_value()));
            budget += text.len();
            if budget > MAX_TEXT_RESPONSE_BYTES / 2 {
                return Err(Error("LIMIT_EXCEEDED", None));
            }
            result.push(json!({"page":number,"text":text,"spans":spans}));
        }
        Ok(json!(result))
    } else {
        let page = get_page(
            request["page"]
                .as_u64()
                .ok_or(Error("INVALID_REQUEST", None))?,
        )?;
        let dpi = request["dpi"]
            .as_u64()
            .filter(|n| (36..=300).contains(n))
            .ok_or(Error("INVALID_REQUEST", None))? as u32;
        let (width, height) = dimensions(page.width().value, page.height().value, dpi)?;
        let config = PdfRenderConfig::new()
            .render_annotations(false)
            .render_form_data(false)
            .set_fixed_width(width as i32)
            .set_fixed_height(height as i32)
            .clear_before_rendering(true)
            .set_clear_color(PdfColor::WHITE);
        page.render_with_config(&config)
            .map_err(|_| Error("INVALID_PDF", None))?
            .as_image()
            .map_err(|_| Error("INTERNAL", None))?
            .save_with_format(
                request["output"]
                    .as_str()
                    .ok_or(Error("INVALID_REQUEST", None))?,
                image::ImageFormat::Png,
            )
            .map_err(|_| Error("INTERNAL", None))?;
        Ok(Value::Null)
    }
}

/// Internal subprocess protocol; never emits native library diagnostics.
pub fn worker() -> i32 {
    let result = (|| {
        let mut args = std::env::args().skip(2);
        let library = args.next().ok_or(Error("INVALID_REQUEST", None))?;
        let request = args
            .next()
            .filter(|s| s.len() <= 65536)
            .ok_or(Error("INVALID_REQUEST", None))?;
        let request = serde_json::from_str(&request).map_err(|_| Error("INVALID_REQUEST", None))?;
        process(Path::new(&library), &request)
    })();
    let (value, code) = match result {
        Ok(value) => (value, 0),
        Err(error) => (json!({"error":error.0}), 1),
    };
    if serde_json::to_writer(std::io::stdout().lock(), &value).is_err() {
        return 1;
    }
    let _ = std::io::stdout().flush();
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_text_decodes_utf16_and_replaces_invalid_units() {
        assert_eq!(
            decode_text([0x41, 0x4e2d, 0xd83d, 0xde00, 0xd840, 0xdc00]),
            "A中\u{1f600}\u{20000}"
        );
        assert_eq!(decode_text([]), "");
        assert_eq!(
            decode_text([0xd83d, 0x41, 0xdc00, 0xd840]),
            "\u{fffd}A\u{fffd}\u{fffd}"
        );
        assert_eq!(decode_text([0x110000]), "\u{fffd}");
    }
    #[test]
    fn unavailable_library_is_canonical_and_cancellation_prevents_spawn() {
        assert_eq!(
            process(Path::new("/nonexistent/libpdfium.so"), &json!({}))
                .unwrap_err()
                .0,
            "BACKEND_UNAVAILABLE"
        );
        let provider = PdfiumProvider {
            library: "/nonexistent/libpdfium.so".into(),
            worker_executable: "/nonexistent/anpdf".into(),
        };
        let control = Control::default();
        control
            .cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            provider
                .extract_text(Path::new("input.pdf"), &[1], &control)
                .unwrap_err()
                .0,
            "TIMEOUT"
        );
    }
    #[test]
    fn explicit_worker_path_is_used_by_library_callers() {
        let provider = PdfiumProvider {
            library: "/nonexistent/libpdfium.so".into(),
            worker_executable: "/nonexistent/anpdf-worker".into(),
        };
        assert_eq!(
            provider
                .extract_text(Path::new("input.pdf"), &[1], &Control::default())
                .unwrap_err()
                .0,
            "BACKEND_UNAVAILABLE"
        );
    }
    #[test]
    fn pixel_dimensions_are_bounded_before_allocation() {
        assert_eq!(dimensions(300.0, 200.0, 144).unwrap(), (600, 400));
        assert_eq!(dimensions(1.1, 2.1, 72).unwrap(), (2, 3));
        for (w, h) in [(0.0, 1.0), (f32::NAN, 1.0), (9000.0, 1.0), (5000.0, 5000.0)] {
            assert_eq!(dimensions(w, h, 72).unwrap_err().0, "LIMIT_EXCEEDED");
        }
    }
    #[test]
    fn spans_preserve_unicode_and_pdf_coordinates() {
        let bounds = PdfRect::new_from_values(-2.0, -1.0, 9.0, 8.0);
        let value = span("中文".into(), bounds).unwrap().unwrap();
        assert_eq!(
            value,
            json!({"text":"中文","bounds":{"left":-1.0,"bottom":-2.0,"right":8.0,"top":9.0}})
        );
        assert!(span(String::new(), bounds).unwrap().is_none());
        assert!(
            span(
                "x".into(),
                PdfRect::new_from_values(f32::NAN, 0.0, 1.0, 1.0)
            )
            .is_err()
        );
    }
}
