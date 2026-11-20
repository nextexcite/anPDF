use crate::{
    backend::{MAX_PNG_BYTES, MAX_TEXT_RESPONSE_BYTES, validate_render_dimensions},
    contract,
    error::{Error, Result},
};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

struct ResponseBudget(usize);

impl Write for ResponseBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or(io::ErrorKind::FileTooLarge)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn validate_envelope(id: &str, value: &Value) -> Result<()> {
    serde_json::to_writer(ResponseBudget(MAX_TEXT_RESPONSE_BYTES), value)
        .map_err(|_| Error("LIMIT_EXCEEDED", None))?;
    if !contract::valid_output(id, value) {
        return Err(Error("INTERNAL", None));
    }
    Ok(())
}

pub(crate) fn text_envelope(
    engine: &str,
    requested: &[u64],
    pages: Vec<Value>,
    warnings: Vec<Value>,
) -> Result<Value> {
    let value = json!({
        "contractVersion":contract::VERSION,"operationId":"extractText",
        "result":{"engine":engine,"pages":pages},"warnings":warnings,
    });
    validate_envelope("extractText", &value)?;
    let pages = value["result"]["pages"].as_array().unwrap();
    if pages.len() != requested.len()
        || pages.iter().zip(requested).any(|(page, &number)| {
            page["page"] != number
                || page["spans"].as_array().unwrap().iter().any(|span| {
                    let bounds = &span["bounds"];
                    bounds["left"].as_f64() > bounds["right"].as_f64()
                        || bounds["bottom"].as_f64() > bounds["top"].as_f64()
                })
        })
    {
        return Err(Error("INTERNAL", None));
    }
    Ok(value)
}

pub(crate) struct PngInfo {
    pub(crate) bytes: u64,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) fn read_png(path: &Path) -> Result<(File, PngInfo)> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| Error("INTERNAL", None))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() < 57 {
        return Err(Error("INTERNAL", None));
    }
    if metadata.len() > MAX_PNG_BYTES {
        return Err(Error("LIMIT_EXCEEDED", None));
    }
    let mut header = [0u8; 24];
    file.read_exact(&mut header)
        .map_err(|_| Error("INTERNAL", None))?;
    if &header[..16] != b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR" {
        return Err(Error("INTERNAL", None));
    }
    let width = u32::from_be_bytes(header[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(header[20..24].try_into().unwrap());
    validate_render_dimensions(width, height)?;
    Ok((
        file,
        PngInfo {
            bytes: metadata.len(),
            width,
            height,
        },
    ))
}
