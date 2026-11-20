//! Bounded image-resampling worker used by the compression size ladder.
//!
//! No permissively licensed third-party downsampler exists, so resampling is a
//! hidden `compress-worker` subcommand re-encoding oversized image XObjects in
//! place: lopdf rewrites the object model, `image` decodes/re-encodes and the
//! caller's qpdf pass applies the structural finish. Masked, multi-filter or
//! otherwise risky images are skipped, never rewritten.

use crate::{
    backend::{Control, ImageResampler, Qpdf},
    error::{Error, Result},
};
use lopdf::{Dictionary, Document, Object, ObjectId, content::Content};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
};

pub const WORKER_ARG: &str = "--internal-compress-worker";
const MAX_IMAGE_PIXELS: u64 = 64_000_000;
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const RESAMPLE_MARGIN: f64 = 1.10; // images within 10% of the cap are left alone
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Default)]
struct Placements {
    placed: HashMap<ObjectId, (f64, f64)>, // image id -> max placed size in points
    fallback: (f64, f64),                  // largest page box in points
    inline_images: u64,
}

fn num(object: &Object) -> f64 {
    match object {
        Object::Integer(i) => *i as f64,
        Object::Real(r) => *r as f64,
        _ => 0.0,
    }
}

fn name(object: &Object) -> Option<&[u8]> {
    match object {
        Object::Name(n) => Some(n),
        _ => None,
    }
}

fn lookup(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<Object> {
    dict.get_deref(key, doc).ok().cloned()
}

fn inherited(doc: &Document, dict: &Dictionary, key: &[u8], depth: usize) -> Option<Object> {
    if depth > 10 {
        return None;
    }
    if let Some(value) = lookup(doc, dict, key) {
        return Some(value);
    }
    let parent = dict.get(b"Parent").ok()?;
    let id = match parent {
        Object::Reference(id) => *id,
        _ => return None,
    };
    let parent_dict = doc.get_dictionary(id).ok()?;
    inherited(doc, parent_dict, key, depth + 1)
}

fn box_size(doc: &Document, page: &Dictionary) -> (f64, f64) {
    let media = match inherited(doc, page, b"MediaBox", 0) {
        Some(Object::Array(points)) if points.len() == 4 => points,
        _ => return (612.0, 792.0),
    };
    let w = (num(&media[2]) - num(&media[0])).abs();
    let h = (num(&media[3]) - num(&media[1])).abs();
    (w.max(1.0), h.max(1.0))
}

type Matrix = [f64; 6];

fn concat(left: Matrix, right: Matrix) -> Matrix {
    [
        left[0] * right[0] + left[1] * right[2],
        left[0] * right[1] + left[1] * right[3],
        left[2] * right[0] + left[3] * right[2],
        left[2] * right[1] + left[3] * right[3],
        left[4] * right[0] + left[5] * right[2] + right[4],
        left[4] * right[1] + left[5] * right[3] + right[5],
    ]
}

fn matrix(object: &Object) -> Option<Matrix> {
    let Object::Array(values) = object else {
        return None;
    };
    if values.len() != 6 {
        return None;
    }
    Some([
        num(&values[0]),
        num(&values[1]),
        num(&values[2]),
        num(&values[3]),
        num(&values[4]),
        num(&values[5]),
    ])
}

fn content_bytes(doc: &Document, page: &Dictionary) -> Vec<u8> {
    let mut bytes = Vec::new();
    let contents = match lookup(doc, page, b"Contents") {
        Some(c) => c,
        None => return bytes,
    };
    let mut streams = Vec::new();
    match contents {
        Object::Array(list) => streams.extend(list),
        single => streams.push(single),
    }
    for item in streams {
        let stream = match &item {
            Object::Stream(s) => Some(s),
            Object::Reference(id) => match doc.get_object(*id) {
                Ok(Object::Stream(s)) => Some(s),
                _ => None,
            },
            _ => None,
        };
        if let Some(stream) = stream {
            bytes.extend(
                stream
                    .decompressed_content()
                    .unwrap_or(stream.content.clone()),
            );
            bytes.push(b'\n');
        }
    }
    bytes
}

fn xobject<'a>(
    doc: &'a Document,
    resources: Option<&'a Dictionary>,
    op: &lopdf::content::Operation,
) -> Option<(ObjectId, &'a Object)> {
    let key = name(op.operands.first()?)?;
    let Object::Dictionary(map) = resources.and_then(|r| lookup(doc, r, b"XObject"))? else {
        return None;
    };
    let id = match map.get(key) {
        Ok(Object::Reference(id)) => *id,
        _ => return None,
    };
    doc.get_object(id).ok().map(|o| (id, o))
}

