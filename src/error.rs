use serde_json::{Value, json};
/// Canonical error code plus an optional human-facing detail naming the
/// field, flag or file at fault. The detail rides inside `message` so the
/// envelope schema is unchanged and identical across CLI, HTTP and MCP.
#[derive(Debug, Clone)]
pub struct Error(pub &'static str, pub Option<String>);
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    /// Attach a detail sentence to an error produced elsewhere.
    pub fn with(mut self, detail: impl Into<String>) -> Self {
        self.1 = Some(detail.into());
        self
    }
    /// `From<io::Error>` loses the path context; rebuild the code with a
    /// detail naming the file or argument that failed.
    pub fn io(e: &std::io::Error, detail: impl Into<String>) -> Self {
        Self::from_io_kind(e.kind()).with(detail)
    }
    fn from_io_kind(kind: std::io::ErrorKind) -> Self {
        Self(
            match kind {
                std::io::ErrorKind::NotFound => "NOT_FOUND",
                std::io::ErrorKind::AlreadyExists => "OUTPUT_EXISTS",
                _ => "ACCESS_DENIED",
            },
            None,
        )
    }
    pub fn envelope(&self, op: &str) -> Value {
        json!({"contractVersion":crate::contract::VERSION,"operationId":op,
        "error":{"code":self.0,"message":self.message(),"retryable":matches!(self.0,"TIMEOUT"|"INPUT_CHANGED"|"BACKEND_UNAVAILABLE")}})
    }
    /// The canonical base sentence for the code, without any detail.
    pub fn base(&self) -> &'static str {
        match self.0 {
            "INVALID_REQUEST" => "Request does not satisfy the operation contract.",
            "NOT_FOUND" => "Requested resource was not found.",
            "ACCESS_DENIED" => "Filesystem or transport policy denied access.",
            "OUTPUT_EXISTS" => "Output exists; explicit replacement is required.",
            "INPUT_CHANGED" => "An input changed during processing.",
            "INVALID_PDF" => "PDF structural processing failed.",
            "INVALID_PASSWORD" => "The supplied password does not open the document.",
            "ENCRYPTED_UNSUPPORTED" => "Encrypted PDFs are unsupported.",
            "SIGNED_UNSUPPORTED" => "Transforming signed PDFs is unsupported.",
            "FORM_UNSUPPORTED" => "Transforming form-bearing PDFs is unsupported.",
            "PRESERVATION_UNKNOWN" => "Preservation could not be established.",
            "PRESERVATION_UNMET" => "Required document aspects could not be fully preserved.",
            "SIZE_TARGET_UNMET" => "Size target not met; no output was published.",
            "UNSUPPORTED_OPERATION" => "Operation is unsupported.",
            "LIMIT_EXCEEDED" => "A configured processing limit was exceeded.",
            "TIMEOUT" => "Processing was cancelled or exceeded its deadline.",
            "BACKEND_UNAVAILABLE" => {
                "No configured backend can complete this operation. Structural processing requires qpdf 12.x >=12.4.2 (exactly 12.4.2 in strict mode)."
            }
            "UNAUTHENTICATED" => "Bearer authentication is required.",
            _ => "Processing failed internally.",
        }
    }
    /// The detail sentence naming the field, flag or file at fault.
    pub fn detail(&self) -> Option<&str> {
        self.1.as_deref()
    }
    pub fn message(&self) -> String {
        match &self.1 {
            Some(detail) => format!("{} — {detail}", self.base()),
            None => self.base().into(),
        }
    }
    pub fn http(&self) -> u16 {
        match self.0 {
            "UNAUTHENTICATED" => 401,
            "INVALID_REQUEST" => 400,
            "NOT_FOUND" => 404,
            "ACCESS_DENIED" | "INVALID_PASSWORD" => 403,
            "OUTPUT_EXISTS" | "INPUT_CHANGED" => 409,
            "LIMIT_EXCEEDED" => 413,
            "TIMEOUT" => 504,
            "BACKEND_UNAVAILABLE" => 503,
            "INVALID_PDF"
            | "ENCRYPTED_UNSUPPORTED"
            | "SIGNED_UNSUPPORTED"
            | "FORM_UNSUPPORTED"
            | "PRESERVATION_UNKNOWN"
            | "PRESERVATION_UNMET"
            | "SIZE_TARGET_UNMET"
            | "UNSUPPORTED_OPERATION" => 422,
            _ => 500,
        }
    }
    pub fn exit(&self) -> i32 {
        match self.http() {
            400 => 2,
            403 | 404 => 3,
            409 => 4,
            422 => {
                if self.0 == "UNSUPPORTED_OPERATION" {
                    6
                } else {
                    5
                }
            }
            413 | 504 => 7,
            _ => 8,
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::from_io_kind(e.kind())
    }
}
