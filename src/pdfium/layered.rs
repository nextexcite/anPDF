//! Fixed-resolution layered rasterization for simple, dense traced pages.
use super::{Error, Result, page_contents, page_render_config, traced_evidence};
use fax::{Color, VecWriter, encoder::Encoder};
use flate2::{Compression, write::ZlibEncoder};
use image::{DynamicImage, GrayImage, RgbImage, imageops};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use pdfium_render::prelude::PdfDocument;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fmt::Write as _, io::Write as _};

const DPI: u32 = 400;
const TILE: u32 = 100;
type Rect = (u32, u32, u32, u32);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Patch {
    Gray,
    Color,
    SparseColor,
}

fn simple_resources(doc: &Document, page: &Dictionary) -> bool {
    if [b"Group".as_slice(), b"UserUnit", b"StructParents", b"AA"]
        .iter()
        .any(|key| page.has(key))
    {
        return false;
    }
    if !page.has(b"Resources") {
        // Don't guess the contents of inherited resources.
        return false;
    }
    let Ok(resources) = page.get_deref(b"Resources", doc).and_then(Object::as_dict) else {
        return false;
    };
    resources.iter().all(|(key, _)| match key.as_slice() {
        b"ProcSet" => true,
        b"XObject" => resources
            .get_deref(b"XObject", doc)
            .and_then(Object::as_dict)
            .is_ok_and(|objects| {
                objects.iter().all(|(name, _)| {
                    objects
                        .get_deref(name, doc)
                        .and_then(Object::as_stream)
                        .is_ok_and(|s| {
                            s.dict
                                .get(b"Subtype")
                                .and_then(Object::as_name)
                                .is_ok_and(|name| name == b"Image")
                                && !s.dict.has(b"OC")
                        })
                })
            }),
        _ => false,
    })
}

fn regions(rgb: &RgbImage, gray: &GrayImage) -> Option<Vec<(Rect, Patch)>> {
    let (w, h) = rgb.dimensions();
    let mut finished = Vec::new();
    let mut active: BTreeMap<(u32, u32, Patch), u32> = BTreeMap::new();
    let mut color_area = 0;
    for y in (0..h).step_by(TILE as usize) {
        let b = (y + TILE).min(h);
        let mut spans: Vec<(u32, u32, Patch)> = Vec::new();
        for x in (0..w).step_by(TILE as usize) {
            let r = (x + TILE).min(w);
            let area = (r - x) * (b - y);
            let (mut color, mut mid, mut pale, mut erased_color) = (0, 0, false, false);
            for j in y..b {
                for i in x..r {
                    let [a, g, c] = rgb.get_pixel(i, j).0;
                    let l = gray.get_pixel(i, j).0[0];
                    let chromatic = a.max(g).max(c) - a.min(g).min(c) >= 15;
                    color += u32::from(chromatic);
                    erased_color |= chromatic && l >= 160;
                    mid += u32::from((55..=218).contains(&l));
                    pale |= (160..255).contains(&l);
                }
            }
            let kind = if color * 500 > area || erased_color {
                color_area += area;
                if color_area > w * h / 2 {
                    return None;
                }
                if color * 500 > area {
                    Patch::Color
                } else {
                    Patch::SparseColor
                }
            } else {
                // Standalone detail that would become white needs a patch even
                // if it covers only one pixel. Ignore the fringe of dark ink.
                let needs_patch = mid * 100 > area * 12
                    || pale
                        && (y..b)
                            .flat_map(|j| (x..r).map(move |i| (i, j)))
                            .any(|(i, j)| {
                                (160..255).contains(&gray.get_pixel(i, j).0[0])
                                    && (j.saturating_sub(2)..=(j + 2).min(h - 1)).all(|yy| {
                                        (i.saturating_sub(2)..=(i + 2).min(w - 1))
                                            .all(|xx| gray.get_pixel(xx, yy).0[0] >= 150)
                                    })
                            });
                if !needs_patch {
                    continue;
                }
                Patch::Gray
            };
            if let Some((_, end, _)) = spans
                .last_mut()
                .filter(|(_, end, previous)| *end == x && *previous == kind)
            {
                *end = r;
            } else {
                spans.push((x, r, kind));
            }
        }
        let next = spans
            .into_iter()
            .map(|s| (s, active.remove(&s).unwrap_or(y)))
            .collect();
        finished.extend(
            active
                .into_iter()
                .map(|((l, r, color), t)| ((l, t, r, y), color)),
        );
        active = next;
    }
    finished.extend(
        active
            .into_iter()
            .map(|((l, r, color), t)| ((l, t, r, h), color)),
    );
    Some(finished)
}

fn deflate(data: &[u8]) -> Result<Vec<u8>> {
    let mut writer = ZlibEncoder::new(Vec::new(), Compression::default());
    writer.write_all(data)?;
    Ok(writer.finish()?)
}

fn base(gray: &GrayImage) -> Result<Stream> {
    let (w, h) = gray.dimensions();
    let mut bitmap = vec![0u8; (w.div_ceil(8) * h) as usize];
    let mut fax = Encoder::new(VecWriter::new());
    for (row, packed) in gray.rows().zip(bitmap.chunks_mut(w.div_ceil(8) as usize)) {
        for (x, pixel) in row.enumerate() {
            if pixel.0[0] >= 160 {
                packed[x / 8] |= 0x80 >> (x % 8);
            }
        }
        fax.encode_line(
            (0..w as usize).map(|x| {
                if packed[x / 8] & (0x80 >> (x % 8)) == 0 {
                    Color::Black
                } else {
                    Color::White
                }
            }),
            w,
        )
        .unwrap();
    }
    let fax = fax.finish().unwrap().finish();
    let flate = deflate(&bitmap)?;
    if fax.len() + 100 < flate.len() {
        let mut stream = image(w, h, 1, "DeviceGray", "CCITTFaxDecode", fax);
        stream.dict.set(
            "DecodeParms",
            dictionary! { "K" => -1, "Columns" => i64::from(w), "Rows" => i64::from(h) },
        );
        Ok(stream)
    } else {
        Ok(image(w, h, 1, "DeviceGray", "FlateDecode", flate))
    }
}