fn walk_ops(
    doc: &Document,
    ops: &[lopdf::content::Operation],
    resources: Option<&Dictionary>,
    state: &mut Vec<Matrix>,
    placements: &mut Placements,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    for op in ops {
        match op.operator.as_str() {
            "q" => state.push(*state.last().unwrap_or(&IDENTITY)),
            "Q" => {
                if state.len() > 1 {
                    state.pop();
                }
            }
            "cm" if op.operands.len() == 6 => {
                let m: Matrix = [
                    num(&op.operands[0]),
                    num(&op.operands[1]),
                    num(&op.operands[2]),
                    num(&op.operands[3]),
                    num(&op.operands[4]),
                    num(&op.operands[5]),
                ];
                let current = *state.last().unwrap_or(&IDENTITY);
                let combined = concat(m, current);
                if state.is_empty() {
                    state.push(combined);
                } else {
                    *state.last_mut().unwrap() = combined;
                }
            }
            "BI" => placements.inline_images += 1,
            "Do" => {
                let Some((id, object)) = xobject(doc, resources, op) else {
                    continue;
                };
                let Object::Stream(stream) = object else {
                    continue;
                };
                let ctm = *state.last().unwrap_or(&IDENTITY);
                match stream.dict.get(b"Subtype").ok().and_then(name) {
                    Some(b"Image") => {
                        let w = (ctm[0] * ctm[0] + ctm[1] * ctm[1]).sqrt();
                        let h = (ctm[2] * ctm[2] + ctm[3] * ctm[3]).sqrt();
                        placements
                            .placed
                            .entry(id)
                            .and_modify(|e| *e = (e.0.max(w), e.1.max(h)))
                            .or_insert((w, h));
                    }
                    Some(b"Form") => {
                        let form_ctm = lookup(doc, &stream.dict, b"Matrix")
                            .and_then(|m| matrix(&m))
                            .map(|fm| concat(fm, ctm))
                            .unwrap_or(ctm);
                        let form_resources = match lookup(doc, &stream.dict, b"Resources") {
                            Some(Object::Dictionary(r)) => Some(r.clone()),
                            _ => resources.cloned(),
                        };
                        let decoded = stream
                            .decompressed_content()
                            .unwrap_or(stream.content.clone());
                        if let Ok(form_content) = Content::decode(&decoded[..]) {
                            state.push(form_ctm);
                            walk_ops(
                                doc,
                                &form_content.operations,
                                form_resources.as_ref(),
                                state,
                                placements,
                                depth + 1,
                            );
                            state.pop();
                        }
                    }
                    _ => continue,
                }
            }
            _ => continue,
        }
    }
}

fn collect_placements(doc: &Document) -> Placements {
    let mut placements = Placements::default();
    for (_page_no, page_id) in doc.get_pages() {
        let page = match doc.get_dictionary(page_id) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let (w, h) = box_size(doc, page);
        placements.fallback = (placements.fallback.0.max(w), placements.fallback.1.max(h));
        let resources = match inherited(doc, page, b"Resources", 0) {
            Some(Object::Dictionary(r)) => Some(r),
            _ => None,
        };
        let bytes = content_bytes(doc, page);
        if let Ok(content) = Content::decode(&bytes[..]) {
            let mut state = vec![IDENTITY];
            walk_ops(
                doc,
                &content.operations,
                resources.as_ref(),
                &mut state,
                &mut placements,
                0,
            );
        }
    }
    placements
}

