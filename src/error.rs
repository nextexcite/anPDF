use serde_json::{Value, json};
#[derive(Debug, Clone)]
pub struct Error(pub &'static str);
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    pub fn envelope(&self, op: &str) -> Value {
        json!({"contractVersion":crate::contract::VERSION,"operationId":op,
        "error":{"code":self.0,"message":self.message(),"retryable":matches!(self.0,"TIMEOUT"|"INPUT_CHANGED"|"BACKEND_UNAVAILABLE")}})
    }
    pub fn message(&self) -> &'static str {
        match self.0 {
            "INVALID_REQUEST" => "Request does not satisfy the operation contract.",
            "NOT_FOUND" => "Requested resource was not found.",
            "ACCESS_DENIED" => "Filesystem or transport policy denied access.",
            "OUTPUT_EXISTS" => "Output exists; explicit replacement is required.",
            "INPUT_CHANGED" => "An input changed during processing.",
            "INVALID_PDF" => "PDF structural processing failed.",
            "ENCRYPTED_UNSUPPORTED" => "Encrypted PDFs are unsupported.",
            "SIGNED_UNSUPPORTED" => "Transforming signed PDFs is unsupported.",
            "FORM_UNSUPPORTED" => "Transforming form-bearing PDFs is unsupported.",
            "PRESERVATION_UNKNOWN" => "Preservation could not be established.",
            "UNSUPPORTED_OPERATION" => "Operation is unsupported.",
            "LIMIT_EXCEEDED" => "A configured processing limit was exceeded.",
            "TIMEOUT" => "Processing was cancelled or exceeded its deadline.",
            "BACKEND_UNAVAILABLE" => "The qualified qpdf backend is unavailable.",
            "UNAUTHENTICATED" => "Bearer authentication is required.",
            _ => "Processing failed internally.",
        }
    }
    pub fn http(&self) -> u16 {
        match self.0 {
            "UNAUTHENTICATED" => 401,
            "INVALID_REQUEST" => 400,
            "NOT_FOUND" => 404,
            "ACCESS_DENIED" => 403,
            "OUTPUT_EXISTS" | "INPUT_CHANGED" => 409,
            "LIMIT_EXCEEDED" => 413,
            "TIMEOUT" => 504,
            "BACKEND_UNAVAILABLE" => 503,
            "INVALID_PDF"
            | "ENCRYPTED_UNSUPPORTED"
            | "SIGNED_UNSUPPORTED"
            | "FORM_UNSUPPORTED"
            | "PRESERVATION_UNKNOWN"
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
        Self(match e.kind() {
            std::io::ErrorKind::NotFound => "NOT_FOUND",
            std::io::ErrorKind::AlreadyExists => "OUTPUT_EXISTS",
            _ => "ACCESS_DENIED",
        })
    }
}
