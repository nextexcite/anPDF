use crate::{
    backend::{
        Control, ExtractTextBackend, MAX_TEXT_RESPONSE_BYTES, Qpdf, RenderPageBackend,
        validate_render_dimensions,
    },
    error::{Error, Result},
};
use pdfium_render::prelude::*;
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
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