fn filters(dict: &Dictionary) -> Vec<Vec<u8>> {
    match dict.get(b"Filter") {
        Ok(Object::Name(n)) => vec![n.clone()],
        Ok(Object::Array(list)) => list
            .iter()
            .filter_map(|f| name(f).map(|n| n.to_vec()))
            .collect(),
        _ => Vec::new(),
    }
}

struct Palette {
    base_components: usize,
    data: Vec<u8>,
    high: usize,
}

fn indexed_palette(
    doc: &Document,
    dict: &Dictionary,
) -> std::result::Result<Option<Palette>, String> {
    let object = match dict.get(b"ColorSpace") {
        Ok(o) => o.clone(),
        Err(_) => return Err("missing colorspace".to_string()),
    };
    let resolved = match &object {
        Object::Reference(id) => doc
            .get_object(*id)
            .map_err(|_| "colorspace ref".to_string())?
            .clone(),
        o => o.clone(),
    };
    let Object::Array(spec) = &resolved else {
        return Ok(None);
    };
    if spec.len() < 4 || name(&spec[0]) != Some(b"Indexed") {
        return Ok(None);
    }
    let base_components = match name(&spec[1]) {
        Some(b"DeviceRGB") => 3,
        Some(b"DeviceGray") => 1,
        Some(b"DeviceCMYK") => 4,
        _ => return Err("indexed base".to_string()),
    };
    let high = num(&spec[2]).max(0.0) as usize;
    let data = match &spec[3] {
        Object::String(bytes, _) => bytes.clone(),
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::Stream(s)) => s.decompressed_content().unwrap_or(s.content.clone()),
            _ => return Err("palette ref".to_string()),
        },
        _ => return Err("palette kind".to_string()),
    };
    Ok(Some(Palette {
        base_components,
        data,
        high,
    }))
}

fn samples_image(
    dict: &Dictionary,
    bytes: &[u8],
    palette: Option<&Palette>,
) -> std::result::Result<image::DynamicImage, String> {
    let w = dict.get(b"Width").ok().map(num).unwrap_or(0.0) as u32;
    let h = dict.get(b"Height").ok().map(num).unwrap_or(0.0) as u32;
    if w == 0 || h == 0 || (w as u64) * (h as u64) > MAX_IMAGE_PIXELS {
        return Err("bounds".to_string());
    }
    let bpc = dict.get(b"BitsPerComponent").ok().map(num).unwrap_or(8.0) as u32;
    let (components, named) = match dict.get(b"ColorSpace") {
        Ok(Object::Name(n)) => (
            match n.as_slice() {
                b"DeviceGray" => 1usize,
                b"DeviceRGB" => 3,
                b"DeviceCMYK" => 4,
                _ => return Err("colorspace".to_string()),
            },
            Some(n.clone()),
        ),
        Ok(_) => match palette {
            Some(p) => (p.base_components, None),
            None => return Err("colorspace".to_string()),
        },
        Err(_) => (1, Some(b"DeviceGray".to_vec())),
    };
    let pixel_count = (w as usize) * (h as usize);
    let to_rgb = |channels: &[u8]| -> Vec<u8> {
        channels
            .chunks_exact(components)
            .flat_map(|px| match components {
                1 => vec![px[0], px[0], px[0]],
                3 => vec![px[0], px[1], px[2]],
                _ => {
                    let (c, m, y, k) = (px[0], px[1], px[2], px[3]);
                    vec![
                        255u8.saturating_sub(c.saturating_add(k)),
                        255u8.saturating_sub(m.saturating_add(k)),
                        255u8.saturating_sub(y.saturating_add(k)),
                    ]
                }
            })
            .collect()
    };
    let unpack = |bits: &[u8], index_bits: u32| -> Vec<u8> {
        let mask = (1u16 << index_bits) - 1;
        let mut out = Vec::with_capacity(pixel_count);
        let per_byte = 8 / index_bits;
        for byte in bits {
            for slot in 0..per_byte {
                let shift = 8 - index_bits * (slot + 1);
                out.push(((*byte as u16 >> shift) & mask) as u8);
                if out.len() == pixel_count {
                    return out;
                }
            }
        }
        out
    };
    let gray_image = |buf: Vec<u8>| -> std::result::Result<image::DynamicImage, String> {
        image::ImageBuffer::<image::Luma<u8>, Vec<u8>>::from_raw(w, h, buf)
            .map(image::DynamicImage::ImageLuma8)
            .ok_or_else(|| "gray buffer".to_string())
    };
    let rgb_image = |buf: Vec<u8>| -> std::result::Result<image::DynamicImage, String> {
        image::ImageBuffer::<image::Rgb<u8>, Vec<u8>>::from_raw(w, h, buf)
            .map(image::DynamicImage::ImageRgb8)
            .ok_or_else(|| "rgb buffer".to_string())
    };
    if let Some(p) = palette {
        if !matches!(bpc, 1 | 2 | 4 | 8) {
            return Err("indexed bpc".to_string());
        }
        let indices = if bpc == 8 {
            bytes.to_vec()
        } else {
            unpack(bytes, bpc)
        };
        if indices.len() < pixel_count {
            return Err("short stream".to_string());
        }
        let mut out = Vec::with_capacity(pixel_count * p.base_components);
        for index in indices.iter().take(pixel_count) {
            let i = (*index as usize).min(p.high) * p.base_components;
            if i + p.base_components > p.data.len() {
                return Err("palette range".to_string());
            }
            out.extend(&p.data[i..i + p.base_components]);
        }
        return if p.base_components == 1 {
            gray_image(out)
        } else {
            rgb_image(to_rgb(&out))
        };
    }
    match named.as_deref() {
        Some(b"DeviceGray") => {
            let buf = if bpc == 8 {
                if bytes.len() < pixel_count {
                    return Err("short stream".to_string());
                }
                bytes[..pixel_count].to_vec()
            } else if bpc == 1 {
                unpack(bytes, 1)
                    .iter()
                    .map(|b| if *b == 0 { 0 } else { 255 })
                    .collect()
            } else {
                return Err("gray bpc".to_string());
            };
            gray_image(buf)
        }
        Some(_) => {
            if bpc != 8 || bytes.len() < pixel_count * components {
                return Err("short stream".to_string());
            }
            rgb_image(to_rgb(&bytes[..pixel_count * components]))
        }
        None => Err("colorspace".to_string()),
    }
}

