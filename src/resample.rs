//! Bounded image-resampling worker used by the compression size ladder.
//!
//! No permissively licensed third-party downsampler exists, so resampling is a
//! hidden `compress-worker` subcommand re-encoding oversized image XObjects in
//! place: lopdf rewrites the object model, `image` decodes/re-encodes and the
//! caller's qpdf pass applies the structural finish. Masked, multi-filter or
//! otherwise risky images are skipped, never rewritten.
//!
//! One invocation covers the whole search: each image is decoded once, then
//! re-encoded at every requested rung so the caller gets exact per-rung byte
//! totals — the gentlest rung whose estimated output fits `targetBytes` is
//! applied and saved in a single pass, instead of re-running the document per
//! step. Per-image work fans out over bounded threads; decoded pixels in
//! flight and retained JPEG bytes are capped so large documents stay inside
//! the worker's address limit.

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
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub const WORKER_ARG: &str = "--internal-compress-worker";
const MAX_IMAGE_PIXELS: u64 = 64_000_000;
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_CONTENT_BYTES: usize = 32 * 1024 * 1024;
const MAX_PALETTE_RAW_BYTES: usize = 4 * 1024 * 1024;
const MAX_RUNGS: usize = 16;
const RESAMPLE_MARGIN: f64 = 1.10; // images within 10% of the cap are left alone
/// Decoded pixels held at once across worker threads (~256 MiB).
/// Decoded pixels in flight stay under this bound so the worker's address
/// space fits its sandbox (RLIMIT_AS) with room for retained JPEGs.
const PIXEL_BUDGET: u64 = 96 * 1024 * 1024;
/// Encoded JPEG bytes retained for the apply pass (~192 MiB); images past the
/// cap are re-decoded once for the chosen rung instead.
/// Retained per-rung JPEG bytes; beyond it the chosen rung is re-encoded at
/// apply time rather than held in memory.
const JPEG_BUDGET: u64 = 96 * 1024 * 1024;
const MAX_WORKERS: usize = 8;
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
            let decoded = stream
                .decompressed_content()
                .unwrap_or(stream.content.clone());
            if decoded.len() > MAX_CONTENT_BYTES {
                continue;
            }
            if bytes.len() + decoded.len() > MAX_CONTENT_BYTES * 4 {
                break;
            }
            bytes.extend(decoded);
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
                        if decoded.len() > MAX_CONTENT_BYTES {
                            continue;
                        }
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
    let expected = (high + 1) * base_components;
    if expected == 0 || expected > 4096 {
        return Err("palette high".to_string());
    }
    let mut data = match &spec[3] {
        Object::String(bytes, _) => bytes.clone(),
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::Stream(s)) => {
                if s.content.len() > MAX_PALETTE_RAW_BYTES {
                    return Err("palette bounds".to_string());
                }
                s.decompressed_content().unwrap_or(s.content.clone())
            }
            _ => return Err("palette ref".to_string()),
        },
        _ => return Err("palette kind".to_string()),
    };
    if data.len() > expected {
        data.truncate(expected);
    }
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
    // Packed rows pad to a byte boundary; unpack per row so the padding
    // bits are not read as the next row's leading pixels.
    let unpack = |bits: &[u8], index_bits: u32| -> Vec<u8> {
        let mask = (1u16 << index_bits) - 1;
        let mut out = Vec::with_capacity(pixel_count);
        let per_byte = 8 / index_bits;
        let row_pixels = w as usize;
        let row_bytes = (row_pixels * index_bits as usize).div_ceil(8);
        for row in 0..h as usize {
            let start = row * row_bytes;
            if start >= bits.len() {
                break;
            }
            let mut produced = 0usize;
            for byte in &bits[start..(start + row_bytes).min(bits.len())] {
                for slot in 0..per_byte {
                    if produced == row_pixels {
                        break;
                    }
                    let shift = 8 - index_bits * (slot + 1);
                    out.push(((*byte as u16 >> shift) & mask) as u8);
                    produced += 1;
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

/// An RGB image whose channels never diverge is achromatic: re-encode it as
/// grayscale so the JPEG drops its chroma planes and the rewritten ColorSpace
/// stays honest. Scanned pages are often stored RGB despite holding
/// monochrome content.
fn achromatic_to_gray(resized: image::DynamicImage) -> image::DynamicImage {
    if let image::DynamicImage::ImageRgb8(rgb) = &resized {
        let achromatic = rgb.pixels().all(|p| {
            let [r, g, b] = p.0;
            r.abs_diff(g) <= 8 && g.abs_diff(b) <= 8 && r.abs_diff(b) <= 8
        });
        if achromatic {
            return image::DynamicImage::ImageLuma8(resized.to_luma8());
        }
    }
    resized
}

fn encode_jpeg(resized: &image::DynamicImage, jpeg_quality: u8) -> Option<Vec<u8>> {
    let mut jpeg = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, jpeg_quality);
    // Encode the concrete buffer, not the DynamicImage: a DynamicImage
    // reports RGBA pixels to the encoder, which would emit a 3-component
    // JPEG under a DeviceGray label.
    let encoded = match resized {
        image::DynamicImage::ImageLuma8(l) => encoder.encode(
            l.as_raw(),
            l.width(),
            l.height(),
            image::ExtendedColorType::L8,
        ),
        _ => {
            let rgb = resized.to_rgb8();
            encoder.encode(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
        }
    };
    encoded.ok().map(|_| jpeg)
}

/// Outcome for one image at one rung: below cap (keep the original stream),
/// encoded (exact JPEG bytes and rewritten dimensions), or unmeasured because
/// the worker's time budget ran out first.
enum RungOut {
    BelowCap,
    Encoded {
        jpeg: Option<Vec<u8>>,
        len: u64,
        width: u32,
        height: u32,
        gray: bool,
    },
    Unmeasured,
}

/// Resize `image` to `placed` points at `dpi_cap` and encode it. Returns
/// `BelowCap` when the image already sits at or below the rung's cap.
fn resample_at(
    image: &image::DynamicImage,
    placed: (f64, f64),
    dpi_cap: u32,
    jpeg_quality: u8,
) -> RungOut {
    let (req_w, req_h) = required_pixels(placed, dpi_cap);
    if (image.width() as f64) <= req_w as f64 * RESAMPLE_MARGIN
        && (image.height() as f64) <= req_h as f64 * RESAMPLE_MARGIN
    {
        return RungOut::BelowCap;
    }
    let scale = ((req_w as f64) / (image.width() as f64))
        .min((req_h as f64) / (image.height() as f64))
        .min(1.0);
    let new_w = ((image.width() as f64) * scale).floor().max(1.0) as u32;
    let new_h = ((image.height() as f64) * scale).floor().max(1.0) as u32;
    let resized = achromatic_to_gray(resampled(image, new_w, new_h));
    let gray = matches!(
        resized,
        image::DynamicImage::ImageLuma8(_) | image::DynamicImage::ImageLumaA8(_)
    );
    match encode_jpeg(&resized, jpeg_quality) {
        Some(jpeg) => RungOut::Encoded {
            len: jpeg.len() as u64,
            jpeg: Some(jpeg),
            width: new_w,
            height: new_h,
            gray,
        },
        None => RungOut::Unmeasured,
    }
}

/// Gentle-most rung whose estimated document bytes fit `target`; falls back
/// to the smallest measured rung so a miss still yields the tightest
/// achievable output. `rung_bytes` are exact image-stream totals plus
/// measured non-image bytes — no monotonicity assumption, so a rung whose
/// images stayed below cap cannot hide a gentler fit. Rungs not fully
/// measured (budget stop) are excluded via `usable`.
pub(crate) fn choose_rung(rung_bytes: &[u64], usable: &[bool], target: u64) -> usize {
    let mut best: Option<usize> = None;
    for (r, (&bytes, &ok)) in rung_bytes.iter().zip(usable).enumerate() {
        if ok && bytes <= target {
            return r;
        }
        if ok {
            match best {
                Some(b) if bytes >= rung_bytes[b] => {}
                _ => best = Some(r),
            }
        }
    }
    best.unwrap_or(rung_bytes.len() - 1)
}

/// Decode one image stream to pixels, or a skip reason. The same unsafe
/// sources that used to stay untouched are skipped identically here and count
/// their original bytes toward every rung's total.
fn decode_image(
    doc: &Document,
    stream: &lopdf::Stream,
) -> std::result::Result<image::DynamicImage, String> {
    let width = num(stream.dict.get(b"Width").unwrap_or(&Object::Null));
    let height = num(stream.dict.get(b"Height").unwrap_or(&Object::Null));
    if stream.dict.get(b"ImageMask").ok() == Some(&Object::Boolean(true))
        || stream.dict.get(b"Mask").is_ok()
        || stream.dict.get(b"SMask").is_ok()
        || stream.dict.get(b"Alternates").is_ok()
        || stream.dict.get(b"Decode").is_ok()
        || stream.dict.get(b"DecodeParms").is_ok()
        || stream.dict.get(b"DP").is_ok()
    {
        return Err("masked-or-decoded".to_string());
    }
    // Reject dimensions that would decode past the pixel budget before
    // touching stream bytes, so a hostile Flate stream cannot inflate.
    if width <= 0.0 || height <= 0.0 || (width as u64) * (height as u64) > MAX_IMAGE_PIXELS {
        return Err("bounds".to_string());
    }
    let decode_bound = (width as u64) * (height as u64) * 4 + 1024;
    match filters(&stream.dict).as_slice() {
        [f] if f == b"DCTDecode" => {
            // Re-encoding rewrites the color space: only plain Device
            // spaces can be relabeled without losing calibration.
            match stream.dict.get(b"ColorSpace") {
                Ok(Object::Name(n)) if matches!(n.as_slice(), b"DeviceGray" | b"DeviceRGB") => {}
                Err(_) => {}
                _ => return Err("colorspace".to_string()),
            }
            image::load_from_memory_with_format(&stream.content, image::ImageFormat::Jpeg)
                .map_err(|_| "jpeg-decode".to_string())
        }
        [f] if f == b"FlateDecode" || f == b"Fl" => {
            let mut flat = Vec::new();
            flate2::read::ZlibDecoder::new(stream.content.as_slice())
                .take(decode_bound)
                .read_to_end(&mut flat)
                .map_err(|_| "flate-decode".to_string())?;
            let palette = indexed_palette(doc, &stream.dict)?;
            samples_image(&stream.dict, &flat, palette.as_ref())
        }
        [] => {
            let palette = indexed_palette(doc, &stream.dict)?;
            samples_image(&stream.dict, &stream.content, palette.as_ref())
        }
        _ => Err("filters".to_string()),
    }
}

/// Claim work while in-flight decoded pixels stay under `PIXEL_BUDGET`; an
/// item larger than the budget runs alone.
struct Claim {
    next: AtomicUsize,
    inflight: AtomicU64,
}
impl Claim {
    fn take(&self, pixel_sizes: &[u64]) -> Option<usize> {
        loop {
            let idx = self.next.load(Ordering::Relaxed);
            if idx >= pixel_sizes.len() {
                return None;
            }
            if self.inflight.load(Ordering::Relaxed) > 0
                && self.inflight.load(Ordering::Relaxed) + pixel_sizes[idx] > PIXEL_BUDGET
            {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            if self
                .next
                .compare_exchange(idx, idx + 1, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                self.inflight.fetch_add(pixel_sizes[idx], Ordering::Relaxed);
                return Some(idx);
            }
        }
    }
    fn done(&self, bytes: u64) {
        self.inflight.fetch_sub(bytes, Ordering::Relaxed);
    }
}

/// One resample worker invocation: classify streams, decode each measurable
/// image once, encode it at every rung (bounded parallel), pick the gentlest
/// rung fitting `target` from exact byte totals, apply it, and save once.
fn resample_document(
    input: &Path,
    output: &Path,
    rungs: &[(u32, u8)],
    target: Option<u64>,
    budget: Option<Duration>,
) -> Result<Value> {
    let started = Instant::now();
    // Measurement stops at 80% of the budget so applying and saving a fully
    // measured rung still fits the worker's deadline.
    let spent =
        || budget.is_some_and(|b| started.elapsed() > b.mul_f64(0.8).max(Duration::from_secs(0)));
    let mut doc = Document::load(input).map_err(|_| Error("INVALID_PDF", None))?;
    let placements = collect_placements(&doc);
    let (fallback_w, fallback_h) = placements.fallback;
    let mut skipped: Vec<String> = Vec::new();
    let mut image_bytes_in = 0u64;

    struct Pending {
        id: ObjectId,
        stream_len: u64,
        pixel_bytes: u64,
        placed: (f64, f64),
        stream: lopdf::Stream,
    }
    let mut pending: Vec<Pending> = Vec::new();
    let mut fixed: Vec<u64> = Vec::new(); // original stream lens held at every rung
    for (&id, object) in doc.objects.iter() {
        let stream = match object {
            Object::Stream(s) => s,
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
        // Below every rung's cap: the original stream stays untouched.
        let below_all = rungs.iter().all(|(cap, _)| {
            let (req_w, req_h) = required_pixels(placed, *cap);
            width <= req_w as f64 * RESAMPLE_MARGIN && height <= req_h as f64 * RESAMPLE_MARGIN
        });
        if below_all {
            skipped.push(format!("{}:{}:below-cap", id.0, id.1));
            fixed.push(stream.content.len() as u64);
            continue;
        }
        pending.push(Pending {
            id,
            stream_len: stream.content.len() as u64,
            pixel_bytes: ((width.max(1.0) as u64) * (height.max(1.0) as u64) * 4).min(PIXEL_BUDGET),
            placed,
            stream: stream.clone(),
        });
    }

    /// Per-image measured state.
    struct Measured {
        id: ObjectId,
        stream_len: u64,
        placed: (f64, f64),
        stream: lopdf::Stream,
        rungs: Vec<RungOut>,
    }
    let pixel_sizes: Vec<u64> = pending.iter().map(|p| p.pixel_bytes).collect();
    let claim = Claim {
        next: AtomicUsize::new(0),
        inflight: AtomicU64::new(0),
    };
    let jpeg_retained = AtomicU64::new(0);
    type Outcome = (usize, std::result::Result<(), String>, Measured);
    let results: Mutex<Vec<Outcome>> = Mutex::new(Vec::new());
    let workers = pending
        .len()
        .min(MAX_WORKERS)
        .min(std::thread::available_parallelism().map_or(4, |n| n.get()));
    if !pending.is_empty() {
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    while let Some(idx) = claim.take(&pixel_sizes) {
                        let p = &pending[idx];
                        let mut entry = Measured {
                            id: p.id,
                            stream_len: p.stream_len,
                            placed: p.placed,
                            stream: p.stream.clone(),
                            rungs: Vec::with_capacity(rungs.len()),
                        };
                        match decode_image(&doc, &p.stream) {
                            Err(reason) => {
                                results.lock().unwrap().push((idx, Err(reason), entry));
                                claim.done(p.pixel_bytes);
                                continue;
                            }
                            Ok(image) => {
                                entry.stream = p.stream.clone();
                                for &(cap, quality) in rungs {
                                    if spent() {
                                        entry.rungs.push(RungOut::Unmeasured);
                                        continue;
                                    }
                                    match resample_at(&image, p.placed, cap, quality) {
                                        RungOut::Encoded {
                                            jpeg,
                                            len,
                                            width,
                                            height,
                                            gray,
                                        } => {
                                            let keep = jpeg.is_some()
                                                && jpeg_retained.fetch_add(len, Ordering::Relaxed)
                                                    + len
                                                    <= JPEG_BUDGET;
                                            entry.rungs.push(RungOut::Encoded {
                                                jpeg: if keep { jpeg } else { None },
                                                len,
                                                width,
                                                height,
                                                gray,
                                            });
                                        }
                                        other => entry.rungs.push(other),
                                    }
                                }
                                results.lock().unwrap().push((idx, Ok(()), entry));
                            }
                        }
                        claim.done(p.pixel_bytes);
                    }
                });
            }
        });
    }
    let mut measured: Vec<Measured> = Vec::with_capacity(pending.len());
    {
        let mut results = results.into_inner().unwrap();
        results.sort_by_key(|(idx, _, _)| *idx);
        for (_idx, outcome, entry) in results {
            match outcome {
                Err(reason) => {
                    skipped.push(format!("{}:{}:{reason}", entry.id.0, entry.id.1));
                    fixed.push(entry.stream_len);
                }
                Ok(()) => measured.push(entry),
            }
        }
    }

    let input_bytes = std::fs::metadata(input).map(|m| m.len()).unwrap_or(0);
    let non_image_bytes = input_bytes.saturating_sub(image_bytes_in);
    let mut rung_bytes: Vec<u64> = vec![non_image_bytes + fixed.iter().sum::<u64>(); rungs.len()];
    let mut usable = vec![true; rungs.len()];
    for m in &measured {
        for (r, slot) in m.rungs.iter().enumerate() {
            match slot {
                RungOut::BelowCap => rung_bytes[r] += m.stream_len,
                RungOut::Encoded { len, .. } => rung_bytes[r] += len,
                RungOut::Unmeasured => usable[r] = false,
            }
        }
    }
    let chosen = target
        .map(|t| choose_rung(&rung_bytes, &usable, t))
        .unwrap_or_else(|| {
            // No target: the last rung is the request (level-pinned ladder).
            let mut r = rungs.len() - 1;
            while r > 0 && !usable[r] {
                r -= 1;
            }
            r
        });
    if !usable[chosen] {
        return Err(Error("TIMEOUT", None));
    }

    // Apply the chosen rung. JPEGs dropped under the retention cap are
    // re-encoded from a fresh decode of the source stream.
    let mut edits: Vec<(ObjectId, Vec<u8>, u32, u32, bool)> = Vec::new();
    let mut processed = 0u64;
    let mut image_bytes_out = 0u64;
    for m in &measured {
        let (cap, quality) = rungs[chosen];
        let out = match &m.rungs[chosen] {
            RungOut::BelowCap => None,
            RungOut::Encoded {
                jpeg,
                width,
                height,
                gray,
                ..
            } => match jpeg {
                Some(j) => Some((j.clone(), *width, *height, *gray)),
                None => decode_image(&doc, &m.stream).ok().and_then(|i| {
                    match resample_at(&i, m.placed, cap, quality) {
                        RungOut::Encoded {
                            jpeg: Some(j),
                            width,
                            height,
                            gray,
                            ..
                        } => Some((j, width, height, gray)),
                        _ => None,
                    }
                }),
            },
            RungOut::Unmeasured => None,
        };
        let Some((jpeg, width, height, gray)) = out else {
            continue;
        };
        edits.push((m.id, jpeg, width, height, gray));
    }
    for (id, jpeg, width, height, gray) in edits {
        let stream = match doc.get_object_mut(id) {
            Ok(Object::Stream(s)) => s,
            _ => continue,
        };
        stream.set_plain_content(jpeg);
        stream
            .dict
            .set(b"Filter".to_vec(), Object::Name(b"DCTDecode".to_vec()));
        stream
            .dict
            .set(b"Width".to_vec(), Object::Integer(width as i64));
        stream
            .dict
            .set(b"Height".to_vec(), Object::Integer(height as i64));
        stream
            .dict
            .set(b"BitsPerComponent".to_vec(), Object::Integer(8));
        stream.dict.set(
            b"ColorSpace".to_vec(),
            Object::Name(if gray {
                b"DeviceGray".to_vec()
            } else {
                b"DeviceRGB".to_vec()
            }),
        );
        stream.dict.remove(b"DecodeParms");
        image_bytes_out += stream.content.len() as u64;
        processed += 1;
    }
    doc.save_to(&mut std::fs::File::create(output).map_err(|_| Error("INTERNAL_ERROR", None))?)
        .map_err(|_| Error("INTERNAL_ERROR", None))?;
    Ok(json!({
        "imagesProcessed": processed,
        "imagesSkipped": skipped,
        "inlineImages": placements.inline_images,
        "imageBytesIn": image_bytes_in,
        "imageBytesOut": image_bytes_out,
        "nonImageBytes": non_image_bytes,
        "rungBytes": rung_bytes,
        "chosenRung": chosen,
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
        rungs: &[(u32, u8)],
        target: Option<u64>,
        control: &Control,
    ) -> Result<Value> {
        let request = json!({
            "input": input.to_string_lossy(),
            "output": output.to_string_lossy(),
            "rungs": rungs.iter().map(|(d, q)| json!({"dpiCap": d, "jpegQuality": q})).collect::<Vec<_>>(),
            "targetBytes": target,
            "budgetMillis": control.remaining().as_millis() as u64,
        });
        let (_code, stdout) = Qpdf {
            executable: self.worker_executable.clone(),
            exact: false,
            as_limit: 1536 * 1024 * 1024,
            cpu_scale: MAX_WORKERS as u64,
        }
        .run(&[WORKER_ARG.to_string(), request.to_string()], control)?;
        let summary: Value =
            serde_json::from_slice(&stdout).map_err(|_| Error("INTERNAL_ERROR", None))?;
        if let Some(code) = summary.get("error").and_then(Value::as_str) {
            return Err(match code {
                "INVALID_PDF" => Error("INVALID_PDF", None),
                _ => Error("INTERNAL_ERROR", None),
            });
        }
        Ok(summary)
    }
}

fn process(arguments: &[OsString]) -> Result<Value> {
    let request_text = arguments
        .first()
        .map(|a| a.to_string_lossy().into_owned())
        .ok_or(Error("INVALID_REQUEST", None))?;
    if request_text.len() > MAX_REQUEST_BYTES {
        return Err(Error("INVALID_REQUEST", None));
    }
    let request: Value =
        serde_json::from_str(&request_text).map_err(|_| Error("INVALID_REQUEST", None))?;
    let input = request
        .get("input")
        .and_then(Value::as_str)
        .ok_or(Error("INVALID_REQUEST", None))?;
    let output = request
        .get("output")
        .and_then(Value::as_str)
        .ok_or(Error("INVALID_REQUEST", None))?;
    let rungs: Vec<(u32, u8)> = request
        .get("rungs")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|r| {
                    let dpi = r.get("dpiCap").and_then(Value::as_u64)?;
                    let q = r.get("jpegQuality").and_then(Value::as_u64).unwrap_or(70);
                    if !(36..=600).contains(&dpi) || q > 100 {
                        return None;
                    }
                    Some((dpi as u32, q as u8))
                })
                .collect()
        })
        .filter(|r: &Vec<(u32, u8)>| !r.is_empty() && r.len() <= MAX_RUNGS)
        .ok_or(Error("INVALID_REQUEST", None))?;
    let target = request.get("targetBytes").and_then(Value::as_u64);
    let budget = request
        .get("budgetMillis")
        .and_then(Value::as_u64)
        .map(Duration::from_millis);
    resample_document(Path::new(input), Path::new(output), &rungs, target, budget)
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
