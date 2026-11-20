use crate::{
    BackendRegistry,
    backend::{
        Control, ExtractTextBackend, MAX_PNG_BYTES, MAX_RENDER_PIXELS, MAX_TEXT_RESPONSE_BYTES,
        RenderPageBackend, child_rss_exceeded, limit_worker, validate_render_dimensions,
    },
    error::{Error, Result},
};
use pdf_oxide::{
    document::{PdfDocument, ReadingOrder},
    geometry::Rect,
    object::Object,
    rendering::{RenderOptions, render_page},
};
use serde_json::{Value, json};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io::{self, BufWriter, Read, Seek, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

pub const ENGINE: &str = "pdfoxide";
#[doc(hidden)]
pub const WORKER_ARG: &str = "--internal-pdfoxide-worker";
const MAX_SPANS: usize = 20_000;

pub fn register(registry: &mut BackendRegistry, executable: PathBuf) -> Result<()> {
    let provider = Arc::new(PdfOxide { executable });
    registry.register_text(ENGINE, provider.clone())?;
    registry.register_renderer(ENGINE, provider)
}

struct PdfOxide {
    executable: PathBuf,
}

impl PdfOxide {
    fn run(
        &self,
        args: &[OsString],
        output: File,
        file_limit: u64,
        control: &Control,
    ) -> Result<()> {
        control.check()?;
        let mut cmd = Command::new(&self.executable);
        cmd.arg(WORKER_ARG)
            .args(args)
            .env_clear()
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(output)
            .stderr(Stdio::null());
        limit_worker(&mut cmd, file_limit as libc::rlim_t);
        let mut child = cmd.spawn().map_err(|_| Error("BACKEND_UNAVAILABLE"))?;
        let pid = child.id() as i32;
        let stop = |child: &mut std::process::Child| {
            unsafe { libc::kill(-pid, libc::SIGKILL) };
            let _ = child.wait();
        };
        loop {
            if let Err(e) = control.check() {
                stop(&mut child);
                return Err(e);
            }
            if child_rss_exceeded(pid) {
                stop(&mut child);
                return Err(Error("LIMIT_EXCEEDED"));
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    unsafe { libc::kill(-pid, libc::SIGKILL) };
                    return match status.code() {
                        Some(0) => Ok(()),
                        Some(2) => Err(Error("INVALID_REQUEST")),
                        Some(5) => Err(Error("INVALID_PDF")),
                        Some(6) => Err(Error("UNSUPPORTED_OPERATION")),
                        Some(7) | None => Err(Error("LIMIT_EXCEEDED")),
                        Some(9) => Err(Error("ENCRYPTED_UNSUPPORTED")),
                        _ => Err(Error("INTERNAL")),
                    };
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => {
                    stop(&mut child);
                    return Err(Error("INTERNAL"));
                }
            }
        }
    }
}

impl ExtractTextBackend for PdfOxide {
    fn extract_text(
        &self,
        input: &Path,
        pages: &[u64],
        control: &Control,
    ) -> Result<(Vec<Value>, Vec<Value>)> {
        let mut output = tempfile::tempfile_in(input.parent().ok_or(Error("INTERNAL"))?)?;
        let mut args = vec!["text".into(), input.into()];
        args.extend(pages.iter().map(|page| page.to_string().into()));
        self.run(
            &args,
            output.try_clone()?,
            MAX_TEXT_RESPONSE_BYTES as u64,
            control,
        )?;
        output.rewind()?;
        let pages = serde_json::from_reader(output.take(MAX_TEXT_RESPONSE_BYTES as u64))
            .map_err(|_| Error("INTERNAL"))?;
        Ok((pages, Vec::new()))
    }
}

impl RenderPageBackend for PdfOxide {
    fn render_page(
        &self,
        input: &Path,
        page: u64,
        dpi: u32,
        output: &Path,
        control: &Control,
    ) -> Result<Vec<Value>> {
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?;
        self.run(
            &[
                "render".into(),
                input.into(),
                page.to_string().into(),
                dpi.to_string().into(),
            ],
            output,
            MAX_PNG_BYTES,
            control,
        )?;
        Ok(Vec::new())
    }
}

fn span_bounds(rect: Rect) -> Result<Value> {
    let [left, bottom] = [f64::from(rect.x), f64::from(rect.y)];
    let right = left + f64::from(rect.width);
    let top = bottom + f64::from(rect.height);
    if ![left, bottom, right, top].into_iter().all(f64::is_finite) || right < left || top < bottom {
        return Err(Error("INVALID_PDF"));
    }
    Ok(json!({"left":left,"bottom":bottom,"right":right,"top":top}))
}

fn extract(doc: &PdfDocument, pages: &[usize], mut output: impl Write) -> Result<()> {
    output.write_all(b"[").map_err(|_| Error("INTERNAL"))?;
    for (i, &page) in pages.iter().enumerate() {
        let spans = doc
            .extract_spans_with_reading_order(page - 1, ReadingOrder::ColumnAware)
            .map_err(|_| Error("INVALID_PDF"))?;
        if spans.len() > MAX_SPANS
            || spans.iter().map(|s| s.text.len()).sum::<usize>() > MAX_TEXT_RESPONSE_BYTES
        {
            return Err(Error("LIMIT_EXCEEDED"));
        }
        let spans = spans
            .into_iter()
            .filter(|s| !s.text.is_empty())
            .map(|s| Ok(json!({"text":s.text,"bounds":span_bounds(s.bbox)?})))
            .collect::<Result<Vec<_>>>()?;
        let text = doc
            .extract_text(page - 1)
            .map_err(|_| Error("INVALID_PDF"))?;
        if text.len() > MAX_TEXT_RESPONSE_BYTES {
            return Err(Error("LIMIT_EXCEEDED"));
        }
        if i != 0 {
            output.write_all(b",").map_err(|_| Error("INTERNAL"))?;
        }
        serde_json::to_writer(&mut output, &json!({"page":page,"text":text,"spans":spans}))
            .map_err(|_| Error("INTERNAL"))?;
    }
    output.write_all(b"]").map_err(|_| Error("INTERNAL"))
}

fn render_dimensions(
    media: Rect,
    crop: Option<Rect>,
    rotation: i32,
    dpi: u32,
) -> Result<(u32, u32)> {
    // Match the pinned renderer's visible box and DPI rounding before it allocates.
    let visible = crop
        .and_then(|crop| {
            let x0 = media.x.max(crop.x);
            let y0 = media.y.max(crop.y);
            let x1 = (media.x + media.width).min(crop.x + crop.width);
            let y1 = (media.y + media.height).min(crop.y + crop.height);
            (x1 > x0 && y1 > y0).then(|| Rect::from_points(x0, y0, x1, y1))
        })
        .unwrap_or(media);
    let scale = dpi as f32 / 72.0;
    let (mut width, mut height) = (
        (visible.width * scale).ceil(),
        (visible.height * scale).ceil(),
    );
    if matches!(rotation.rem_euclid(360), 90 | 270) {
        std::mem::swap(&mut width, &mut height);
    }
    if !width.is_finite() || !height.is_finite() || width < 1.0 || height < 1.0 {
        return Err(Error("INVALID_PDF"));
    }
    let dimensions = (width as u32, height as u32);
    validate_render_dimensions(dimensions.0, dimensions.1)?;
    let precise_extent = |points| (f64::from(points) * f64::from(scale)).ceil();
    if precise_extent(visible.width) * precise_extent(visible.height) > MAX_RENDER_PIXELS as f64 {
        return Err(Error("LIMIT_EXCEEDED"));
    }
    Ok(dimensions)
}

fn rasterize(doc: &PdfDocument, page: usize, dpi: u32, mut output: impl Write) -> Result<()> {
    let page_dict = doc.get_page(page - 1).map_err(|_| Error("INVALID_PDF"))?;
    if let Some(unit) = page_dict
        .as_dict()
        .ok_or(Error("INVALID_PDF"))?
        .get("UserUnit")
    {
        let unit = doc.resolve_object(unit).map_err(|_| Error("INVALID_PDF"))?;
        match unit {
            Object::Null | Object::Integer(1) | Object::Real(1.0) => {}
            Object::Integer(n) if n > 0 => return Err(Error("UNSUPPORTED_OPERATION")),
            Object::Real(n) if n.is_finite() && n > 0.0 => {
                return Err(Error("UNSUPPORTED_OPERATION"));
            }
            _ => return Err(Error("INVALID_PDF")),
        }
    }
    let info = doc
        .get_page_info(page - 1)
        .map_err(|_| Error("INVALID_PDF"))?;
    let dimensions = render_dimensions(info.media_box, info.crop_box, info.rotation, dpi)?;
    let mut options = RenderOptions::with_dpi(dpi);
    options.render_annotations = false;
    options.max_output_pixels = MAX_RENDER_PIXELS;
    let image = render_page(doc, page - 1, &options).map_err(|_| Error("INVALID_PDF"))?;
    if (image.width, image.height) != dimensions {
        return Err(Error("INTERNAL"));
    }
    if image.data.len() as u64 > MAX_PNG_BYTES {
        return Err(Error("LIMIT_EXCEEDED"));
    }
    output.write_all(&image.data).map_err(|_| Error("INTERNAL"))
}

fn run_worker(mut args: impl Iterator<Item = OsString>, mut output: impl Write) -> Result<()> {
    let job = args.next().ok_or(Error("INVALID_REQUEST"))?;
    let input = args.next().ok_or(Error("INVALID_REQUEST"))?;
    let doc = PdfDocument::open(input).map_err(|_| Error("INVALID_PDF"))?;
    if doc.is_encrypted() {
        return Err(Error("ENCRYPTED_UNSUPPORTED"));
    }
    let count = doc.page_count().map_err(|_| Error("INVALID_PDF"))?;
    let number = |arg: OsString| {
        arg.to_str()
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or(Error("INVALID_REQUEST"))
    };
    let page = |arg| {
        number(arg).and_then(|n| {
            (1..=count)
                .contains(&n)
                .then_some(n)
                .ok_or(Error("INVALID_REQUEST"))
        })
    };
    match job.as_os_str() {
        j if j == OsStr::new("text") => extract(
            &doc,
            &args.map(page).collect::<Result<Vec<_>>>()?,
            &mut output,
        )?,
        j if j == OsStr::new("render") => {
            let page = page(args.next().ok_or(Error("INVALID_REQUEST"))?)?;
            let dpi = number(args.next().ok_or(Error("INVALID_REQUEST"))?)?;
            if !(36..=300).contains(&dpi) || args.next().is_some() {
                return Err(Error("INVALID_REQUEST"));
            }
            rasterize(&doc, page, dpi as u32, &mut output)?;
        }
        _ => return Err(Error("INVALID_REQUEST")),
    }
    output.flush().map_err(|_| Error("INTERNAL"))
}

#[doc(hidden)]
pub fn worker(args: impl Iterator<Item = OsString>) -> i32 {
    match run_worker(args, BufWriter::new(io::stdout().lock())) {
        Ok(()) => 0,
        Err(Error("ENCRYPTED_UNSUPPORTED")) => 9,
        Err(e) => e.exit(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_keep_the_pdf_origin_and_y_up_axes() {
        assert_eq!(
            span_bounds(Rect::new(-20.0, 120.0, 30.0, 16.0)).unwrap(),
            json!({"left":-20.0,"bottom":120.0,"right":10.0,"top":136.0})
        );
        assert_eq!(
            span_bounds(Rect::new(0.0, f32::INFINITY, 1.0, 1.0))
                .unwrap_err()
                .0,
            "INVALID_PDF"
        );
    }

    #[test]
    fn dimensions_apply_crop_rotation_and_limits_at_requested_dpi() {
        let media = Rect::new(0.0, 0.0, 300.0, 200.0);
        let crop = Rect::new(-10.0, 40.0, 160.0, 40.0);
        assert_eq!(
            render_dimensions(media, Some(crop), -90, 144).unwrap(),
            (80, 300)
        );
        for (w, h) in [(8193.0, 1.0), (4097.0, 4096.0)] {
            assert_eq!(
                render_dimensions(Rect::new(0.0, 0.0, w, h), None, 0, 72)
                    .unwrap_err()
                    .0,
                "LIMIT_EXCEEDED"
            );
        }
    }

    #[test]
    fn expired_control_prevents_worker_launch() {
        let provider = PdfOxide {
            executable: PathBuf::new(),
        };
        let control = Control {
            deadline: std::time::Instant::now() - Duration::from_secs(1),
            ..Control::default()
        };
        assert_eq!(
            provider
                .run(&[], tempfile::tempfile().unwrap(), 1, &control)
                .unwrap_err()
                .0,
            "TIMEOUT"
        );
    }
}