fn required_pixels(placed_pt: (f64, f64), dpi_cap: u32) -> (u32, u32) {
    (
        ((placed_pt.0 / 72.0) * dpi_cap as f64).ceil().max(1.0) as u32,
        ((placed_pt.1 / 72.0) * dpi_cap as f64).ceil().max(1.0) as u32,
    )
}

fn resampled(image: &image::DynamicImage, w: u32, h: u32) -> image::DynamicImage {
    match image {
        image::DynamicImage::ImageLuma8(l) => image::DynamicImage::ImageLuma8(
            image::imageops::resize(l, w, h, image::imageops::FilterType::Lanczos3),
        ),
        other => image::DynamicImage::ImageRgb8(image::imageops::resize(
            &other.to_rgb8(),
            w,
            h,
            image::imageops::FilterType::Lanczos3,
        )),
    }
}

fn resample_document(input: &Path, output: &Path, dpi_cap: u32, jpeg_quality: u8) -> Result<Value> {
    let mut doc = Document::load(input).map_err(|_| Error("INVALID_PDF"))?;
    let placements = collect_placements(&doc);
    let (fallback_w, fallback_h) = placements.fallback;
    let mut processed = 0u64;
    let mut skipped: Vec<String> = Vec::new();
    let mut image_bytes_in = 0u64;
    let mut image_bytes_out = 0u64;
    struct Edit {
        id: ObjectId,
        jpeg: Vec<u8>,
        width: u32,
        height: u32,
        gray: bool,
    }
    let mut edits: Vec<Edit> = Vec::new();
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        let stream = match doc.get_object(id) {
            Ok(Object::Stream(s)) => s,
            _ => continue,
        };
        if stream.dict.get(b"Subtype").ok().and_then(name) != Some(b"Image") {
            continue;
        }
        image_bytes_in += stream.content.len() as u64;
        let width = num(stream.dict.get(b"Width").unwrap_or(&Object::Null));
        let height = num(stream.dict.get(b"Height").unwrap_or(&Object::Null));
        let placed = placements
            .placed
            .get(&id)
            .copied()
            .unwrap_or((fallback_w, fallback_h));
        let (req_w, req_h) = required_pixels(placed, dpi_cap);
        if width <= req_w as f64 * RESAMPLE_MARGIN && height <= req_h as f64 * RESAMPLE_MARGIN {
            skipped.push(format!("{}:{}:below-cap", id.0, id.1));
            continue;
        }
        if stream.dict.get(b"ImageMask").ok() == Some(&Object::Boolean(true))
            || stream.dict.get(b"Mask").is_ok()
            || stream.dict.get(b"SMask").is_ok()
            || stream.dict.get(b"Alternates").is_ok()
            || stream.dict.get(b"Decode").is_ok()
            || stream.dict.get(b"DecodeParms").is_ok()
            || stream.dict.get(b"DP").is_ok()
        {
            skipped.push(format!("{}:{}:masked-or-decoded", id.0, id.1));
            continue;
        }
        let filter_list = filters(&stream.dict);
        let decoded = match filter_list.as_slice() {
            [f] if f == b"DCTDecode" => {
                match image::load_from_memory_with_format(&stream.content, image::ImageFormat::Jpeg)
                {
                    Ok(img) => Some(img),
                    Err(_) => {
                        skipped.push(format!("{}:{}:jpeg-decode", id.0, id.1));
                        continue;
                    }
                }
            }
            [f] if f == b"FlateDecode" || f == b"Fl" => {
                let mut flat = Vec::new();
                match flate2::read::ZlibDecoder::new(stream.content.as_slice())
                    .read_to_end(&mut flat)
                {
                    Ok(_) => {}
                    Err(_) => {
                        skipped.push(format!("{}:{}:flate-decode", id.0, id.1));
                        continue;
                    }
                }
                let palette = match indexed_palette(&doc, &stream.dict) {
                    Ok(p) => p,
                    Err(reason) => {
                        skipped.push(format!("{}:{}:{reason}", id.0, id.1));
                        continue;
                    }
                };
                match samples_image(&stream.dict, &flat, palette.as_ref()) {
                    Ok(img) => Some(img),
                    Err(reason) => {
                        skipped.push(format!("{}:{}:{reason}", id.0, id.1));
                        continue;
                    }
                }
            }
            [] => {
                let palette = match indexed_palette(&doc, &stream.dict) {
                    Ok(p) => p,
                    Err(reason) => {
                        skipped.push(format!("{}:{}:{reason}", id.0, id.1));
                        continue;
                    }
                };
                match samples_image(&stream.dict, &stream.content, palette.as_ref()) {
                    Ok(img) => Some(img),
                    Err(reason) => {
                        skipped.push(format!("{}:{}:{reason}", id.0, id.1));
                        continue;
                    }
                }
            }
            _ => {
                skipped.push(format!("{}:{}:filters", id.0, id.1));
                continue;
            }
        };
        let Some(image) = decoded else {
            continue;
        };
        let (px_w, px_h) = (image.width(), image.height());
        let scale = ((req_w as f64) / (px_w as f64))
            .min((req_h as f64) / (px_h as f64))
            .min(1.0);
        let new_w = ((px_w as f64) * scale).floor().max(1.0) as u32;
        let new_h = ((px_h as f64) * scale).floor().max(1.0) as u32;
        let resized = resampled(&image, new_w, new_h);
        let mut jpeg = Vec::new();
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, jpeg_quality);
        if encoder.encode_image(&resized).is_err() {
            skipped.push(format!("{}:{}:jpeg-encode", id.0, id.1));
            continue;
        }
        let gray = matches!(
            resized,
            image::DynamicImage::ImageLuma8(_) | image::DynamicImage::ImageLumaA8(_)
        );
        edits.push(Edit {
            id,
            jpeg,
            width: new_w,
            height: new_h,
            gray,
        });
    }
    for edit in edits {
        let stream = match doc.get_object_mut(edit.id) {
            Ok(Object::Stream(s)) => s,
            _ => continue,
        };
        stream.set_plain_content(edit.jpeg.clone());
        stream
            .dict
            .set(b"Filter".to_vec(), Object::Name(b"DCTDecode".to_vec()));
        stream
            .dict
            .set(b"Width".to_vec(), Object::Integer(edit.width as i64));
        stream
            .dict
            .set(b"Height".to_vec(), Object::Integer(edit.height as i64));
        stream
            .dict
            .set(b"BitsPerComponent".to_vec(), Object::Integer(8));
        stream.dict.set(
            b"ColorSpace".to_vec(),
            Object::Name(if edit.gray {
                b"DeviceGray".to_vec()
            } else {
                b"DeviceRGB".to_vec()
            }),
        );
        stream.dict.remove(b"DecodeParms");
        image_bytes_out += edit.jpeg.len() as u64;
        processed += 1;
    }
    doc.save_to(&mut std::fs::File::create(output).map_err(|_| Error("INTERNAL_ERROR"))?)
        .map_err(|_| Error("INTERNAL_ERROR"))?;
    let input_bytes = std::fs::metadata(input).map(|m| m.len()).unwrap_or(0);
    Ok(json!({
        "imagesProcessed": processed,
        "imagesSkipped": skipped,
        "inlineImages": placements.inline_images,
        "imageBytesIn": image_bytes_in,
        "imageBytesOut": image_bytes_out,
        "nonImageBytes": input_bytes.saturating_sub(image_bytes_in),
    }))
}