fn image(w: u32, h: u32, bits: i64, space: &str, filter: &str, data: Vec<u8>) -> Stream {
    Stream::new(
        dictionary! {
            "Type" => Object::Name(b"XObject".to_vec()), "Subtype" => Object::Name(b"Image".to_vec()),
            "Width" => i64::from(w), "Height" => i64::from(h), "BitsPerComponent" => bits,
            "ColorSpace" => Object::Name(space.as_bytes().to_vec()), "Filter" => Object::Name(filter.as_bytes().to_vec()),
        },
        data,
    )
}

pub(super) fn rebuild(document: &PdfDocument, request: &Value) -> Result<Value> {
    let path = |key: &str| request[key].as_str().ok_or(Error("INVALID_REQUEST", None));
    let mut doc = Document::load(path("input")?).map_err(|_| Error("INVALID_PDF", None))?;
    let pages = doc.get_pages();
    let mut rebuilt = 0;
    let protected = doc.catalog().map_err(|_| Error("INVALID_PDF", None))?;
    if [b"StructTreeRoot".as_slice(), b"OCProperties", b"MarkInfo"]
        .iter()
        .any(|key| protected.has(key))
    {
        return Ok(json!({"rebuilt": 0, "skipped": pages.len()}));
    }
    for (&n, &id) in &pages {
        let mut page = doc
            .get_dictionary(id)
            .map_err(|_| Error("INVALID_PDF", None))?
            .clone();
        if !simple_resources(&doc, &page)
            || traced_evidence(&doc, &page, &page_contents(&doc, &page).0, true).is_some()
        {
            continue;
        }
        let rendered = document
            .pages()
            .get(i32::try_from(n - 1).map_err(|_| Error("LIMIT_EXCEEDED", None))?)
            .map_err(|_| Error("INVALID_PDF", None))?;
        let (pt_w, pt_h) = (rendered.width().value, rendered.height().value);
        let (w, h) = super::dimensions(pt_w, pt_h, DPI)?;
        if super::validate_render_dimensions(w, h).is_err() {
            continue;
        }
        let rgb = rendered
            .render_with_config(&page_render_config((w, h)))
            .map_err(|_| Error("RENDER_FAILED", None))?
            .as_image()
            .map_err(|_| Error("INTERNAL", None))?
            .into_rgb8();
        let mut gray = imageops::grayscale(&rgb);
        let Some(regions) = regions(&rgb, &gray) else {
            continue;
        };
        let (w, h) = rgb.dimensions();
        let mut objects = Dictionary::new();
        let mut contents = String::new();
        let (sx, sy) = (
            f64::from(pt_w) / f64::from(w),
            f64::from(pt_h) / f64::from(h),
        );
        let mut add = |stream, (l, t, r, b): Rect| {
            let name = format!("Im{}", objects.len());
            writeln!(
                contents,
                "q {:.7} 0 0 {:.7} {:.7} {:.7} cm /{name} Do Q",
                sx * f64::from(r - l),
                sy * f64::from(b - t),
                sx * f64::from(l),
                sy * f64::from(h - b)
            )
            .unwrap();
            objects.set(name, Object::Reference(doc.add_object(stream)));
        };
        let mut patches = Vec::with_capacity(regions.len());
        for (rect @ (l, t, r, b), kind) in regions {
            let (pw, ph) = (r - l, b - t);
            let stream = if kind != Patch::Gray {
                let rgb = imageops::crop_imm(&rgb, l, t, pw, ph).to_image();
                let (filter, data) = if kind == Patch::SparseColor {
                    ("FlateDecode", deflate(rgb.as_raw())?)
                } else {
                    (
                        "DCTDecode",
                        super::encode_jpeg(&DynamicImage::ImageRgb8(rgb), 88)
                            .ok_or(Error("INTERNAL", None))?,
                    )
                };
                image(pw, ph, 8, "DeviceRGB", filter, data)
            } else {
                let patch = imageops::crop_imm(&gray, l, t, pw, ph).to_image();
                image(
                    pw,
                    ph,
                    8,
                    "DeviceGray",
                    "FlateDecode",
                    deflate(patch.as_raw())?,
                )
            };
            patches.push((stream, rect));
            for y in t..b {
                let start = (y * w + l) as usize;
                gray.as_mut()[start..start + pw as usize].fill(255);
            }
        }
        add(base(&gray)?, (0, 0, w, h));
        for (stream, rect) in patches {
            add(stream, rect);
        }
        page.set("Resources", dictionary! { "XObject" => objects });
        page.set(
            "Contents",
            Object::Reference(
                doc.add_object(Stream::new(Dictionary::new(), contents.into_bytes())),
            ),
        );
        doc.objects.insert(id, Object::Dictionary(page));
        rebuilt += 1;
    }
    if rebuilt != 0 {
        doc.prune_objects();
        doc.save(path("output")?)
            .map_err(|_| Error("INTERNAL", None))?;
    }
    Ok(json!({"rebuilt": rebuilt, "skipped": pages.len() - rebuilt}))
}