pub struct ResampleProvider {
    pub worker_executable: PathBuf,
}

impl ImageResampler for ResampleProvider {
    fn resample_images(
        &self,
        input: &Path,
        output: &Path,
        dpi_cap: u32,
        jpeg_quality: u8,
        control: &Control,
    ) -> Result<Value> {
        let request = json!({
            "input": input.to_string_lossy(),
            "output": output.to_string_lossy(),
            "dpiCap": dpi_cap,
            "jpegQuality": jpeg_quality,
        });
        let (_code, stdout) = Qpdf {
            executable: self.worker_executable.clone(),
            exact: false,
        }
        .run(&[WORKER_ARG.to_string(), request.to_string()], control)?;
        let summary: Value =
            serde_json::from_slice(&stdout).map_err(|_| Error("INTERNAL_ERROR"))?;
        if let Some(code) = summary.get("error").and_then(Value::as_str) {
            return Err(match code {
                "INVALID_PDF" => Error("INVALID_PDF"),
                _ => Error("INTERNAL_ERROR"),
            });
        }
        Ok(summary)
    }
}

fn process(arguments: &[OsString]) -> Result<Value> {
    let request_text = arguments
        .first()
        .map(|a| a.to_string_lossy().into_owned())
        .ok_or(Error("INVALID_REQUEST"))?;
    if request_text.len() > MAX_REQUEST_BYTES {
        return Err(Error("INVALID_REQUEST"));
    }
    let request: Value =
        serde_json::from_str(&request_text).map_err(|_| Error("INVALID_REQUEST"))?;
    let input = request
        .get("input")
        .and_then(Value::as_str)
        .ok_or(Error("INVALID_REQUEST"))?;
    let output = request
        .get("output")
        .and_then(Value::as_str)
        .ok_or(Error("INVALID_REQUEST"))?;
    let dpi = request
        .get("dpiCap")
        .and_then(Value::as_u64)
        .filter(|d| (36..=600).contains(d))
        .ok_or(Error("INVALID_REQUEST"))? as u32;
    let quality = request
        .get("jpegQuality")
        .and_then(Value::as_u64)
        .filter(|q| *q <= 100)
        .unwrap_or(70) as u8;
    resample_document(Path::new(input), Path::new(output), dpi, quality)
}

pub fn worker(args: impl Iterator<Item = OsString>) -> i32 {
    let arguments: Vec<OsString> = args.collect();
    match process(&arguments) {
        Ok(result) => {
            println!("{result}");
            0
        }
        Err(error) => {
            println!("{}", json!({"error": error.0}));
            1
        }
    }
}
