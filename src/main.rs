use anpdf::{
    BackendRegistry, Dispatcher,
    backend::{Control, Progress},
    contract,
    error::{Error, Result},
};
use clap::{CommandFactory, Parser};
use serde_json::Value;
use std::{
    collections::HashSet,
    ffi::OsString,
    io::{IsTerminal, Read, Write},
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const STANDARD_QPDF_PATHS: &[&str] = &[
    "/opt/qpdf/bin/qpdf",
    "/usr/bin/qpdf",
    "/opt/homebrew/bin/qpdf",
    "/usr/local/bin/qpdf",
];

#[derive(Debug)]
enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "bash" => Ok(Self::Bash),
            "zsh" => Ok(Self::Zsh),
            "fish" => Ok(Self::Fish),
            _ => Err(Error("INVALID_REQUEST", None)),
        }
    }

    fn clap(self) -> clap_complete::Shell {
        match self {
            Self::Bash => clap_complete::Shell::Bash,
            Self::Zsh => clap_complete::Shell::Zsh,
            Self::Fish => clap_complete::Shell::Fish,
        }
    }
}

#[derive(Debug)]
enum Action {
    Help(Option<String>),
    Version,
    Completions(Shell),
    Run(Box<Options>),
}

#[derive(Debug)]
struct Options {
    command: String,
    root: PathBuf,
    qpdf: PathBuf,
    qpdf_exact: bool,
    pdfium_lib: Option<PathBuf>,
    request: Option<String>,
    dry: bool,
    json: bool,
    port: u16,
    token_file: Option<PathBuf>,
    output_only: Option<String>,
    output_byte_ceiling: Option<u64>,
    output_operation: Option<String>,
    schema: Option<String>,
    password_file: Option<String>,
    owner_password_file: Option<String>,
}

const TOP_AFTER_HELP: &str = "\
SHORTHAND (paths workspace-relative to --root, or absolute/~ adapting it):
  anpdf inspect <doc.pdf>
  anpdf merge <a.pdf> <b.pdf> [...] -o <merged.pdf>
  anpdf extract <doc.pdf> <pages> -o <subset.pdf>
  anpdf split <doc.pdf> <part.pdf=pages> [<part.pdf=pages> ...]
  anpdf remove-pages <doc.pdf> <pages> -o <trimmed.pdf>
  anpdf reorder <doc.pdf> <ordered-pages> -o <ordered.pdf>
  anpdf compress <doc.pdf> [-o <out.pdf>] [--mode lossless|lossy|layered] [--max-bytes N]
  anpdf extract-text <doc.pdf> <pages> [--engine <id>]
  anpdf render-page <doc.pdf> <page> -o <page.png> [--dpi 36-300] [--engine <id>]
  anpdf encrypt <doc.pdf> [<out.pdf>]   AES-256 + user password (+ optional owner)
  anpdf decrypt <doc.pdf> [<out.pdf>]   strip password protection (password required)

<pages> are 1-based numbers or ranges: \"1,3\", \"2-5\" (positional or --pages).
For reorder, <ordered-pages> is a full output-order permutation such as \"3,1,2\";
ranges still expand ascending, and the complete order is validated after inspection.
Use -o/--output to name a destination. A trailing output path also works
for single-output commands. Compress, encrypt and decrypt can derive an output
name. --replace overwrites an existing destination, never an input. A size
target without a mode or level can enable lossy compression; choose mode explicitly.
Shorthand paths may be absolute or start with ~: the workspace
root adapts to the files' common directory (--root pins it); JSON requests
stay relative to --root. A full JSON request may instead be passed via
--request-json, --request-file or piped stdin; a bare operation on a
terminal prints usage instead of blocking on stdin. encrypt/decrypt never
take a password inline — argv is already exposed to the OS at launch and
--request-json refuses password fields (without echoing them) — so a
terminal prompts hidden on /dev/tty, and scripts use a private
--password-file/--owner-password-file, --request-file or piped stdin
carrying the JSON fields.

`anpdf help <command>` shows the exact accepted set for that command.

OUTPUT AND PAGE OPTIONS:
  -o, --output <path>  Output path
  --pages <spec>       Page spec/order for extract/remove-pages/reorder/extract-text

SAFETY OPTIONS:
  --replace            Overwrite an existing destination, never the input
  --preserve <list>    Required aspects: metadata,bookmarks,annotations,attachments (or \"all\")
  --dry-run            Validate request and destination policy without processing the PDF

CREDENTIAL OPTIONS:
  --password-file <f>  Private (chmod 600) file; first line is the user password —
                       encrypt/decrypt (a lone newline supplies an empty password)
  --owner-password-file <f> Private file with the optional owner password — encrypt

COMMON OPTIONS:
  --root <dir>         Workspace root (default: .; adapts to absolute shorthand paths)
  --qpdf <abs-path>    Absolute path to qpdf 12.x >=12.4.2 (tested: 12.4.2)
  --qpdf-exact         Require exactly qpdf 12.4.2
  --pdfium-lib <path>  Optional PDFium dynamic library (overrides ANPDF_PDFIUM_LIB)
  --request-json <str> Inline JSON request payload (or pass JSON on stdin)
  --request-file <f>   Read JSON request payload from file
  --json               On a terminal, print the JSON result envelope instead of the summary
  --                   End of flags; following arguments are positional paths

SERVER OPTIONS:
  --port <port>        Loopback HTTP port for serve (default: 8787)
  --token-file <file>  Bearer token file (>=32 ASCII graphic chars) for serve

ENVIRONMENT:
  ANPDF_QPDF           Absolute path to qpdf 12.x >=12.4.2 (auto-discovers trusted paths)
  ANPDF_QPDF_EXACT     Set to 1 to require tested qpdf 12.4.2
  ANPDF_PDFIUM_LIB     Optional absolute PDFium library path (requires pdfium feature)

EXIT CODES:
  0 ok · 2 invalid request · 3 not found / access denied
  4 output exists / input changed · 5 processing rejected · 6 unsupported
  7 limit or timeout · 8 backend unavailable / internal
";

const HELP_TEMPLATE: &str =
    "{name} {version}\n{about}\n\nUSAGE:\n  {usage}\n\n{all-args}{after-help}";
const SUB_HELP_TEMPLATE: &str = "USAGE: {usage}{after-help}\n\n{all-args}";
const OUTPUT_HEADING: &str = "OUTPUT OPTIONS";
const OPERATION_HEADING: &str = "OPERATION OPTIONS";
const SAFETY_HEADING: &str = "SAFETY OPTIONS";
const CREDENTIAL_HEADING: &str = "CREDENTIAL OPTIONS";
const SERVER_HEADING: &str = "SERVER OPTIONS";
const COMMON_HEADING: &str = "COMMON OPTIONS";

/// Shared backend/root options on every document operation and server.
#[derive(clap::Args)]
struct Backends {
    #[arg(long, value_name = "dir", allow_hyphen_values = true, help_heading = COMMON_HEADING,
        value_hint = clap::ValueHint::DirPath,
        help = "Workspace root (default: .; adapts to absolute shorthand paths)")]
    root: Option<PathBuf>,
    #[arg(long, value_name = "abs-path", allow_hyphen_values = true, help_heading = COMMON_HEADING,
        value_hint = clap::ValueHint::FilePath,
        help = "Absolute path to qpdf 12.x >=12.4.2 (tested: 12.4.2)")]
    qpdf: Option<PathBuf>,
    #[arg(long, help_heading = COMMON_HEADING, help = "Require exactly qpdf 12.4.2")]
    qpdf_exact: bool,
    #[arg(long, value_name = "path", allow_hyphen_values = true, help_heading = COMMON_HEADING,
        value_hint = clap::ValueHint::FilePath,
        help = "Optional PDFium dynamic library (overrides ANPDF_PDFIUM_LIB)")]
    pdfium_lib: Option<PathBuf>,
}

/// Request-source and presentation options shared by document operations.
#[derive(clap::Args)]
struct Common {
    #[arg(long, help_heading = SAFETY_HEADING,
        help = "Validate request and destination policy without processing the PDF")]
    dry_run: bool,
    #[command(flatten)]
    backends: Backends,
    #[arg(long, value_name = "json", allow_hyphen_values = true, help_heading = COMMON_HEADING,
        help = "Inline JSON request payload (or pass JSON on stdin)")]
    request_json: Option<String>,
    #[arg(long, value_name = "file", allow_hyphen_values = true, help_heading = COMMON_HEADING,
        value_hint = clap::ValueHint::FilePath,
        help = "Read JSON request payload from file")]
    request_file: Option<PathBuf>,
    #[arg(long, help_heading = COMMON_HEADING,
        help = "On a terminal, print the JSON result envelope instead of the summary")]
    json: bool,
}

#[derive(clap::Args)]
struct Out {
    #[arg(
        short = 'o',
        long = "output",
        value_name = "path",
        allow_hyphen_values = true,
        value_hint = clap::ValueHint::FilePath,
        help_heading = OUTPUT_HEADING,
        help = "Output path"
    )]
    output: Option<String>,
}

#[derive(clap::Args)]
struct Preserve {
    #[arg(
        long,
        value_name = "list",
        allow_hyphen_values = true,
        help_heading = SAFETY_HEADING,
        help = "Required aspects: metadata,bookmarks,annotations,attachments (or \"all\")"
    )]
    preserve: Option<String>,
}

#[derive(clap::Args)]
struct Pages {
    #[arg(
        long,
        value_name = "spec",
        allow_hyphen_values = true,
        help_heading = OPERATION_HEADING,
        help = "Page spec: 1-based numbers or ranges like '1,3' or '2-5'"
    )]
    pages: Option<String>,
}

#[derive(clap::Args)]
struct OrderedPages {
    #[arg(
        long,
        value_name = "spec",
        allow_hyphen_values = true,
        help_heading = OPERATION_HEADING,
        help = "Full output order: every input page exactly once, e.g. '3,1,2'"
    )]
    pages: Option<String>,
}

#[derive(clap::Args)]
struct Engine {
    #[arg(
        long,
        value_name = "id",
        allow_hyphen_values = true,
        help_heading = OPERATION_HEADING,
        help = "Registered engine ID (see 'anpdf operations')"
    )]
    engine: Option<String>,
}

// Positional shorthands stay optional: a full JSON request via
// --request-json/--request-file/piped stdin is the other request source, and
// a bare command on a terminal prints usage rather than erroring here.
const DOC_HELP: &str = "Shorthand input PDF: workspace-relative, absolute or ~/…";

macro_rules! doc_args {
    ($name:ident { $($pos:tt)* } $(, $field:tt : $ty:ty)*) => {
        #[derive(clap::Args)]
        struct $name {
            $($pos)*
            $(#[command(flatten)] $field: $ty,)*
            #[command(flatten)]
            common: Common,
        }
    };
}

doc_args!(InspectArgs {
    #[arg(value_name = "DOC.pdf", value_hint = clap::ValueHint::FilePath,
        required = true,
        help = DOC_HELP)]
    doc: Option<String>,
}, deep_check: DeepCheck);

#[derive(clap::Args)]
struct DeepCheck {
    #[arg(
        long,
        help_heading = OPERATION_HEADING,
        help = "Decode stream payloads for complete qpdf structural validation"
    )]
    deep_check: bool,
}

doc_args!(MergeArgs {
    #[arg(value_name = "INPUTS", value_hint = clap::ValueHint::FilePath,
        num_args = 1.., required = true,
        help = "2..20 input PDFs in merge order; workspace-relative, absolute or ~/…")]
    inputs: Vec<String>,
}, out: Out, replace: ReplaceOnly, preserve: Preserve);

doc_args!(ExtractArgs {
    #[arg(value_name = "DOC.pdf", value_hint = clap::ValueHint::FilePath,
        required = true,
        help = DOC_HELP)]
    doc: Option<String>,
    #[arg(value_name = "PAGES",
        required_unless_present_any = ["request_json", "request_file", "pages"],
        help = "Page spec: 1-based numbers or ranges like '1,3' or '2-5'")]
    pages_pos: Option<String>,
    #[arg(value_name = "OUT.pdf", value_hint = clap::ValueHint::FilePath,
        help = "Output PDF (or -o/--output)")]
    out_pos: Option<String>,
}, out: Out, pages: Pages, replace: ReplaceOnly, preserve: Preserve);

doc_args!(SplitArgs {
    #[arg(value_name = "DOC.pdf", value_hint = clap::ValueHint::FilePath,
        required = true,
        help = DOC_HELP)]
    doc: Option<String>,
    #[arg(value_name = "PART.pdf=PAGES", value_hint = clap::ValueHint::FilePath,
        num_args = 1.., required = true,
        help = "Output part and its page spec, e.g. 'a.pdf=1-3'; repeat per part")]
    parts: Vec<String>,
}, preserve: Preserve, replace: ReplaceOnly);

doc_args!(RemoveArgs {
    #[arg(value_name = "DOC.pdf", value_hint = clap::ValueHint::FilePath,
        required = true,
        help = DOC_HELP)]
    doc: Option<String>,
    #[arg(value_name = "PAGES",
        required_unless_present_any = ["request_json", "request_file", "pages"],
        help = "Page spec: 1-based numbers or ranges like '1,3' or '2-5'")]
    pages_pos: Option<String>,
    #[arg(value_name = "OUT.pdf", value_hint = clap::ValueHint::FilePath,
        help = "Output PDF (or -o/--output)")]
    out_pos: Option<String>,
}, out: Out, pages: Pages, replace: ReplaceOnly, preserve: Preserve);

doc_args!(ReorderArgs {
    #[arg(value_name = "DOC.pdf", value_hint = clap::ValueHint::FilePath,
        required = true,
        help = DOC_HELP)]
    doc: Option<String>,
    #[arg(value_name = "ORDERED-PAGES",
        required_unless_present_any = ["request_json", "request_file", "pages"],
        help = "Full output order: every input page exactly once, e.g. '3,1,2'")]
    pages_pos: Option<String>,
    #[arg(value_name = "OUT.pdf", value_hint = clap::ValueHint::FilePath,
        help = "Output PDF (or -o/--output)")]
    out_pos: Option<String>,
}, out: Out, pages: OrderedPages, replace: ReplaceOnly, preserve: Preserve);

doc_args!(ExtractTextArgs {
    #[arg(value_name = "DOC.pdf", value_hint = clap::ValueHint::FilePath,
        required = true,
        help = DOC_HELP)]
    doc: Option<String>,
    #[arg(value_name = "PAGES",
        required_unless_present_any = ["request_json", "request_file", "pages"],
        help = "Page spec: 1-based numbers or ranges like '1,3' or '2-5'")]
    pages_pos: Option<String>,
}, engine: Engine, pages: Pages);

doc_args!(RenderArgs {
    #[arg(value_name = "DOC.pdf", value_hint = clap::ValueHint::FilePath,
        required = true,
        help = DOC_HELP)]
    doc: Option<String>,
    #[arg(value_name = "PAGE", required = true,
        help = "1-based page number to render")]
    page_pos: Option<String>,
    #[arg(value_name = "OUT.png", value_hint = clap::ValueHint::FilePath,
        help = "Output PNG (or -o/--output)")]
    out_pos: Option<String>,
}, out: Out, engine: Engine, dpi: Dpi, replace: ReplaceOnly);

#[derive(clap::Args)]
struct ReplaceOnly {
    #[arg(long, help_heading = SAFETY_HEADING, help = "Overwrite an existing destination, never the input")]
    replace: bool,
}

#[derive(clap::Args)]
struct Dpi {
    #[arg(
        long,
        value_name = "36-300",
        allow_hyphen_values = true,
        help_heading = OPERATION_HEADING,
        help = "Render DPI (36–300; default: 144)"
    )]
    dpi: Option<String>,
}

#[derive(clap::Args)]
struct CompressFlags {
    #[command(flatten)]
    out: Out,
    #[arg(long, value_name = "mode", allow_hyphen_values = true,
        value_parser = clap::builder::PossibleValuesParser::new(["lossless", "lossy", "layered"]),
        help_heading = OPERATION_HEADING,
        help = "Compression strategy (see Modes)")]
    mode: Option<String>,
    #[arg(
        long,
        value_name = "0-100",
        allow_hyphen_values = true,
        help_heading = OPERATION_HEADING,
        help = "Initial JPEG quality (lossy mode only; default: 80)"
    )]
    jpeg_quality: Option<String>,
    #[arg(long, value_name = "1-3", allow_hyphen_values = true,
        value_parser = clap::builder::PossibleValuesParser::new(["1", "2", "3"]),
        help_heading = OPERATION_HEADING,
        help = "Resampling level: 1 keeps pixels, 2 ≈200 DPI, 3 ≈120 DPI")]
    level: Option<String>,
    #[arg(
        long,
        value_name = "N",
        allow_hyphen_values = true,
        help_heading = OPERATION_HEADING,
        help = "Target size in bytes, decimal (3MB), or binary (3MiB)"
    )]
    max_bytes: Option<String>,
    #[arg(long, value_name = "behavior", allow_hyphen_values = true,
        value_parser = clap::builder::PossibleValuesParser::new(["bestEffort", "error"]),
        help_heading = SAFETY_HEADING,
        help = "If over target: write the smallest result (default) or fail without writing")]
    on_target_miss: Option<String>,
    #[arg(long, help_heading = SAFETY_HEADING, help = "Keep compressed bytes even when not smaller")]
    allow_larger: bool,
    #[command(flatten)]
    replace: ReplaceOnly,
}

#[derive(clap::Args)]
struct CompressArgs {
    #[arg(value_name = "DOC.pdf", value_hint = clap::ValueHint::FilePath,
        required = true,
        help = DOC_HELP)]
    doc: Option<String>,
    #[arg(value_name = "OUT.pdf", value_hint = clap::ValueHint::FilePath,
        help = "Output PDF (default: <name>-compressed.pdf)")]
    out_pos: Option<String>,
    #[command(flatten)]
    flags: CompressFlags,
    #[command(flatten)]
    common: Common,
}

/// Password channels for encrypt/decrypt: private files only — never
/// inline argv. With neither flag on a terminal, the dispatcher prompts
/// hidden on /dev/tty.
#[derive(clap::Args)]
struct UserSecret {
    #[arg(long, value_name = "file", allow_hyphen_values = true,
        value_hint = clap::ValueHint::FilePath,
        help_heading = CREDENTIAL_HEADING,
        help = "Read the password from a private file (chmod 600); on a terminal, omit for a hidden prompt")]
    password_file: Option<String>,
}

#[derive(clap::Args)]
struct OwnerSecret {
    #[arg(long, value_name = "file", allow_hyphen_values = true,
        value_hint = clap::ValueHint::FilePath,
        help_heading = CREDENTIAL_HEADING,
        help = "Read the optional owner password from a private file; a blank prompt or omission generates a random owner")]
    owner_password_file: Option<String>,
}

doc_args!(EncryptArgs {
    #[arg(value_name = "DOC.pdf", value_hint = clap::ValueHint::FilePath,
        required = true,
        help = DOC_HELP)]
    doc: Option<String>,
    #[arg(value_name = "OUT.pdf", value_hint = clap::ValueHint::FilePath,
        help = "Output PDF (or -o/--output; default: <name>-encrypted.pdf)")]
    out_pos: Option<String>,
}, out: Out, user: UserSecret, owner: OwnerSecret, replace: ReplaceOnly, preserve: Preserve);

doc_args!(DecryptArgs {
    #[arg(value_name = "DOC.pdf", value_hint = clap::ValueHint::FilePath,
        required = true,
        help = DOC_HELP)]
    doc: Option<String>,
    #[arg(value_name = "OUT.pdf", value_hint = clap::ValueHint::FilePath,
        help = "Output PDF (or -o/--output; default: <name>-decrypted.pdf)")]
    out_pos: Option<String>,
}, out: Out, user: UserSecret, replace: ReplaceOnly, preserve: Preserve);

#[derive(clap::Args)]
struct ServeArgs {
    #[arg(
        long,
        value_name = "port",
        allow_hyphen_values = true,
        help_heading = SERVER_HEADING,
        help = "Loopback HTTP port (default: 8787)"
    )]
    port: Option<String>,
    #[arg(
        long,
        value_name = "file",
        allow_hyphen_values = true,
        value_hint = clap::ValueHint::FilePath,
        help_heading = SERVER_HEADING,
        help = "Bearer token file: a private file holding >=32 ASCII graphic characters (required)"
    )]
    token_file: Option<PathBuf>,
    #[command(flatten)]
    backends: Backends,
}

#[derive(clap::Args)]
struct McpArgs {
    #[arg(
        long,
        value_name = "PATH",
        help_heading = SAFETY_HEADING,
        help = "Allow new output only at this workspace-relative path; disallow replacement"
    )]
    output_only: Option<String>,
    #[arg(long, requires = "output_only", value_name = "BYTES", value_parser = clap::value_parser!(u64).range(1..=250 * 1024 * 1024), help_heading = SAFETY_HEADING, help = "Maximum published bytes at --output-only (integer bytes)")]
    output_byte_ceiling: Option<u64>,
    #[arg(
        long,
        requires = "output_only",
        value_name = "OPERATION_ID",
        help_heading = SAFETY_HEADING,
        help = "Allow only this operation to write the approved output"
    )]
    output_operation: Option<String>,
    #[command(flatten)]
    backends: Backends,
}

#[derive(clap::Args)]
struct OperationsArgs {
    #[command(flatten)]
    backends: Backends,
}

#[derive(clap::Args)]
struct SchemaArgs {
    #[arg(
        value_name = "operationId",
        required = true,
        help = "Operation ID — see 'anpdf operations'"
    )]
    operation_id: String,
}

#[derive(clap::Args)]
struct CompletionsArgs {
    #[arg(
        value_name = "shell",
        required = true,
        help = "Shell: bash, zsh or fish"
    )]
    shell: String,
}

#[derive(clap::Args)]
struct HelpArgs {
    #[arg(value_name = "command", help = "Command name")]
    topic: Option<String>,
}

#[derive(clap::Subcommand)]
enum Cmd {
    #[command(about = "Inspect PDF structure, metadata, and page geometry",
        help_template = SUB_HELP_TEMPLATE,
        after_help = "Alias: validate (operationId: inspectPdf)\nDefault parses structure and metadata; --deep-check (or \"deepCheck\":true) decodes stream payloads.\nRequest: {\"input\":{\"path\":\"doc.pdf\"}}")]
    Inspect(InspectArgs),
    #[command(about = "Alias for inspect",
        help_template = SUB_HELP_TEMPLATE,
        after_help = "Alias for inspectPdf.\nRequest: {\"input\":{\"path\":\"doc.pdf\"}}")]
    Validate(InspectArgs),
    #[command(about = "Merge 2..20 PDFs in input order",
        help_template = SUB_HELP_TEMPLATE,
        after_help = "Operation: mergePdf. The first input is primary for preservation. Use -o to mark the output; without it, the last positional is the output.\nExample: anpdf merge cover.pdf report.pdf -o packet.pdf\nRequest: {\"inputs\":[{\"path\":\"a.pdf\"},{\"path\":\"b.pdf\"}],\"output\":{\"path\":\"merged.pdf\"}}")]
    Merge(MergeArgs),
    #[command(about = "Extract an ascending unique page subset",
        help_template = SUB_HELP_TEMPLATE,
        after_help = "Operation: extractPdf (strictly increasing 1-based pages; e.g. \"1,3\" or \"2-5\")\nRequest: {\"input\":{\"path\":\"doc.pdf\"},\"pages\":[1,3],\"output\":{\"path\":\"subset.pdf\"}}")]
    Extract(ExtractArgs),
    #[command(about = "Split a PDF into 2..20 page-subset outputs",
        help_template = SUB_HELP_TEMPLATE,
        after_help = "Operation: splitPdf (2..20 parts); each part is <output>=<pages>. Parts can overlap or leave pages unused.\nExample: anpdf split report.pdf cover.pdf=1 body.pdf=2-5\nRequest: {\"input\":{\"path\":\"doc.pdf\"},\"parts\":[{\"pages\":[1],\"output\":{\"path\":\"p1.pdf\"}},{\"pages\":[2,3],\"output\":{\"path\":\"p2.pdf\"}}]}")]
    Split(SplitArgs),
    #[command(about = "Remove selected pages and keep the remaining pages",
        help_template = SUB_HELP_TEMPLATE,
        after_help = "Operation: removePages (strictly increasing 1-based pages to remove; at least 1 page must remain)\nRequest: {\"input\":{\"path\":\"doc.pdf\"},\"pages\":[2],\"output\":{\"path\":\"trimmed.pdf\"}}")]
    RemovePages(RemoveArgs),
    #[command(about = "Write every page in the requested output order",
        help_template = SUB_HELP_TEMPLATE,
        after_help = "Operation: reorderPages. The order must cover every input page exactly once;\nranges expand ascending, so use commas for a custom order.\nExample: anpdf reorder report.pdf 3,1,2 -o ordered.pdf\nRequest: {\"input\":{\"path\":\"doc.pdf\"},\"pages\":[3,1,2],\"output\":{\"path\":\"ordered.pdf\"}}")]
    Reorder(ReorderArgs),
    #[command(about = "Compress PDF streams, images, or eligible traced pages",
        help_template = SUB_HELP_TEMPLATE,
        after_help = if cfg!(feature = "lossless-only") {
            "This build permits lossless compression only. With --max-bytes, set\n--mode lossless explicitly and use --on-target-miss error for a hard limit.\n\nExample:\n  anpdf compress input.pdf -o output.pdf --mode lossless --max-bytes 3MB --on-target-miss error\n\nJSON schema: anpdf schema compressPdf"
        } else { "Examples:
  anpdf compress input.pdf
  anpdf compress input.pdf -o output.pdf --mode lossless --max-bytes 3MB --on-target-miss error
  anpdf compress input.pdf -o output.pdf --mode lossy --max-bytes 3MB

Modes:
  lossless  Recompress streams without degrading images (default without a
            size target or --level 2/3). An explicit choice stays lossless
            with any target and never rasterizes pages.
  lossy     Recompress images. With a target, try lower resolutions and, if
            needed, rasterize eligible traced pages. Levels 2 and 3 limit
            how far image resolution can drop.
  layered   With PDFium, rebuild eligible traced pages at 400 DPI using
            separate ink, grayscale, and color layers. Rebuilt pages lose
            vector detail. No lower-resolution retries; --level and
            --jpeg-quality are not accepted.

With no mode or level, --max-bytes can enable lossy processing for v1
compatibility. Choose a mode explicitly. Use --on-target-miss error if
the byte limit is mandatory; a miss then leaves the destination untouched.

JSON schema: anpdf schema compressPdf" })]
    Compress(CompressArgs),
    #[command(about = "Extract embedded text and bounding boxes (no OCR)",
        help_template = SUB_HELP_TEMPLATE,
        after_help = "Operation: extractText. Requires a configured text engine; scan-only pages can return empty text.\nRequest: {\"input\":{\"path\":\"doc.pdf\"},\"pages\":[1,3]}")]
    ExtractText(ExtractTextArgs),
    #[command(about = "Render one page to PNG (optional engine)",
        help_template = SUB_HELP_TEMPLATE,
        after_help = "Operation: renderPage. Writes a PNG with crop and rotation applied, without annotations. Requires a configured rendering engine.\nRequest: {\"input\":{\"path\":\"doc.pdf\"},\"page\":1,\"output\":{\"path\":\"page.png\"}}")]
    RenderPage(RenderArgs),
    #[command(about = "Encrypt a PDF with AES-256 (user password, optional owner)",
        help_template = SUB_HELP_TEMPLATE,
        after_help = "Operation: encryptPdf; output defaults to <name>-encrypted.pdf.\nPasswords never go inline — omit --password-file on a terminal for a hidden\nprompt (a bare Enter sets an intentionally empty password), or use a\nprivate file, --request-file or piped stdin JSON fields \"password\"/\n\"ownerPassword\"; an omitted owner — or a blank prompt answer — generates\na random one.\nRequest: {\"input\":{\"path\":\"doc.pdf\"},\"output\":{\"path\":\"locked.pdf\"}}")]
    Encrypt(EncryptArgs),
    #[command(about = "Decrypt a password-protected PDF (password required)",
        help_template = SUB_HELP_TEMPLATE,
        after_help = "Operation: decryptPdf; writes an unencrypted copy (default: <name>-decrypted.pdf).\nOmit --password-file on a terminal for a hidden prompt, or use a private\n--password-file, --request-file or piped stdin. Never put a password in argv.\nExample: anpdf decrypt locked.pdf -o open.pdf --password-file ./private-password\nJSON schema: anpdf schema decryptPdf")]
    Decrypt(DecryptArgs),
    #[command(about = "Run authenticated loopback HTTP server (127.0.0.1)",
        help_template = SUB_HELP_TEMPLATE,
        after_help = "--token-file is required: a private file holding >=32 ASCII graphic characters.")]
    Serve(ServeArgs),
    #[command(about = "Run newline-delimited JSON-RPC 2.0 MCP server on stdio",
        help_template = SUB_HELP_TEMPLATE)]
    Mcp(McpArgs),
    #[command(about = "Print supported operations, schemas, limits, and unsupported list",
        help_template = SUB_HELP_TEMPLATE)]
    Operations(OperationsArgs),
    #[command(about = "Print JSON schemas for one operation",
        help_template = SUB_HELP_TEMPLATE)]
    Schema(SchemaArgs),
    #[command(about = "Print shell completion script (bash, zsh, fish)",
        help_template = SUB_HELP_TEMPLATE)]
    Completions(CompletionsArgs),
    #[command(about = "Show this help text (--help, -h)",
        help_template = SUB_HELP_TEMPLATE)]
    Help(HelpArgs),
    #[command(about = "Show version (--version, -V)")]
    Version,
}

#[derive(clap::Parser)]
#[command(
    name = "anpdf",
    version,
    about = "Local, contract-validated PDF toolkit (CLI, loopback HTTP, MCP stdio).",
    disable_help_flag = false,
    disable_help_subcommand = true,
    disable_version_flag = true,
    help_template = HELP_TEMPLATE,
    after_help = TOP_AFTER_HELP,
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

/// Flags the old parser knew in its global table — used to distinguish
/// "not valid for '<cmd>'" (known flag, wrong command) from "not a
/// recognized option" (unknown flag) on clap's UnknownArgument errors.
/// Flags that consume the next argv token as their value.
const VALUE_FLAGS: &[&str] = &[
    "-o",
    "--output",
    "--pages",
    "--preserve",
    "--root",
    "--qpdf",
    "--pdfium-lib",
    "--request-json",
    "--request-file",
    "--mode",
    "--jpeg-quality",
    "--level",
    "--max-bytes",
    "--on-target-miss",
    "--engine",
    "--dpi",
    "--port",
    "--token-file",
    "--password-file",
    "--owner-password-file",
];

const KNOWN_FLAGS: &[&str] = &[
    "--root",
    "--qpdf",
    "--qpdf-exact",
    "--pdfium-lib",
    "--request-json",
    "--request-file",
    "--dry-run",
    "--json",
    "--output",
    "-o",
    "--replace",
    "--preserve",
    "--mode",
    "--jpeg-quality",
    "--level",
    "--max-bytes",
    "--on-target-miss",
    "--allow-larger",
    "--engine",
    "--dpi",
    "--pages",
    "--deep-check",
    "--port",
    "--token-file",
    "--password-file",
    "--owner-password-file",
    "--help",
];

/// Commands that accept no positional shorthand arguments.
const NO_POSITIONALS: &[&str] = &["serve", "mcp", "operations", "schema", "completions"];

/// All clap subcommand names (KNOWN_COMMANDS plus help/version).
const SUBCOMMANDS: &[&str] = &[
    "inspect",
    "validate",
    "merge",
    "extract",
    "split",
    "remove-pages",
    "reorder",
    "compress",
    "extract-text",
    "render-page",
    "encrypt",
    "decrypt",
    "serve",
    "mcp",
    "operations",
    "schema",
    "completions",
    "help",
    "version",
];

/// Every `'x'` quoted substring in a clap error message, in order.
fn quoted(text: &str) -> Vec<String> {
    text.split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// clap's "tip: a similar argument exists: 'x'" suggestion, as
/// "(did you mean 'x'?)". With several candidates, keep the one
/// sharing the longest prefix with the rejected `needle`.
fn suggestion(text: &str, needle: &str) -> String {
    for line in text.lines() {
        let line = line.trim_start();
        if !line.starts_with("tip:") {
            continue;
        }
        let best = quoted(line)
            .into_iter()
            .filter(|v| !v.contains(' '))
            .max_by_key(|v| {
                needle
                    .chars()
                    .zip(v.chars())
                    .take_while(|(a, b)| a == b)
                    .count()
            });
        if let Some(v) = best {
            return format!(" (did you mean '{v}'?)");
        }
    }
    String::new()
}

/// Map a clap parse error onto this CLI's error codes and detail strings:
/// the old hand-rolled parser's messages stay stable for callers and tests.
fn clap_error(command: &str, e: clap::Error) -> Error {
    use clap::error::ErrorKind;
    let text = e.to_string();
    let q = quoted(&text);
    if !SUBCOMMANDS.contains(&command) {
        return Error(
            "UNSUPPORTED_OPERATION",
            Some(format!(
                "unknown command '{command}' — see 'anpdf help'{}",
                suggestion(&text, command)
            )),
        );
    }
    match e.kind() {
        ErrorKind::InvalidSubcommand => {
            let value = q.first().map(String::as_str).unwrap_or(command);
            Error(
                "UNSUPPORTED_OPERATION",
                Some(format!(
                    "unknown command '{value}' — see 'anpdf help'{}",
                    suggestion(&text, value)
                )),
            )
        }
        ErrorKind::ArgumentConflict | ErrorKind::TooManyValues => {
            let flag = q.first().map(String::as_str).unwrap_or("flag");
            let flag = flag.split(' ').next().unwrap_or(flag);
            bad(format!("{flag} given twice"))
        }
        ErrorKind::TooFewValues => {
            if command == "split" {
                return bad(
                    "split expects <doc.pdf> and 2..20 parts as <part.pdf=pages> — see 'anpdf help split'",
                );
            }
            let flag = q
                .first()
                .map(String::as_str)
                .unwrap_or("flag")
                .split(' ')
                .next()
                .unwrap_or("flag")
                .to_string();
            need(&flag)
        }
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => {
            // "a value is required for '--flag <META>' but none was supplied"
            // or "invalid value 'x' for '--flag <META>'".
            if text.contains("a value is required") {
                let flag = q
                    .first()
                    .map(String::as_str)
                    .unwrap_or("flag")
                    .split(' ')
                    .next()
                    .unwrap_or("flag")
                    .to_string();
                return need(&flag);
            }
            let value = q.first().map(String::as_str).unwrap_or("");
            let flag = q
                .get(1)
                .map(String::as_str)
                .unwrap_or("value")
                .split(' ')
                .next()
                .unwrap_or("value")
                .to_string();
            let hint = suggestion(&text, value);
            match flag.as_str() {
                "--level" => bad("--level expects 1, 2 or 3"),
                "--mode" => bad(format!(
                    "invalid --mode '{value}' — expected lossless|lossy|layered{hint}"
                )),
                "--on-target-miss" => bad(format!(
                    "invalid --on-target-miss '{value}' — expected bestEffort|error{hint}"
                )),
                _ => bad(format!("invalid {flag} '{value}'{hint}")),
            }
        }
        ErrorKind::MissingRequiredArgument => bad(match command {
            "completions" => "completions expects a shell: bash, zsh or fish".to_string(),
            "schema" => "schema expects an operationId — see 'anpdf operations'".to_string(),
            "serve" => "serve requires --token-file <file> — see 'anpdf help serve'".to_string(),
            _ => format!(
                "{} expects required arguments — see 'anpdf help {}'",
                command, command
            ),
        }),
        _ => {
            let arg = q.first().map(String::as_str).unwrap_or("");
            if arg.starts_with('-') {
                let hint = suggestion(&text, arg);
                if KNOWN_FLAGS.contains(&arg) {
                    if arg == "--owner-password-file" {
                        bad(format!(
                            "{arg} applies to encrypt only — see 'anpdf help {command}'"
                        ))
                    } else if arg == "--password-file" {
                        bad(format!(
                            "{arg} is only valid for encrypt and decrypt — see 'anpdf help {command}'"
                        ))
                    } else {
                        bad(format!(
                            "{arg} is not valid for '{command}' — see 'anpdf help {command}'{hint}"
                        ))
                    }
                } else {
                    bad(format!(
                        "{arg} is not a recognized option for '{command}' — see 'anpdf help {command}'{hint}"
                    ))
                }
            } else if !arg.is_empty() {
                match command {
                    "help" => bad("help takes at most one command topic"),
                    "completions" => bad("unexpected argument after the shell name"),
                    _ if NO_POSITIONALS.contains(&command) => bad(format!(
                        "'{command}' takes no positional arguments — pipe a JSON request or use --request-json"
                    )),
                    _ => bad(format!("unexpected argument '{arg}'")),
                }
            } else {
                bad(text
                    .lines()
                    .next()
                    .unwrap_or("invalid arguments")
                    .trim_start_matches("error: ")
                    .to_string())
            }
        }
    }
}

/// Byte-size values for --max-bytes: a bare integer is bytes; SI units
/// (k, kB, KB, MB, GB, TB) are decimal ×1000; IEC units (KiB, MiB, GiB,
/// TiB) are binary ×1024.
fn parse_size(spec: &str) -> Result<u64> {
    let invalid = || {
        bad(format!(
            "--max-bytes expects a byte count or size like '3MB', '3MiB' or '500k' (SI units are decimal), got '{spec}'"
        ))
    };
    let s = spec.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'));
    let (num, unit) = match split {
        Some(i) => (s[..i].trim(), s[i..].trim()),
        None => (s, ""),
    };
    let value: f64 = num.parse().map_err(|_| invalid())?;
    if !value.is_finite() || value < 0.0 || num.is_empty() {
        return Err(invalid());
    }
    let multiplier: f64 = match unit {
        "" | "B" | "b" => 1.0,
        "k" | "K" | "kB" | "KB" | "kb" => 1e3,
        "MB" | "mb" | "mB" | "Mb" => 1e6,
        "GB" | "gb" | "gB" | "Gb" => 1e9,
        "TB" | "tb" | "tB" | "Tb" => 1e12,
        "KiB" | "kiB" | "kib" | "KIB" => 1024.0,
        "MiB" | "miB" | "mib" | "MIB" => 1024.0f64.powi(2),
        "GiB" | "giB" | "gib" | "GIB" => 1024.0f64.powi(3),
        "TiB" | "tiB" | "tib" | "TIB" => 1024.0f64.powi(4),
        _ => return Err(invalid()),
    };
    let bytes = (value * multiplier).round();
    if bytes < 0.0 || bytes > u64::MAX as f64 {
        return Err(invalid());
    }
    Ok(bytes as u64)
}

/// Task groups shown in the top-level command list. Commands themselves
/// remain flat: users still type `anpdf reorder`, not a nested action path.
const COMMAND_GROUPS: &[(&str, &[&str])] = &[
    ("Inspect", &["inspect", "validate"]),
    (
        "Organize pages",
        &["merge", "extract", "split", "remove-pages", "reorder"],
    ),
    ("Reduce size", &["compress"]),
    ("Read content", &["extract-text", "render-page"]),
    ("Protect", &["encrypt", "decrypt"]),
    ("Serve", &["serve", "mcp"]),
    (
        "Discover",
        &["operations", "schema", "completions", "help", "version"],
    ),
];

/// Render help for `topic` (or the overview) from the clap grammar —
/// aligned columns, wrapped lines, per-flag help; plain text. The overview's
/// command inventory is grouped by task while keeping the command syntax flat.
fn usage(topic: Option<&str>) -> String {
    let mut command = Cli::command();
    let text = match topic {
        Some(name) if name != "help" => command
            .get_subcommands_mut()
            .find(|c| c.get_name() == name)
            .map(|c| c.render_help())
            .unwrap_or_else(|| Cli::command().render_help()),
        _ => command.render_help(),
    };
    let mut text = text.to_string();
    if topic.is_none() || topic == Some("help") {
        text = group_command_list(text, &command);
    }
    text
}

/// Replace clap's flat overview inventory with task groups, taking each
/// one-line description from the same clap grammar rather than duplicating it.
fn group_command_list(text: String, command: &clap::Command) -> String {
    let Some(start) = text.find("Commands:\n") else {
        return text;
    };
    let Some(options) = text[start..].find("\nOptions:").map(|i| start + i) else {
        return text;
    };
    let mut grouped = String::new();
    for (heading, names) in COMMAND_GROUPS {
        grouped.push_str(heading);
        grouped.push_str(":\n");
        for name in *names {
            let Some(subcommand) = command.get_subcommands().find(|c| c.get_name() == *name) else {
                continue;
            };
            let about = subcommand
                .get_about()
                .map(|a| a.to_string())
                .unwrap_or_default();
            grouped.push_str(&format!("  {name:<14}{about}\n"));
        }
        grouped.push('\n');
    }
    while grouped.ends_with("\n\n") {
        grouped.pop();
    }
    format!("{}{}{}", &text[..start], grouped, &text[options..])
}

fn is_executable_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && (m.mode() & 0o111 != 0))
        .unwrap_or(false)
}

fn is_trusted_dir(dir: &Path) -> bool {
    if !dir.is_absolute() {
        return false;
    }
    let Ok(m) = std::fs::metadata(dir) else {
        return false;
    };
    m.is_dir() && (m.mode() & 0o022 == 0) && (m.uid() == unsafe { libc::geteuid() } || m.uid() == 0)
}

fn resolve_qpdf(
    flag: Option<PathBuf>,
    env_qpdf: Option<OsString>,
    path_env: Option<OsString>,
) -> Result<PathBuf> {
    if let Some(p) = flag {
        if !p.is_absolute() {
            return Err(Error("INVALID_REQUEST", None));
        }
        return Ok(p);
    }
    if let Some(val) = env_qpdf.filter(|v| !v.is_empty()) {
        let p = PathBuf::from(val);
        if !p.is_absolute() {
            return Err(Error("INVALID_REQUEST", None));
        }
        return Ok(p);
    }
    for candidate in STANDARD_QPDF_PATHS.iter().map(Path::new) {
        if is_executable_file(candidate) {
            return Ok(candidate.to_path_buf());
        }
    }
    if let Some(paths) = path_env {
        for dir in std::env::split_paths(&paths) {
            if is_trusted_dir(&dir) {
                let candidate = dir.join("qpdf");
                if is_executable_file(&candidate) {
                    return Ok(candidate);
                }
            }
        }
    }
    Ok(PathBuf::from("/usr/bin/qpdf"))
}

fn resolve_pdfium(
    flag: Option<PathBuf>,
    env_library: Option<OsString>,
    executable: Option<PathBuf>,
) -> Result<Option<PathBuf>> {
    if let Some(path) = flag.or_else(|| env_library.filter(|v| !v.is_empty()).map(PathBuf::from)) {
        if !path.is_absolute() {
            return Err(Error("INVALID_REQUEST", None));
        }
        return Ok(path.is_file().then_some(path));
    }
    let Some(executable) = executable.and_then(|p| p.canonicalize().ok()) else {
        return Ok(None);
    };
    let Some(dir) = executable.parent() else {
        return Ok(None);
    };
    let library = if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else {
        "libpdfium.so"
    };
    for candidate in [dir.join("../lib/anpdf").join(library), dir.join(library)] {
        if let Ok(path) = candidate.canonicalize()
            && path.is_file()
            && path.parent().is_some_and(is_trusted_dir)
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

const KNOWN_COMMANDS: &[&str] = &[
    "inspect",
    "validate",
    "merge",
    "extract",
    "split",
    "remove-pages",
    "reorder",
    "compress",
    "extract-text",
    "render-page",
    "encrypt",
    "decrypt",
    "serve",
    "mcp",
    "operations",
    "schema",
    "completions",
];

/// Request-shaping options collected alongside positional shorthand arguments.
#[derive(Default)]
struct CliArgs {
    output: Option<String>,
    replace: bool,
    preserve: Option<Vec<String>>,
    mode: Option<String>,
    jpeg_quality: Option<u64>,
    level: Option<u64>,
    max_bytes: Option<u64>,
    on_miss: Option<String>,
    allow_larger: bool,
    engine: Option<String>,
    dpi: Option<u64>,
    pages: Option<String>,
    deep_check: bool,
}

impl CliArgs {
    fn shapes_request(&self) -> bool {
        self.output.is_some()
            || self.replace
            || self.preserve.is_some()
            || self.mode.is_some()
            || self.jpeg_quality.is_some()
            || self.level.is_some()
            || self.max_bytes.is_some()
            || self.on_miss.is_some()
            || self.allow_larger
            || self.engine.is_some()
            || self.dpi.is_some()
            || self.pages.is_some()
            || self.deep_check
    }
}

/// INVALID_REQUEST carrying a human-facing detail that names the flag,
/// field or shape that failed.
fn bad(detail: impl Into<String>) -> Error {
    Error("INVALID_REQUEST", Some(detail.into()))
}
fn need(flag: &str) -> Error {
    bad(format!("{flag} expects a value"))
}

/// Parse a `<pages>` spec: comma-separated 1-based numbers or `a-b` ranges
/// ("1,3", "2-5"). Ordering/duplicates are validated by the contract.
fn parse_page_list(spec: &str) -> Result<Vec<u64>> {
    let invalid = || {
        bad(format!(
            "invalid pages '{spec}' — expected 1-based numbers or ranges like '1,3' or '2-5', at most {MAX_PAGES} pages"
        ))
    };
    // Widest `pages` bound in the contract (extractPdf/removePages/splitPdf);
    // exceeding it is always INVALID_REQUEST, so fail before allocating.
    const MAX_PAGES: u64 = 2000;
    let mut pages = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        match part.split_once('-') {
            Some((start, end)) => {
                let start: u64 = start.trim().parse().map_err(|_| invalid())?;
                let end: u64 = end.trim().parse().map_err(|_| invalid())?;
                if start > end || end - start >= MAX_PAGES {
                    return Err(invalid());
                }
                for page in start..=end {
                    if pages.len() as u64 >= MAX_PAGES {
                        return Err(invalid());
                    }
                    pages.push(page);
                }
            }
            None => {
                if pages.len() as u64 >= MAX_PAGES {
                    return Err(invalid());
                }
                pages.push(part.parse().map_err(|_| invalid())?);
            }
        }
    }
    if pages.is_empty() {
        Err(invalid())
    } else {
        Ok(pages)
    }
}

fn output_value(path: &str, replace: bool) -> Value {
    if replace {
        serde_json::json!({"path": path, "replaceExisting": true})
    } else {
        serde_json::json!({"path": path})
    }
}

fn preserve_value(list: &[String]) -> Result<Value> {
    const ASPECTS: [&str; 4] = ["metadata", "bookmarks", "annotations", "attachments"];
    let mut map = serde_json::Map::new();
    for name in list {
        if name == "all" {
            for aspect in ASPECTS {
                map.insert(aspect.into(), true.into());
            }
            continue;
        }
        if !ASPECTS.contains(&name.as_str()) {
            return Err(bad(format!(
                "--preserve aspect '{name}' is unknown (expected metadata|bookmarks|annotations|attachments|all)"
            )));
        }
        map.insert(name.clone(), true.into());
    }
    Ok(map.into())
}

/// Expand a leading `~` or `~/` to $HOME for human-typed shorthand paths.
fn expand_home(path: &str) -> Result<String> {
    if path == "~" {
        let home =
            std::env::var_os("HOME").ok_or(bad("'~' requires the HOME environment variable"))?;
        Ok(PathBuf::from(home).to_string_lossy().into_owned())
    } else if let Some(rest) = path.strip_prefix("~/") {
        let home =
            std::env::var_os("HOME").ok_or(bad("'~' requires the HOME environment variable"))?;
        Ok(PathBuf::from(home)
            .join(rest)
            .to_string_lossy()
            .into_owned())
    } else {
        Ok(path.into())
    }
}

/// Canonical path of the directory containing `path` (the file itself need
/// not exist — output paths are canonicalized by their parent directory).
fn canonical_dir(path: &Path) -> Result<PathBuf> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| Error::io(&e, "current directory"))?
            .join(path)
    };
    let dir = abs
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    std::fs::canonicalize(&dir)
        .map_err(|e| Error::io(&e, format!("directory for '{}'", path.display())))
}

/// Collapse `.`/`..` without touching the filesystem, so a typed path can
/// be compared with its canonical form to expose symlinked directories.
fn normalize_components(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// Does `s` look like a <pages> spec rather than a file path?
fn looks_like_pages(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_digit() || matches!(c, ',' | '-' | ' '))
}

/// Longest common directory prefix of two canonical paths.
fn common_ancestor(a: PathBuf, b: &Path) -> PathBuf {
    a.components()
        .zip(b.components())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.as_os_str())
        .collect()
}

/// Human-typed shorthand paths may be absolute or start with `~`. They are
/// rewritten to workspace-relative specs inside the dispatcher's sandbox:
/// with an explicit --root they must resolve inside it; otherwise the
/// effective root adapts to the files' common directory for this
/// invocation. JSON requests, HTTP and MCP are unaffected — their paths
/// stay workspace-relative to --root as before.
fn normalize_shorthand(
    command: &str,
    positionals: &mut [String],
    cli: &mut CliArgs,
    root: &mut PathBuf,
    root_explicit: bool,
) -> Result<()> {
    // Path-bearing slot of a positional: for split's `<name.pdf>=<pages>`
    // only the part name is a path.
    let path_part = |i: usize, raw: &str| -> String {
        if command == "split" && i > 0 {
            raw.split_once('=').map_or(raw, |(p, _)| p).into()
        } else {
            raw.into()
        }
    };
    // Positional slots that carry a file path, per command. Page specs
    // (`extract f.pdf 1-3`) and page numbers (`render-page f.pdf 2`) are
    // not paths and must never be rewritten as workspace files.
    let path_slot = |i: usize, raw: &str| -> bool {
        match command {
            "extract" | "remove-pages" | "reorder" | "extract-text" if i == 1 => {
                // [1] is the pages spec — except under --pages, where a
                // non-pages-shaped value there is the output.
                cli.pages.is_some() && !looks_like_pages(raw)
            }
            "render-page" if i == 1 => false,
            _ => true,
        }
    };
    let expanded: Vec<String> = positionals
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if path_slot(i, p) {
                expand_home(&path_part(i, p))
            } else {
                Ok(p.clone())
            }
        })
        .collect::<Result<_>>()?;
    let expanded_output = cli.output.as_deref().map(expand_home).transpose()?;
    if expanded.iter().all(|p| !p.starts_with('/'))
        && expanded_output.as_ref().is_none_or(|p| !p.starts_with('/'))
    {
        // Nothing outside the workspace conventions — write back expanded
        // (covers a bare '~') and leave --root untouched.
        for (i, p) in positionals.iter_mut().enumerate() {
            if command == "split" && i > 0 {
                if let Some((_, pages)) = p.split_once('=') {
                    *p = format!("{}={pages}", expanded[i]);
                }
            } else {
                *p = expanded[i].clone();
            }
        }
        cli.output = expanded_output;
        return Ok(());
    }
    let canon = |p: &String| -> Result<PathBuf> {
        if p.ends_with('/') {
            return Err(bad(format!(
                "path '{p}' ends with '/' — directories are not valid file arguments"
            )));
        }
        // The workspace opens every path component with O_NOFOLLOW;
        // canonicalizing a symlinked parent here would erase it before
        // that check runs. Require the typed path to match its canonical
        // form so no component is a symlink.
        let raw = Path::new(p);
        let abs = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| Error::io(&e, "current directory"))?
                .join(raw)
        };
        let lexical = normalize_components(&abs);
        let lexical_parent = lexical
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/"));
        let dir = std::fs::canonicalize(&lexical_parent)
            .map_err(|e| Error::io(&e, format!("directory for '{p}'")))?;
        if dir != lexical_parent {
            return Err(Error("ACCESS_DENIED", None).with(format!(
                "path '{p}' traverses a symlink — path components must be real directories"
            )));
        }
        let name = lexical
            .file_name()
            .ok_or(bad(format!("path '{p}' has no file name")))?;
        Ok(dir.join(name))
    };
    let relativize = |canon_root: &Path, explicit: bool, p: &String| -> Result<String> {
        // With an explicit --root, relative paths are already
        // workspace-relative and pass through unchanged. When the root
        // adapts to the files' common directory, relative paths resolve
        // against the cwd and must be relativized like absolute ones.
        if explicit && !p.starts_with('/') {
            return Ok(p.clone());
        }
        let full = canon(p)?;
        let rel = full.strip_prefix(canon_root).map_err(|_| {
            bad(format!(
                "path '{p}' is outside the workspace root '{}'",
                canon_root.display()
            ))
        })?;
        Ok(rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"))
    };
    let canon_root: PathBuf = if root_explicit {
        std::fs::canonicalize(&*root)
            .map_err(|e| Error::io(&e, format!("--root '{}'", root.display())))?
    } else {
        // The effective root is the common ancestor of every referenced
        // file's directory (relative paths resolve against the cwd).
        let mut eff: Option<PathBuf> = None;
        let path_values = expanded
            .iter()
            .enumerate()
            .filter(|(i, _)| path_slot(*i, &positionals[*i]))
            .map(|(_, p)| p)
            .chain(expanded_output.iter());
        for p in path_values {
            let dir = canonical_dir(Path::new(p))?;
            eff = Some(match eff {
                Some(e) => common_ancestor(e, &dir),
                None => dir,
            });
        }
        let eff = eff.unwrap_or_else(|| PathBuf::from("."));
        *root = eff.clone();
        eff
    };
    for (i, p) in positionals.iter_mut().enumerate() {
        if !path_slot(i, p) {
            continue;
        }
        let rel = relativize(&canon_root, root_explicit, &expanded[i])?;
        if command == "split" && i > 0 {
            if let Some((_, pages)) = p.split_once('=') {
                *p = format!("{rel}={pages}");
            }
        } else {
            *p = rel;
        }
    }
    if let Some(out) = expanded_output {
        cli.output = Some(relativize(&canon_root, root_explicit, &out)?);
    }
    Ok(())
}

/// Derived compress output: "dir/name.pdf" -> "dir/name-compressed.pdf".
fn derived_output(input: &str, suffix: &str) -> String {
    let basename_start = input.rfind('/').map_or(0, |i| i + 1);
    match input.rfind('.') {
        Some(dot) if dot > basename_start => {
            format!("{}-{suffix}{}", &input[..dot], &input[dot..])
        }
        _ => format!("{input}-{suffix}"),
    }
}

fn derived_compress_output(input: &str) -> String {
    derived_output(input, "compressed")
}

/// Build the operation request from positional shorthand arguments.
fn request_from_positionals(command: &str, positionals: &[String], cli: &CliArgs) -> Result<Value> {
    let arity = |expected: &str| -> Error {
        bad(format!(
            "{command} expects {expected} — see 'anpdf help {command}'"
        ))
    };
    let output_for = |tail: Option<&String>| -> Result<Value> {
        match (&cli.output, tail) {
            (Some(_), Some(_)) => Err(bad(
                "output given twice: -o/--output and a positional output are exclusive",
            )),
            (Some(output), None) => Ok(output_value(output, cli.replace)),
            (None, Some(path)) => Ok(output_value(path, cli.replace)),
            (None, None) => Err(arity(
                "an output path (-o/--output or a trailing positional)",
            )),
        }
    };
    let preserve = cli.preserve.as_deref().map(preserve_value).transpose()?;
    // Pages may come from --pages or a positional spec; both is ambiguous.
    let pages_for = |positional: Option<&String>| -> Result<Value> {
        match (&cli.pages, positional) {
            (Some(_), Some(pos)) if looks_like_pages(pos.as_str()) => {
                Err(bad("pages given both positionally and via --pages"))
            }
            (Some(spec), _) => parse_page_list(spec).map(|p| p.into()),
            (None, Some(pos)) => parse_page_list(pos).map(|p| p.into()),
            (None, None) => Err(arity(
                "a <pages> spec (e.g. '1,3' or '2-5', positional or --pages)",
            )),
        }
    };
    let mut request = match command {
        "inspect" | "validate" => {
            if positionals.len() != 1 {
                return Err(arity("exactly one <doc.pdf>"));
            }
            let mut request = serde_json::json!({"input": {"path": positionals[0]}});
            if cli.deep_check {
                request["deepCheck"] = true.into();
            }
            request
        }
        "merge" => {
            let (inputs, output) = if cli.output.is_some() {
                (positionals, output_for(None)?)
            } else {
                if positionals.len() < 3 {
                    return Err(arity(
                        "2..20 input PDFs plus an output (-o/--output or a trailing positional)",
                    ));
                }
                let (inputs, tail) = positionals.split_at(positionals.len() - 1);
                (inputs, output_for(Some(&tail[0]))?)
            };
            if !(2..=20).contains(&inputs.len()) {
                return Err(bad(format!(
                    "merge expects 2..20 inputs, got {}",
                    inputs.len()
                )));
            }
            serde_json::json!({
                "inputs": inputs.iter().map(|p| serde_json::json!({"path": p})).collect::<Vec<_>>(),
                "output": output,
            })
        }
        "extract" | "remove-pages" | "reorder" => {
            let (pages, tail) = match (&cli.pages, positionals.len()) {
                (Some(_), 1) => (pages_for(None)?, None),
                (Some(_), 2) => (
                    // With --pages the second positional is the output,
                    // unless it looks like another pages spec — then the
                    // call is ambiguous.
                    pages_for(positionals.get(1))?,
                    positionals.get(1),
                ),
                (Some(_), _) => return Err(arity("<doc.pdf> [<out.pdf>] with --pages")),
                (None, 2..=3) => (pages_for(positionals.get(1))?, positionals.get(2)),
                (None, _) => return Err(arity("<doc.pdf> <pages> [<out.pdf>]")),
            };
            serde_json::json!({
                "input": {"path": positionals[0]},
                "pages": pages,
                "output": output_for(tail)?,
            })
        }
        "split" => {
            if positionals.len() < 3 {
                return Err(arity("<doc.pdf> and 2..20 parts as <part.pdf=pages>"));
            }
            let mut parts = Vec::new();
            for part in &positionals[1..] {
                let (path, pages) = part.split_once('=').ok_or(bad(format!(
                    "split part '{part}' must be written as <part.pdf=pages>"
                )))?;
                parts.push(serde_json::json!({
                    "pages": parse_page_list(pages)?,
                    "output": output_value(path, cli.replace),
                }));
            }
            serde_json::json!({"input": {"path": positionals[0]}, "parts": parts})
        }
        "compress" => {
            if !(1..=2).contains(&positionals.len()) {
                return Err(arity("<doc.pdf> [<out.pdf>]"));
            }
            let output = match (&cli.output, positionals.get(1)) {
                (Some(_), Some(_)) => {
                    return Err(bad(
                        "output given twice: -o/--output and a positional output are exclusive",
                    ));
                }
                (Some(output), None) => output_value(output, cli.replace),
                (None, Some(path)) => output_value(path, cli.replace),
                (None, None) => {
                    output_value(&derived_compress_output(&positionals[0]), cli.replace)
                }
            };
            let mut request =
                serde_json::json!({"input": {"path": positionals[0]}, "output": output});
            if let Some(mode) = &cli.mode {
                request["mode"] = mode.as_str().into();
            }
            if let Some(quality) = cli.jpeg_quality {
                request["jpegQuality"] = quality.into();
            }
            if let Some(level) = cli.level {
                request["compressionLevel"] = level.into();
            }
            if let Some(bytes) = cli.max_bytes {
                request["maxOutputBytes"] = bytes.into();
            }
            if let Some(miss) = &cli.on_miss {
                request["onTargetMiss"] = miss.as_str().into();
            }
            if cli.allow_larger {
                request["allowLargerOutput"] = true.into();
            }
            request
        }
        "extract-text" => {
            let pages = match (&cli.pages, positionals.len()) {
                (Some(_), 1) => pages_for(None)?,
                (None, 2) => pages_for(positionals.get(1))?,
                (Some(_), _) => return Err(arity("<doc.pdf> with --pages")),
                (None, _) => return Err(arity("<doc.pdf> <pages>")),
            };
            let mut request = serde_json::json!({
                "input": {"path": positionals[0]},
                "pages": pages,
            });
            if let Some(engine) = &cli.engine {
                request["engine"] = engine.as_str().into();
            }
            request
        }
        "encrypt" | "decrypt" => {
            if !(1..=2).contains(&positionals.len()) {
                return Err(arity("<doc.pdf> [<out.pdf>]"));
            }
            let output = match (&cli.output, positionals.get(1)) {
                (Some(_), Some(_)) => {
                    return Err(bad(
                        "output given twice: -o/--output and a positional output are exclusive",
                    ));
                }
                (Some(output), None) => output_value(output, cli.replace),
                (None, Some(path)) => output_value(path, cli.replace),
                (None, None) => output_value(
                    &derived_output(
                        &positionals[0],
                        if command == "encrypt" {
                            "encrypted"
                        } else {
                            "decrypted"
                        },
                    ),
                    cli.replace,
                ),
            };
            serde_json::json!({"input": {"path": positionals[0]}, "output": output})
        }
        "render-page" => {
            if !(2..=3).contains(&positionals.len()) {
                return Err(arity("<doc.pdf> <page> [<out.png>]"));
            }
            let mut request = serde_json::json!({
                "input": {"path": positionals[0]},
                "page": positionals[1].parse::<u64>().map_err(|_| {
                    bad(format!("page '{}' is not a positive integer", positionals[1]))
                })?,
                "output": output_for(positionals.get(2))?,
            });
            if let Some(dpi) = cli.dpi {
                request["dpi"] = dpi.into();
            }
            if let Some(engine) = &cli.engine {
                request["engine"] = engine.as_str().into();
            }
            request
        }
        _ => {
            return Err(bad(format!(
                "'{command}' takes no positional arguments — pipe a JSON request or use --request-json"
            )));
        }
    };
    if let Some(preserve) = preserve {
        request["preserve"] = preserve;
    }
    Ok(request)
}

fn parse_action<I: Iterator<Item = String>>(args: I) -> Result<Action> {
    let argv: Vec<String> = args.collect();
    let Some(command) = argv.first().cloned() else {
        return Ok(Action::Help(None));
    };
    match command.as_str() {
        "--help" | "-h" => {
            return if argv.len() > 1 {
                Err(bad("unexpected argument after --help"))
            } else {
                Ok(Action::Help(None))
            };
        }
        "--version" | "-V" | "version" => {
            return if argv.len() > 1 {
                Err(bad("version takes no arguments"))
            } else {
                Ok(Action::Version)
            };
        }
        _ => {}
    }
    // clap consumes argv left to right like the 0.3.2 parser: a `-h/--help`
    // reached as a flag raises help, while one after `--` or used as a flag
    // value (`-o -h`) stays a path, and earlier invalid arguments win.
    let parsed =
        match Cli::try_parse_from(std::iter::once("anpdf".to_string()).chain(argv.iter().cloned()))
        {
            Ok(parsed) => parsed,
            Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => {
                return Ok(Action::Help(Some(command)));
            }
            Err(e)
                if e.kind() == clap::error::ErrorKind::MissingRequiredArgument
                    && !NO_POSITIONALS.contains(&command.as_str()) =>
            {
                // Positionals are declared required so usage renders
                // <DOC.pdf>, but a shorthand operation without them is not
                // always an error: --request-json/--request-file/piped stdin
                // is the other 0.3.2 request source. Reconstruct that path
                // from argv; shaping flags without a document, short
                // positionals and bare terminal calls keep their
                // INVALID_REQUESTs.
                let mut opt = Options {
                    command: command.clone(),
                    root: PathBuf::from("."),
                    qpdf: PathBuf::new(),
                    qpdf_exact: false,
                    pdfium_lib: None,
                    request: None,
                    dry: false,
                    json: false,
                    port: 8787,
                    token_file: None,
                    output_only: None,
                    output_byte_ceiling: None,
                    output_operation: None,
                    schema: None,
                    password_file: None,
                    owner_password_file: None,
                };
                let mut positional_seen = false;
                let mut shaping: Vec<String> = Vec::new();
                let mut req_file: Option<String> = None;
                let mut qpdf_flag: Option<PathBuf> = None;
                let mut pdfium_flag: Option<PathBuf> = None;
                let mut it = argv[1..].iter().peekable();
                let mut terminated = false;
                while let Some(t) = it.next() {
                    if terminated {
                        positional_seen = true;
                        break;
                    }
                    if !t.starts_with('-') {
                        positional_seen = true;
                        break;
                    }
                    if t == "--" {
                        terminated = true;
                        continue;
                    }
                    let (name, inline) = match t.split_once('=') {
                        Some((n, v)) => (n.to_string(), Some(v.to_string())),
                        None => (t.clone(), None),
                    };
                    if !KNOWN_FLAGS.contains(&name.as_str()) {
                        // Attached short-flag value: '-oout.pdf' still names
                        // the shaping flag -o.
                        if name.starts_with("-o") && name.len() > 2 {
                            shaping.push("-o".to_string());
                        }
                        continue;
                    }
                    let value = inline.or_else(|| {
                        if VALUE_FLAGS.contains(&name.as_str()) {
                            it.next().cloned()
                        } else {
                            None
                        }
                    });
                    match name.as_str() {
                        "--request-json" => {
                            reject_inline_secrets(value.as_deref().unwrap_or_default())?;
                            opt.request = value;
                        }
                        "--request-file" => req_file = value,
                        "--root" => opt.root = PathBuf::from(value.unwrap_or_default()),
                        "--qpdf" => qpdf_flag = value.map(PathBuf::from),
                        "--pdfium-lib" => pdfium_flag = value.map(PathBuf::from),
                        "--password-file" => opt.password_file = value,
                        "--owner-password-file" => opt.owner_password_file = value,
                        "--dry-run" => opt.dry = true,
                        "--json" => opt.json = true,
                        "--qpdf-exact" => opt.qpdf_exact = true,
                        _ => shaping.push(name),
                    }
                }
                if opt.request.is_some() && req_file.is_some() {
                    return Err(bad("--request-json and --request-file are exclusive"));
                }
                if !shaping.is_empty() {
                    return Err(bad(format!(
                        "{} apply only to positional shorthand ({} <doc.pdf> ...)",
                        shaping.join(", "),
                        command
                    )));
                }
                if positional_seen {
                    let expected = match command.as_str() {
                        "extract" | "remove-pages" => "<doc.pdf> <pages> [<out.pdf>]",
                        "reorder" => "<doc.pdf> <ordered-pages> [<out.pdf>]",
                        "split" => "<doc.pdf> and 2..20 parts as <part.pdf=pages>",
                        "extract-text" => "<doc.pdf> <pages>",
                        "render-page" => "<doc.pdf> <page> [<out.png>]",
                        _ => "<doc.pdf>",
                    };
                    return Err(bad(format!(
                        "{command} expects {expected} — see 'anpdf help {command}'"
                    )));
                }
                if let Some(p) = req_file {
                    opt.request =
                        Some(read(std::fs::File::open(&p).map_err(|e| {
                            Error::io(&e, format!("--request-file '{p}'"))
                        })?)?);
                }
                opt.pdfium_lib = resolve_pdfium(
                    pdfium_flag,
                    std::env::var_os("ANPDF_PDFIUM_LIB"),
                    std::env::current_exe().ok(),
                )?;
                opt.qpdf = resolve_qpdf(
                    qpdf_flag,
                    std::env::var_os("ANPDF_QPDF"),
                    std::env::var_os("PATH"),
                )?;
                return if opt.request.is_none() && std::io::stdin().is_terminal() {
                    Err(bad(format!(
                        "{} needs a document or a JSON request — pipe stdin or use --request-json",
                        command
                    )))
                } else {
                    Ok(Action::Run(Box::new(opt)))
                };
            }
            Err(e) => return Err(clap_error(&command, e)),
        };
    let cmd = parsed.command;

    let mut o = Options {
        command,
        root: PathBuf::from("."),
        qpdf: PathBuf::new(),
        qpdf_exact: false,
        pdfium_lib: None,
        request: None,
        dry: false,
        json: false,
        port: 8787,
        token_file: None,
        output_only: None,
        output_byte_ceiling: None,
        output_operation: None,
        schema: None,
        password_file: None,
        owner_password_file: None,
    };
    let mut cli = CliArgs::default();
    let mut positionals: Vec<String> = Vec::new();
    let mut qpdf_flag = None;
    let mut request_file: Option<PathBuf> = None;
    let mut seen = HashSet::new();

    macro_rules! common {
        ($c:expr) => {{
            let c = $c;
            if let Some(v) = c.backends.root {
                o.root = v;
                seen.insert("--root");
            }
            if let Some(v) = c.backends.qpdf {
                qpdf_flag = Some(v);
                seen.insert("--qpdf");
            }
            if c.backends.qpdf_exact {
                o.qpdf_exact = true;
            }
            if let Some(v) = c.backends.pdfium_lib {
                o.pdfium_lib = Some(v);
                seen.insert("--pdfium-lib");
            }
            if let Some(v) = c.request_json {
                reject_inline_secrets(&v)?;
                o.request = Some(v);
                seen.insert("--request-json");
            }
            if let Some(v) = c.request_file {
                request_file = Some(v);
                seen.insert("--request-file");
            }
            if c.dry_run {
                o.dry = true;
            }
            if c.json {
                o.json = true;
            }
        }};
    }
    macro_rules! backends {
        ($c:expr) => {{
            let c = $c;
            if let Some(v) = c.root {
                o.root = v;
                seen.insert("--root");
            }
            if let Some(v) = c.qpdf {
                qpdf_flag = Some(v);
                seen.insert("--qpdf");
            }
            if c.qpdf_exact {
                o.qpdf_exact = true;
            }
            if let Some(v) = c.pdfium_lib {
                o.pdfium_lib = Some(v);
                seen.insert("--pdfium-lib");
            }
        }};
    }
    macro_rules! out {
        ($x:expr) => {{
            cli.output = $x.output;
        }};
    }
    macro_rules! replace {
        ($x:expr) => {{
            cli.replace = $x.replace;
        }};
    }
    macro_rules! preserve {
        ($x:expr) => {{
            cli.preserve = $x.preserve.map(|s| {
                s.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });
        }};
    }
    macro_rules! pages {
        ($x:expr) => {{
            cli.pages = $x.pages;
        }};
    }
    macro_rules! engine {
        ($x:expr) => {{
            cli.engine = $x.engine;
        }};
    }
    fn parse_u64(value: Option<String>, detail: &str) -> Result<Option<u64>> {
        value
            .map(|s| s.parse().map_err(|_| bad(detail.to_string())))
            .transpose()
    }

    match cmd {
        Cmd::Help(HelpArgs { topic }) => return Ok(Action::Help(topic)),
        Cmd::Version => return Ok(Action::Version),
        Cmd::Completions(CompletionsArgs { shell }) => {
            return Ok(Action::Completions(Shell::parse(&shell)?));
        }
        Cmd::Schema(SchemaArgs { operation_id }) => {
            o.schema = Some(operation_id);
        }
        Cmd::Serve(ServeArgs {
            backends: b,
            port,
            token_file,
        }) => {
            backends!(b);
            o.port = port
                .map(|s| {
                    s.parse()
                        .map_err(|_| bad("--port expects a port number 0..65535".to_string()))
                })
                .transpose()?
                .unwrap_or(8787);
            o.token_file = token_file;
        }
        Cmd::Mcp(a) => {
            backends!(a.backends);
            o.output_only = a.output_only;
            o.output_byte_ceiling = a.output_byte_ceiling;
            o.output_operation = a.output_operation;
        }
        Cmd::Operations(OperationsArgs { backends: b }) => backends!(b),
        Cmd::Inspect(a) | Cmd::Validate(a) => {
            positionals = a.doc.into_iter().collect();
            cli.deep_check = a.deep_check.deep_check;
            if cli.deep_check {
                seen.insert("--deep-check");
            }
            common!(a.common);
        }
        Cmd::Merge(a) => {
            positionals = a.inputs;
            out!(a.out);
            replace!(a.replace);
            preserve!(a.preserve);
            common!(a.common);
        }
        Cmd::Extract(a) => {
            positionals = [a.doc, a.pages_pos, a.out_pos]
                .into_iter()
                .flatten()
                .collect();
            out!(a.out);
            replace!(a.replace);
            preserve!(a.preserve);
            pages!(a.pages);
            common!(a.common);
        }
        Cmd::RemovePages(a) => {
            positionals = [a.doc, a.pages_pos, a.out_pos]
                .into_iter()
                .flatten()
                .collect();
            out!(a.out);
            replace!(a.replace);
            preserve!(a.preserve);
            pages!(a.pages);
            common!(a.common);
        }
        Cmd::Reorder(a) => {
            positionals = [a.doc, a.pages_pos, a.out_pos]
                .into_iter()
                .flatten()
                .collect();
            out!(a.out);
            replace!(a.replace);
            preserve!(a.preserve);
            pages!(a.pages);
            common!(a.common);
        }
        Cmd::Split(a) => {
            positionals = a.doc.into_iter().chain(a.parts).collect();
            cli.replace = a.replace.replace;
            preserve!(a.preserve);
            common!(a.common);
        }
        Cmd::ExtractText(a) => {
            positionals = [a.doc, a.pages_pos].into_iter().flatten().collect();
            engine!(a.engine);
            pages!(a.pages);
            common!(a.common);
        }
        Cmd::RenderPage(a) => {
            positionals = [a.doc, a.page_pos, a.out_pos]
                .into_iter()
                .flatten()
                .collect();
            out!(a.out);
            engine!(a.engine);
            replace!(a.replace);
            cli.dpi = parse_u64(a.dpi.dpi, "--dpi expects 36..300")?;
            common!(a.common);
        }
        Cmd::Encrypt(a) => {
            positionals = [a.doc, a.out_pos].into_iter().flatten().collect();
            out!(a.out);
            replace!(a.replace);
            preserve!(a.preserve);
            o.password_file = a.user.password_file;
            o.owner_password_file = a.owner.owner_password_file;
            common!(a.common);
        }
        Cmd::Decrypt(a) => {
            positionals = [a.doc, a.out_pos].into_iter().flatten().collect();
            out!(a.out);
            replace!(a.replace);
            preserve!(a.preserve);
            o.password_file = a.user.password_file;
            common!(a.common);
        }
        Cmd::Compress(a) => {
            positionals = [a.doc, a.out_pos].into_iter().flatten().collect();
            out!(a.flags.out);
            cli.replace = a.flags.replace.replace;
            cli.mode = a.flags.mode;
            cli.jpeg_quality = parse_u64(
                a.flags.jpeg_quality,
                "--jpeg-quality expects an integer 0-100",
            )?;
            cli.level = a.flags.level.map(|s| s.parse::<u64>().unwrap());
            cli.max_bytes = a.flags.max_bytes.map(|s| parse_size(&s)).transpose()?;
            cli.on_miss = a.flags.on_target_miss;
            cli.allow_larger = a.flags.allow_larger;
            common!(a.common);
        }
    }

    if seen.contains("--request-json") && seen.contains("--request-file") {
        return Err(bad("--request-json and --request-file are exclusive"));
    }
    if let Some(p) = request_file {
        o.request = Some(read(std::fs::File::open(&p).map_err(|e| {
            Error::io(&e, format!("--request-file '{}'", p.display()))
        })?)?)
    }
    // Positional shorthand and request flags are exclusive request sources;
    // shaping flags only combine with shorthand, and a bare operation never
    // blocks on an interactive stdin — it reports INVALID_REQUEST instead.
    if !positionals.is_empty() && o.request.is_some() {
        return Err(bad(
            "positional shorthand cannot be combined with --request-json/--request-file",
        ));
    }
    if cli.shapes_request() && (positionals.is_empty() || o.request.is_some()) {
        let flags = seen
            .iter()
            .filter(|f| {
                ![
                    "--root",
                    "--qpdf",
                    "--qpdf-exact",
                    "--pdfium-lib",
                    "--request-json",
                    "--request-file",
                    "--dry-run",
                    "--json",
                ]
                .iter()
                .any(|b| *b == **f)
            })
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        return Err(bad(format!(
            "{flags} apply only to positional shorthand ({} <doc.pdf> ...)",
            o.command
        )));
    }
    if !positionals.is_empty() {
        normalize_shorthand(
            &o.command,
            &mut positionals,
            &mut cli,
            &mut o.root,
            seen.contains("--root"),
        )?;
        o.request = Some(request_from_positionals(&o.command, &positionals, &cli)?.to_string());
    }
    if o.command == "serve" && o.token_file.is_none() {
        return Err(bad(
            "serve requires --token-file <file> — see 'anpdf help serve'",
        ));
    }
    if o.request.is_none()
        && matches!(
            o.command.as_str(),
            "inspect"
                | "validate"
                | "merge"
                | "extract"
                | "split"
                | "remove-pages"
                | "reorder"
                | "compress"
                | "extract-text"
                | "render-page"
                | "encrypt"
                | "decrypt"
        )
        && std::io::stdin().is_terminal()
    {
        return Err(bad(format!(
            "{} needs a document or a JSON request — pipe stdin or use --request-json",
            o.command
        )));
    }
    o.pdfium_lib = resolve_pdfium(
        o.pdfium_lib,
        std::env::var_os("ANPDF_PDFIUM_LIB"),
        std::env::current_exe().ok(),
    )?;
    if !matches!(o.command.as_str(), "operations" | "schema") {
        o.qpdf = resolve_qpdf(
            qpdf_flag,
            std::env::var_os("ANPDF_QPDF"),
            std::env::var_os("PATH"),
        )?;
    }
    Ok(Action::Run(Box::new(o)))
}

/// `--request-json` puts its payload on argv: the OS and shell record it
/// when the process launches, so anPDF refuses literal secrets there —
/// without echoing them — while being honest that refusal cannot undo
/// the exposure. Secrets arrive through the hidden prompt, a private
/// --password-file/--owner-password-file, --request-file or piped stdin.
fn reject_inline_secrets(raw: &str) -> Result<()> {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return Ok(());
    };
    if v.get("password").is_some() || v.get("ownerPassword").is_some() {
        return Err(bad(
            "--request-json cannot carry \"password\"/\"ownerPassword\" — argv was already visible when the process launched; use a hidden prompt, --password-file/--owner-password-file, --request-file or piped stdin",
        ));
    }
    Ok(())
}

/// First line of a private (chmod 600) secret file; bounded read, trailing
/// newline stripped. A lone newline supplies an intentionally empty
/// password; a zero-byte file supplies nothing and is refused.
fn read_secret_file(flag: &str, path: &str) -> Result<String> {
    let file = std::fs::File::open(path).map_err(|e| Error::io(&e, format!("{flag} '{path}'")))?;
    let meta = file.metadata().map_err(|e| Error::io(&e, path))?;
    if meta.mode() & 0o077 != 0 {
        return Err(bad(format!(
            "{flag} '{path}' is readable beyond its owner — chmod 600 it first"
        )));
    }
    let mut bytes = Vec::new();
    file.take(1025).read_to_end(&mut bytes)?;
    if bytes.len() > 1024 {
        return Err(bad(format!("{flag} '{path}' exceeds 1 KiB")));
    }
    if bytes.is_empty() {
        return Err(bad(format!("{flag} '{path}' is empty")));
    }
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.lines().next().unwrap_or_default().to_string())
}

/// Hidden prompt on /dev/tty: termios echo off for the entry, restored
/// before returning. Bounded at 1 KiB.
#[cfg(unix)]
fn prompt_secret(prompt: &str) -> Result<String> {
    use std::os::unix::io::AsRawFd;
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| {
            bad(format!(
                "{prompt} — no terminal for a hidden prompt; use --password-file, --request-file or piped stdin"
            ))
        })?;
    let fd = tty.as_raw_fd();
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    let masked = unsafe { libc::tcgetattr(fd, &mut saved) } == 0;
    if masked {
        let mut silent = saved;
        silent.c_lflag &= !libc::ECHO;
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &silent) };
    }
    let read_result = (|| -> Result<String> {
        tty.write_all(prompt.as_bytes())?;
        tty.flush()?;
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        let mut terminated = false;
        loop {
            let n = tty.read(&mut byte)?;
            if n == 0 {
                break;
            }
            if byte[0] == b'\n' {
                terminated = true;
                break;
            }
            line.push(byte[0]);
            if line.len() > 1024 {
                return Err(bad("password entry too long"));
            }
        }
        // A bare Enter is an intentionally empty password (an owner-only
        // file); EOF without a newline is a cancelled entry, which is
        // different.
        if !terminated {
            return Err(bad("password entry cancelled"));
        }
        Ok(String::from_utf8_lossy(&line)
            .trim_end_matches('\r')
            .to_string())
    })();
    if masked {
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &saved) };
        let _ = tty.write_all(b"\n");
    }
    read_result
}

#[cfg(not(unix))]
fn prompt_secret(prompt: &str) -> Result<String> {
    Err(bad(format!(
        "{prompt} — hidden prompts need a Unix terminal; use --password-file, --request-file or piped stdin"
    )))
}

/// Fill `password` (required) and `ownerPassword` (encrypt only) on
/// encrypt/decrypt requests: a request field, else a private
/// --password-file/--owner-password-file, else a hidden /dev/tty prompt.
/// A blank owner prompt omits the field — the dispatcher generates a
/// random owner, so no blank-password backdoor exists.
fn fill_protect_secrets(
    command: &str,
    dry: bool,
    password_file: Option<&str>,
    owner_password_file: Option<&str>,
    request: &mut Value,
) -> Result<()> {
    if !matches!(command, "encrypt" | "decrypt") {
        if password_file.is_some() || owner_password_file.is_some() {
            return Err(bad(
                "--password-file/--owner-password-file apply to encrypt and decrypt",
            ));
        }
        return Ok(());
    }
    // A request body that is not a JSON object fails schema validation in
    // the dispatcher; indexing a non-object Value would panic instead.
    if !request.is_object() {
        return Ok(());
    }
    let encrypting = command == "encrypt";
    let mut prompted = false;
    if request.get("password").is_none() {
        if dry {
            // Dry runs validate but never reach the backend — a placeholder
            // satisfies the required field without inventing a real secret.
            request["password"] = "x".into();
        } else {
            request["password"] = match password_file {
                Some(f) => read_secret_file("--password-file", f)?,
                None => {
                    prompted = true;
                    prompt_secret("Password: ")?
                }
            }
            .into();
        }
    } else if password_file.is_some() {
        return Err(bad(
            "password given twice: request field and --password-file are exclusive",
        ));
    }
    if !encrypting {
        if owner_password_file.is_some() {
            return Err(bad("--owner-password-file applies to encrypt only"));
        }
        return Ok(());
    }
    if request.get("ownerPassword").is_some() {
        if owner_password_file.is_some() {
            return Err(bad(
                "ownerPassword given twice: request field and --owner-password-file are exclusive",
            ));
        }
        return Ok(());
    }
    if dry {
        request["ownerPassword"] = "x".into();
        return Ok(());
    }
    // Scripted channels (request JSON, --password-file, piped stdin) omit the
    // owner deliberately — it is generated. Only an interactive session is
    // offered the prompt for it.
    let owner = match owner_password_file {
        Some(f) => Some(read_secret_file("--owner-password-file", f)?),
        None if prompted => match prompt_secret("Owner password (blank to generate): ")? {
            s if s.is_empty() => None,
            s => Some(s),
        },
        None => None,
    };
    if let Some(owner) = owner {
        request["ownerPassword"] = owner.into();
    }
    Ok(())
}

fn read(r: impl Read) -> Result<String> {
    let mut b = Vec::new();
    r.take(1024 * 1024 + 1).read_to_end(&mut b)?;
    if b.len() > 1024 * 1024 {
        return Err(Error(
            "LIMIT_EXCEEDED",
            Some("request input exceeds 1 MiB".into()),
        ));
    }
    String::from_utf8(b).map_err(|_| bad("request input is not valid UTF-8"))
}

fn id(command: &str) -> &'static str {
    match command {
        "inspect" | "validate" => "inspectPdf",
        "merge" => "mergePdf",
        "extract" => "extractPdf",
        "split" => "splitPdf",
        "remove-pages" => "removePages",
        "reorder" => "reorderPages",
        "compress" => "compressPdf",
        "extract-text" => "extractText",
        "render-page" => "renderPage",
        "encrypt" => "encryptPdf",
        "decrypt" => "decryptPdf",
        _ => "unknown",
    }
}

fn configured_registry(
    executable: PathBuf,
    exact: bool,
    library: Option<PathBuf>,
) -> Result<BackendRegistry> {
    let registry = BackendRegistry::with_qpdf_exact(executable, exact);
    #[cfg(feature = "pdfium")]
    let registry = {
        let mut registry = registry;
        if let Some(library) = library {
            let worker_executable =
                std::env::current_exe().map_err(|_| Error("BACKEND_UNAVAILABLE", None))?;
            let provider = Arc::new(anpdf::pdfium::PdfiumProvider {
                library,
                worker_executable,
            });
            registry
                .register_text("pdfium", provider.clone())
                .expect("unique engine");
            registry
                .register_renderer("pdfium", provider.clone())
                .expect("unique engine");
            // The same isolated worker rasterizes traced pages for
            // size-targeted compression when image resampling misses.
            registry.register_rasterizer(provider);
        }
        registry
    };
    #[cfg(not(feature = "pdfium"))]
    let _ = library;
    #[cfg(feature = "pdfoxide")]
    let registry = {
        let mut registry = registry;
        let executable = std::env::current_exe().map_err(|_| Error("BACKEND_UNAVAILABLE", None))?;
        anpdf::pdfoxide::register(&mut registry, executable)?;
        registry
    };
    let mut registry = registry;
    registry.register_resampler(Arc::new(anpdf::resample::ResampleProvider {
        worker_executable: std::env::current_exe()
            .map_err(|_| Error("BACKEND_UNAVAILABLE", None))?,
    }));
    Ok(registry)
}

fn main() {
    #[cfg(feature = "pdfium")]
    if std::env::args_os().nth(1).as_deref() == Some("pdfium-worker".as_ref()) {
        std::process::exit(anpdf::pdfium::worker());
    }
    #[cfg(feature = "pdfoxide")]
    if std::env::args_os().nth(1).as_deref() == Some(anpdf::pdfoxide::WORKER_ARG.as_ref()) {
        std::process::exit(anpdf::pdfoxide::worker(std::env::args_os().skip(2)));
    }
    if std::env::args_os().nth(1).as_deref() == Some(anpdf::resample::WORKER_ARG.as_ref()) {
        std::process::exit(anpdf::resample::worker(std::env::args_os().skip(2)));
    }
    async_main();
}

/// Compact human byte size: `616 B`, `12.0 KiB`, `1.5 MiB`.
fn fmt_bytes(b: u64) -> String {
    if b >= 1024 * 1024 {
        format!("{:.1} MiB", b as f64 / 1_048_576.0)
    } else if b >= 1024 {
        format!("{:.1} KiB", b as f64 / 1024.0)
    } else {
        format!("{b} B")
    }
}

fn str_field<'v>(v: &'v Value, k: &str) -> &'v str {
    v.get(k).and_then(Value::as_str).unwrap_or("?")
}

fn u64_field(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

/// Filenames inherited from user input can carry terminal control
/// characters; print them escaped rather than raw.
fn tty_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

/// Result paths are workspace-relative; on a terminal print them
/// anchored to the effective workspace root when it differs from the
/// invoking directory, so the printed name is where the file is.
fn tty_path<'a>(root: Option<&'a Path>, rel: &'a str) -> String {
    let canon_root = root.and_then(|r| r.canonicalize().ok());
    let canon_cwd = std::env::current_dir()
        .ok()
        .and_then(|c| c.canonicalize().ok());
    match canon_root {
        Some(r) if Some(&r) != canon_cwd.as_ref() => tty_escape(&r.join(rel).to_string_lossy()),
        _ => tty_escape(rel),
    }
}

/// Short plain-language rendering of a result envelope for terminals.
/// Piped stdout keeps the compact envelope; `--json` prints it on a
/// terminal too. Returns None for non-result payloads (operations,
/// schema), which stay as JSON.
fn human_result(v: &Value, root: Option<&Path>) -> Option<String> {
    if v.get("dryRun").and_then(Value::as_bool).unwrap_or(false) {
        return Some("request and destination policy valid (dry run — nothing written)".into());
    }
    let result = v.get("result")?;
    let op = v.get("operationId")?.as_str()?;
    let mut lines: Vec<String> = Vec::new();
    match op {
        "inspectPdf" => {
            let mut flags = vec![
                format!("{} pages", u64_field(result, "pageCount")),
                fmt_bytes(u64_field(result, "bytes")),
                format!("PDF {}", str_field(result, "pdfVersion")),
            ];
            if result
                .get("encrypted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                flags.push("encrypted".into());
            }
            // forms/signatures/actions are "present"|"absent"|"unknown"
            // strings, not booleans.
            for k in ["forms", "signatures", "actions"] {
                match str_field(result, k) {
                    "present" => flags.push(k.into()),
                    "unknown" => flags.push(format!("{k}?")),
                    _ => {}
                }
            }
            if result
                .get("hasMetadata")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                flags.push("metadata".into());
            }
            if result
                .get("hasOutline")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                flags.push("outline".into());
            }
            if result
                .get("hasAttachments")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                flags.push("attachments".into());
            }
            flags.push(format!(
                "structure: {}",
                str_field(result, "structuralStatus")
            ));
            lines.push(flags.join(" · "));
        }
        "mergePdf" | "extractPdf" | "removePages" | "reorderPages" | "encryptPdf"
        | "decryptPdf" => {
            lines.push(format!(
                "wrote '{}' — {} pages, {}",
                tty_path(root, str_field(result, "path")),
                u64_field(result, "pageCount"),
                fmt_bytes(u64_field(result, "bytes"))
            ));
        }
        "splitPdf" => {
            let outputs = result
                .get("outputs")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            lines.push(format!("wrote {} outputs", outputs.len()));
            for o in &outputs {
                lines.push(format!(
                    "  {} — {} pages, {}",
                    tty_path(root, str_field(o, "path")),
                    u64_field(o, "pageCount"),
                    fmt_bytes(u64_field(o, "bytes"))
                ));
            }
        }
        "compressPdf" => {
            let mut line = format!(
                "wrote '{}' — {} → {} ({})",
                tty_path(root, str_field(result, "path")),
                fmt_bytes(u64_field(result, "inputBytes")),
                fmt_bytes(u64_field(result, "bytes")),
                str_field(result, "mode")
            );
            let saved = u64_field(result, "savedBytes");
            if str_field(result, "outputSource") == "original" {
                line.push_str(" — no reduction; original bytes kept");
            } else if saved > 0 {
                let pct = (1.0
                    - result
                        .get("compressionRatio")
                        .and_then(Value::as_f64)
                        .unwrap_or(1.0))
                    * 100.0;
                line.push_str(&format!(
                    " — saved {} ({pct:.0}% smaller)",
                    fmt_bytes(saved)
                ));
            } else {
                line.push_str(" — no size reduction");
            }
            if let (Some(t), Some(met)) = (
                result.get("targetBytes").and_then(Value::as_u64),
                result.get("targetMet").and_then(Value::as_bool),
            ) {
                line.push_str(&format!(
                    "; target {} {}",
                    fmt_bytes(t),
                    if met { "met" } else { "missed" }
                ));
            }
            lines.push(line);
        }
        "extractText" => {
            let pages = result
                .get("pages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            lines.push(format!(
                "{} page{} of text via {}",
                pages.len(),
                if pages.len() == 1 { "" } else { "s" },
                str_field(result, "engine")
            ));
            for p in &pages {
                lines.push(format!("── page {} ──", u64_field(p, "page")));
                lines.push(str_field(p, "text").to_string());
            }
        }
        "renderPage" => {
            lines.push(format!(
                "wrote '{}' — page {}, {}×{} @{} dpi via {}, {}",
                tty_path(root, str_field(result, "path")),
                u64_field(result, "page"),
                u64_field(result, "width"),
                u64_field(result, "height"),
                u64_field(result, "dpi"),
                str_field(result, "engine"),
                fmt_bytes(u64_field(result, "bytes"))
            ));
        }
        _ => return None,
    }
    for w in v
        .get("warnings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        lines.push(format!(
            "warning {}: {}",
            str_field(&w, "code"),
            str_field(&w, "message")
        ));
    }
    Some(lines.join("\n"))
}

/// Report an error and exit. Piped stderr keeps the canonical JSON
/// envelope; on a terminal a person gets a readable `anpdf <cmd>: <reason>`
/// line, plus usage when the failure is arg-shape misuse.
fn report_error(e: &Error, command: &str, op: &str) -> ! {
    if std::io::stderr().is_terminal() {
        let topic = KNOWN_COMMANDS.contains(&command).then_some(command);
        // On a terminal lead with the specific reason — the generic
        // base sentence only shows when no detail exists. Filenames in
        // details can carry terminal control characters; escape them.
        let tty_message: String = e
            .detail()
            .unwrap_or_else(|| e.base())
            .chars()
            .flat_map(|c| {
                if c.is_control() {
                    c.escape_default().collect::<Vec<_>>()
                } else {
                    vec![c]
                }
            })
            .collect();
        eprintln!(
            "anpdf{}: {}",
            topic.map(|t| format!(" {t}")).unwrap_or_default(),
            tty_message
        );
        if matches!(e.0, "INVALID_REQUEST" | "UNSUPPORTED_OPERATION") {
            eprint!("\n{}", usage(topic));
        }
    } else {
        eprintln!("{}", e.envelope(op));
    }
    std::process::exit(e.exit());
}

#[tokio::main]
async fn async_main() {
    let argv: Vec<String> = match std::env::args_os()
        .skip(1)
        .map(|a| {
            a.into_string()
                .map_err(|_| bad("arguments must be valid UTF-8"))
        })
        .collect::<Result<Vec<_>>>()
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{}", e.envelope("unknown"));
            std::process::exit(e.exit());
        }
    };
    let command = argv.first().cloned().unwrap_or_default();
    let op = id(&command);
    let action = match parse_action(argv.into_iter()) {
        Ok(action) => action,
        Err(e) => report_error(&e, &command, op),
    };
    let want_json = matches!(&action, Action::Run(o) if o.json);
    let tty_root = match &action {
        Action::Run(o) => Some(o.root.clone()),
        _ => None,
    };
    let result = async {
        match action {
            Action::Help(topic) => {
                print!("{}", usage(topic.as_deref()));
                Ok(None)
            }
            Action::Version => {
                println!("anpdf {}", env!("CARGO_PKG_VERSION"));
                Ok(None)
            }
            Action::Completions(shell) => {
                let mut out = std::io::stdout().lock();
                clap_complete::generate(shell.clap(), &mut Cli::command(), "anpdf", &mut out);
                Ok(None)
            }
            Action::Run(o) => {
                let mut o = *o;
                if o.command == "operations" {
                    return Ok(Some(contract::discovery_with(&configured_registry(
                        o.qpdf,
                        o.qpdf_exact,
                        o.pdfium_lib,
                    )?)));
                }
                if o.command == "schema" {
                    return contract::operation(o.schema.as_deref().unwrap())
                        .map(|v| {
                            Some(serde_json::json!({
                                "operation": v,
                                "inputSchema": contract::input(o.schema.as_deref().unwrap()),
                                "outputSchema": contract::resolve(&v["responses"]["200"]["content"]["application/json"]["schema"]),
                                "errorSchema": contract::resolve(&contract::document()["components"]["schemas"]["Error"]),
                            }))
                        })
                        .ok_or(Error(
                            "UNSUPPORTED_OPERATION",
                            Some(format!(
                                "no such operationId '{}' — see 'anpdf operations'",
                                o.schema.as_deref().unwrap()
                            )),
                        ));
                }
                // configured_registry takes ownership of the backend paths;
                // lift the secret-channel flags first for the fill below.
                let password_file = o.password_file.take();
                let owner_password_file = o.owner_password_file.take();
                let d = Dispatcher::with_registry(
                    &o.root,
                    configured_registry(o.qpdf, o.qpdf_exact, o.pdfium_lib)?,
                )?;
                let d = Arc::new(match o.output_only {
                    Some(path) => d.restrict_output(path, o.output_byte_ceiling, o.output_operation)?,
                    None => d,
                });
                match o.command.as_str() {
                    "serve" => {
                        let file = o.token_file.ok_or(Error("INVALID_REQUEST", None))?;
                        let token = read(std::fs::File::open(file)?)?.trim().to_string();
                        anpdf::http::serve(d, o.port, token).await?;
                        Ok(None)
                    }
                    "mcp" => {
                        let worker_dispatcher = d.clone();
                        let mut worker = tokio::task::spawn_blocking(move || {
                            anpdf::mcp::serve(worker_dispatcher)
                        });
                        tokio::select! {
                            r = &mut worker => { r.map_err(|_| Error("INTERNAL", None))??; Ok(None) },
                            _ = anpdf::http::shutdown_signal() => {
                                d.cancel_all();
                                while d.active_count() > 0 {
                                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                                }
                                std::process::exit(130)
                            }
                        }
                    }
                    _ => {
                        let mut request: Value = serde_json::from_str(&match o.request {
                            Some(s) => s,
                            None => read(std::io::stdin())?,
                        })
                        .map_err(|e| bad(format!("request is not valid JSON: {e}")))?;
                        fill_protect_secrets(
                            &o.command,
                            o.dry,
                            password_file.as_deref(),
                            owner_password_file.as_deref(),
                            &mut request,
                        )?;
                        let mut control = Control::default();
                        // Progress goes on stderr only when every stream is a
                        // terminal and --json is off — pipes stay byte-exact.
                        // The ticker waits 300ms so instant failures never
                        // flash a spinner; a dropped ticker clears its line.
                        let bar = if std::io::stderr().is_terminal()
                            && std::io::stdout().is_terminal()
                            && std::io::stdin().is_terminal()
                            && !o.json
                        {
                            let message = Arc::new(std::sync::Mutex::new(String::new()));
                            let stopped = Arc::new(AtomicBool::new(false));
                            let ticker = {
                                let message = message.clone();
                                let stopped = stopped.clone();
                                std::thread::spawn(move || {
                                    use std::io::Write as _;
                                    const FRAMES: &[&str] =
                                        &["⠁", "⠉", "⠙", "⠚", "⠒", "⠂"];
                                    let mut frame = 0usize;
                                    std::thread::sleep(Duration::from_millis(300));
                                    while !stopped.load(Ordering::Relaxed) {
                                        let text = message
                                            .lock()
                                            .map(|m| m.clone())
                                            .unwrap_or_default();
                                        let mut err = std::io::stderr();
                                        let _ = write!(
                                            err,
                                            "\r\x1b[2K{} {}",
                                            FRAMES[frame % FRAMES.len()],
                                            text
                                        );
                                        let _ = err.flush();
                                        frame += 1;
                                        std::thread::sleep(Duration::from_millis(80));
                                    }
                                    if frame > 0 {
                                        let mut err = std::io::stderr();
                                        let _ = write!(err, "\r\x1b[2K");
                                        let _ = err.flush();
                                    }
                                })
                            };
                            let sink = message.clone();
                            control.progress = Some(Arc::new(move |p: &Progress| {
                                let msg = match p.step {
                                    Some((done, total)) => {
                                        format!("{} — {} ({}/{})", p.phase, p.detail, done, total)
                                    }
                                    None => format!("{} — {}", p.phase, p.detail),
                                };
                                if let Ok(mut m) = sink.lock() {
                                    *m = msg;
                                }
                            }));
                            Some((ticker, stopped))
                        } else {
                            None
                        };
                        let cancel = control.clone();
                        let mut task = tokio::task::spawn_blocking(move || {
                            d.dispatch(op, &request, o.dry, &control)
                        });
                        let selected = tokio::select! {
                            result = &mut task => result.map_err(|_| Error("INTERNAL", None))?.map(Some),
                            _ = anpdf::http::shutdown_signal() => {
                                cancel.cancelled.store(true, Ordering::Relaxed);
                                let _ = task.await;
                                eprintln!("{}", Error("TIMEOUT", None).envelope(op));
                                std::process::exit(130)
                            }
                        };
                        if let Some((ticker, stopped)) = bar {
                            stopped.store(true, Ordering::Relaxed);
                            let _ = ticker.join();
                        }
                        selected
                    }
                }
            }
        }
    }
    .await;
    match result {
        Ok(Some(v)) => {
            // Piped stdout keeps the compact envelope bytes; on a
            // terminal a person gets a short plain summary, or the
            // pretty envelope with --json.
            if std::io::stdout().is_terminal() {
                let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string());
                if want_json {
                    println!("{pretty}");
                } else {
                    println!(
                        "{}",
                        human_result(&v, tty_root.as_deref()).unwrap_or(pretty)
                    );
                }
            } else {
                println!("{v}");
            }
        }
        Ok(None) => {}
        Err(e) => report_error(&e, &command, op),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Action, COMMAND_GROUPS, Cli, Shell, derived_compress_output, parse_action, parse_page_list,
        request_from_positionals, resolve_pdfium, resolve_qpdf, usage,
    };
    use clap::CommandFactory;
    use serde_json::Value;
    use std::{ffi::OsString, fs, os::unix::fs::PermissionsExt, path::PathBuf};

    fn run_request(args: &[&str]) -> Value {
        match parse_action(args.iter().copied().map(String::from)) {
            Ok(Action::Run(o)) => serde_json::from_str(&o.request.unwrap()).unwrap(),
            other => panic!("expected Run for {args:?}, got {other:?}"),
        }
    }

    fn parse_err(args: &[&str]) -> &'static str {
        match parse_action(args.iter().copied().map(String::from)) {
            Err(super::Error(code, _)) => code,
            other => panic!("expected error for {args:?}, got {other:?}"),
        }
    }

    #[test]
    fn bare_invocation_shows_help() {
        assert!(matches!(
            parse_action(std::iter::empty()),
            Ok(Action::Help(None))
        ));
        assert!(matches!(
            parse_action(["--help"].iter().copied().map(String::from)),
            Ok(Action::Help(None))
        ));
        assert!(matches!(
            parse_action(["inspect", "--help"].iter().copied().map(String::from)),
            Ok(Action::Help(Some(topic))) if topic == "inspect"
        ));
        // help also works alongside positional shorthand
        assert!(matches!(
            parse_action(["compress", "doc.pdf", "-h"].iter().copied().map(String::from)),
            Ok(Action::Help(Some(topic))) if topic == "compress"
        ));
        assert_eq!(parse_err(&["bogus", "--help"]), "UNSUPPORTED_OPERATION");
        // A bare operation with piped stdin still reads a JSON request from it.
        assert!(matches!(
            parse_action(["inspect"].iter().copied().map(String::from)),
            Ok(Action::Run(o)) if o.request.is_none()
        ));
    }

    #[test]
    fn overview_groups_flat_commands_by_task() {
        let text = usage(None);
        assert!(!text.contains("Commands:\n"));
        let inventory = text.split("\nOptions:").next().unwrap();
        let mut previous = 0;
        for (heading, names) in COMMAND_GROUPS {
            let heading_at = inventory
                .find(&format!("{heading}:"))
                .unwrap_or_else(|| panic!("missing help group {heading}"));
            assert!(heading_at >= previous);
            previous = heading_at;
            let mut previous_name = heading_at;
            for name in *names {
                let row = format!("  {name:<14}");
                assert_eq!(
                    inventory.matches(&row).count(),
                    1,
                    "missing or duplicate command row {name}"
                );
                let at = inventory.find(&row).unwrap();
                assert!(at > previous_name, "{name} is outside or before {heading}");
                previous_name = at;
            }
        }
        for subcommand in Cli::command().get_subcommands() {
            let row = format!("  {:<14}", subcommand.get_name());
            assert_eq!(
                inventory.matches(&row).count(),
                1,
                "ungrouped or duplicate command {}",
                subcommand.get_name()
            );
        }
        assert!(text.contains("anpdf reorder <doc.pdf> <ordered-pages> -o <ordered.pdf>"));
        assert!(!text.contains("anpdf pages reorder"));
        let output = text.find("OUTPUT AND PAGE OPTIONS:").unwrap();
        let safety = text.find("SAFETY OPTIONS:").unwrap();
        let credential = text.find("CREDENTIAL OPTIONS:").unwrap();
        let common = text.find("COMMON OPTIONS:").unwrap();
        let server = text.find("SERVER OPTIONS:").unwrap();
        assert!(output < safety && safety < credential && credential < common && common < server);

        let compress = usage(Some("compress"));
        assert!(compress.contains("--mode lossless --max-bytes 3MB --on-target-miss error"));
        let modes = compress.find("Modes:").unwrap();
        let output = compress.find("OUTPUT OPTIONS:").unwrap();
        let operation = compress.find("OPERATION OPTIONS:").unwrap();
        let safety = compress.find("SAFETY OPTIONS:").unwrap();
        let common = compress.find("COMMON OPTIONS:").unwrap();
        assert!(modes < output && output < operation && operation < safety && safety < common);
        let operation_section = &compress[operation..safety];
        assert!(operation_section.contains("--mode <mode>"));
        assert!(operation_section.contains("--max-bytes <N>"));
        let safety_section = &compress[safety..common];
        assert!(safety_section.contains("--on-target-miss <behavior>"));
        assert!(safety_section.contains("--allow-larger"));
        assert!(safety_section.contains("--replace"));
        assert!(safety_section.contains("--dry-run"));

        let reorder = usage(Some("reorder"));
        let output = reorder.find("OUTPUT OPTIONS:").unwrap();
        let operation = reorder.find("OPERATION OPTIONS:").unwrap();
        let safety = reorder.find("SAFETY OPTIONS:").unwrap();
        let common = reorder.find("COMMON OPTIONS:").unwrap();
        assert!(output < operation && operation < safety && safety < common);
        assert!(reorder[operation..safety].contains("--pages <spec>"));
        assert!(reorder[safety..common].contains("--preserve <list>"));

        let encrypt = usage(Some("encrypt"));
        let credential = encrypt.find("CREDENTIAL OPTIONS:").unwrap();
        let safety = encrypt.find("SAFETY OPTIONS:").unwrap();
        let common = encrypt.find("COMMON OPTIONS:").unwrap();
        assert!(credential < safety && safety < common);
        assert!(encrypt[credential..safety].contains("--password-file <file>"));

        let serve = usage(Some("serve"));
        let server = serve.find("SERVER OPTIONS:").unwrap();
        let common = serve.find("COMMON OPTIONS:").unwrap();
        assert!(server < common && serve[server..common].contains("--token-file <file>"));
    }

    #[test]
    fn positional_shorthand_builds_requests() {
        assert_eq!(
            run_request(&["inspect", "doc.pdf"]),
            serde_json::json!({"input": {"path": "doc.pdf"}})
        );
        assert_eq!(
            run_request(&["validate", "doc.pdf"]),
            serde_json::json!({"input": {"path": "doc.pdf"}})
        );
        assert_eq!(
            run_request(&["compress", "doc.pdf"]),
            serde_json::json!({"input": {"path": "doc.pdf"}, "output": {"path": "doc-compressed.pdf"}})
        );
        assert_eq!(
            run_request(&["compress", "docs/doc.pdf", "out.pdf"]),
            serde_json::json!({"input": {"path": "docs/doc.pdf"}, "output": {"path": "out.pdf"}})
        );
        assert_eq!(
            run_request(&["compress", "doc.pdf", "-o", "out.pdf", "--replace"]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "output": {"path": "out.pdf", "replaceExisting": true}
            })
        );
        assert_eq!(
            run_request(&[
                "compress",
                "doc.pdf",
                "--mode",
                "lossy",
                "--jpeg-quality",
                "70",
                "--level",
                "3",
                "--max-bytes",
                "20000",
                "--on-target-miss",
                "error",
                "--allow-larger",
            ]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "output": {"path": "doc-compressed.pdf"},
                "mode": "lossy",
                "jpegQuality": 70,
                "compressionLevel": 3,
                "maxOutputBytes": 20000,
                "onTargetMiss": "error",
                "allowLargerOutput": true
            })
        );
        assert_eq!(
            run_request(&["merge", "a.pdf", "b.pdf", "merged.pdf"]),
            serde_json::json!({
                "inputs": [{"path": "a.pdf"}, {"path": "b.pdf"}],
                "output": {"path": "merged.pdf"}
            })
        );
        assert_eq!(
            run_request(&[
                "merge",
                "a.pdf",
                "b.pdf",
                "-o",
                "merged.pdf",
                "--preserve",
                "metadata,bookmarks"
            ]),
            serde_json::json!({
                "inputs": [{"path": "a.pdf"}, {"path": "b.pdf"}],
                "output": {"path": "merged.pdf"},
                "preserve": {"metadata": true, "bookmarks": true}
            })
        );
        assert_eq!(
            run_request(&["extract", "doc.pdf", "1-3", "-o", "subset.pdf"]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "pages": [1, 2, 3],
                "output": {"path": "subset.pdf"}
            })
        );
        assert_eq!(
            run_request(&[
                "remove-pages",
                "doc.pdf",
                "2,5",
                "trimmed.pdf",
                "--preserve",
                "all"
            ]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "pages": [2, 5],
                "output": {"path": "trimmed.pdf"},
                "preserve": {"metadata": true, "bookmarks": true, "annotations": true, "attachments": true}
            })
        );
        assert_eq!(
            run_request(&["reorder", "doc.pdf", "3,1,2", "ordered.pdf"]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "pages": [3, 1, 2],
                "output": {"path": "ordered.pdf"}
            })
        );
        assert_eq!(
            run_request(&[
                "reorder",
                "doc.pdf",
                "--pages",
                "2,1,3",
                "-o",
                "ordered.pdf"
            ]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "pages": [2, 1, 3],
                "output": {"path": "ordered.pdf"}
            })
        );
        assert_eq!(
            run_request(&["split", "doc.pdf", "p1.pdf=1-2", "p2.pdf=3"]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "parts": [
                    {"pages": [1, 2], "output": {"path": "p1.pdf"}},
                    {"pages": [3], "output": {"path": "p2.pdf"}}
                ]
            })
        );
        assert_eq!(
            run_request(&["extract-text", "doc.pdf", "1,3", "--engine", "pdfium"]),
            serde_json::json!({"input": {"path": "doc.pdf"}, "pages": [1, 3], "engine": "pdfium"})
        );
        assert_eq!(
            run_request(&[
                "render-page",
                "doc.pdf",
                "2",
                "-o",
                "page.png",
                "--dpi",
                "72"
            ]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "page": 2,
                "output": {"path": "page.png"},
                "dpi": 72
            })
        );
        // `--` terminates flag parsing for dash-leading filenames.
        assert_eq!(
            run_request(&["inspect", "--", "-flag.pdf"]),
            serde_json::json!({"input": {"path": "-flag.pdf"}})
        );
        // `-h` after `--` or as an option value is a filename, not help.
        assert_eq!(
            run_request(&["inspect", "--", "-h"]),
            serde_json::json!({"input": {"path": "-h"}})
        );
        assert_eq!(
            run_request(&["compress", "a.pdf", "-o", "-h"]),
            serde_json::json!({"input": {"path": "a.pdf"}, "output": {"path": "-h"}})
        );
        assert!(matches!(
            parse_action(["schema", "--help"].iter().copied().map(String::from)),
            Ok(Action::Help(Some(topic))) if topic == "schema"
        ));
        // A trailing --help short-circuits even after --request-file names a
        // missing file; without --help the file error surfaces normally.
        assert!(matches!(
            parse_action(
                ["inspect", "--request-file", "definitely-missing.json", "--help"]
                    .iter()
                    .copied()
                    .map(String::from)
            ),
            Ok(Action::Help(Some(topic))) if topic == "inspect"
        ));
        assert_eq!(
            parse_err(&["inspect", "--request-file", "definitely-missing.json"]),
            "NOT_FOUND"
        );
    }

    #[test]
    fn positional_shorthand_rejects_bad_shapes() {
        for args in [
            &["inspect", "a.pdf", "b.pdf"][..],
            &["inspect", "a.pdf", "--request-json", "{}"][..],
            &["compress", "a.pdf", "b.pdf", "-o", "c.pdf"][..],
            &["compress", "--mode", "lossy"][..],
            &["compress", "a.pdf", "--request-file", "r.json"][..],
            &["merge", "a.pdf", "b.pdf"][..],
            &["extract", "doc.pdf", "1", "out.pdf", "-o", "other.pdf"][..],
            &["extract", "doc.pdf", "3-1", "-o", "x.pdf"][..],
            &["extract", "doc.pdf", "abc", "-o", "x.pdf"][..],
            &["extract", "doc.pdf", "1-2001", "-o", "x.pdf"][..],
            &[
                "extract",
                "doc.pdf",
                "1-18446744073709551615",
                "-o",
                "x.pdf",
            ][..],
            &["split", "doc.pdf", "p1.pdf"][..],
            &["split", "doc.pdf", "no-equals-sign", "p2.pdf=3"][..],
            &["render-page", "doc.pdf", "two", "-o", "p.png"][..],
            &["inspect", "a.pdf", "--output", "x.pdf"][..],
            &["inspect", "a.pdf", "--preserve", "bogus"][..],
            &["serve", "extra.pdf"][..],
            &["serve"][..],
            &["operations", "x.pdf"][..],
            &["fill-forms"][..],
            &["-V", "extra"][..],
        ] {
            parse_err(args);
        }
        assert_eq!(parse_err(&["fill-forms"]), "UNSUPPORTED_OPERATION");
        assert_eq!(parse_err(&["serve"]), "INVALID_REQUEST");
    }

    /// Errors name the flag, field or file that failed.
    fn parse_detail(args: &[&str]) -> String {
        match parse_action(args.iter().copied().map(String::from)) {
            Err(e) => e.message(),
            other => panic!("expected error for {args:?}, got {other:?}"),
        }
    }

    #[test]
    fn error_details_name_flags_fields_and_files() {
        let d = parse_detail(&["compress", "--mode"]);
        assert!(d.contains("--mode expects a value"), "{d}");
        let d = parse_detail(&["compress", "a.pdf", "--level", "x"]);
        assert!(d.contains("--level expects 1, 2 or 3"), "{d}");
        let d = parse_detail(&["inspect", "a.pdf", "--output", "x.pdf"]);
        assert!(d.contains("--output is not valid for 'inspect'"), "{d}");
        let d = parse_detail(&["extract", "doc.pdf", "1", "out.pdf", "-o", "other.pdf"]);
        assert!(d.contains("output given twice"), "{d}");
        let d = parse_detail(&["merge", "a.pdf"]);
        assert!(d.contains("merge expects"), "{d}");
        let d = parse_detail(&["extract", "doc.pdf", "1-2", "--pages", "3"]);
        assert!(d.contains("pages given both"), "{d}");
        let d = parse_detail(&[
            "extract", "doc.pdf", "1-2", "--output", "x.pdf", "--pages", "3",
        ]);
        assert!(d.contains("pages given both"), "{d}");
        let d = parse_detail(&["serve"]);
        assert!(d.contains("--token-file"), "{d}");
        let d = parse_detail(&["bogus"]);
        assert!(d.contains("unknown command 'bogus'"), "{d}");
        let d = parse_detail(&["compress", "a.pdf", "--request-json", "{}"]);
        assert!(d.contains("--request-json/--request-file"), "{d}");
        let d = parse_detail(&["compress", "--level", "2"]);
        assert!(d.contains("apply only to positional shorthand"), "{d}");
        let d = parse_detail(&["extract", "doc.pdf", "abc", "-o", "x.pdf"]);
        assert!(d.contains("invalid pages 'abc'"), "{d}");
        let d = parse_detail(&["render-page", "doc.pdf", "two"]);
        assert!(d.contains("page 'two' is not a positive integer"), "{d}");
        let d = parse_detail(&["split", "doc.pdf", "p1.pdf", "p2.pdf=3"]);
        assert!(d.contains("<part.pdf=pages>"), "{d}");
    }

    #[test]
    fn pages_flag_shapes_requests() {
        assert_eq!(
            run_request(&["extract", "doc.pdf", "--pages", "2-4", "-o", "x.pdf"]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "pages": [2, 3, 4],
                "output": {"path": "x.pdf"}
            })
        );
        assert_eq!(
            run_request(&["extract", "doc.pdf", "x.pdf", "--pages", "1,3"]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "pages": [1, 3],
                "output": {"path": "x.pdf"}
            })
        );
        assert_eq!(
            parse_err(&["extract", "doc.pdf", "--pages", "1,3"]),
            "INVALID_REQUEST"
        );
        assert_eq!(
            run_request(&["reorder", "doc.pdf", "x.pdf", "--pages", "3,1,2"]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "pages": [3, 1, 2],
                "output": {"path": "x.pdf"}
            })
        );
        assert_eq!(
            run_request(&["reorder", "doc.pdf", "--pages", "3,1,2", "-o", "1-3"]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "pages": [3, 1, 2],
                "output": {"path": "1-3"}
            })
        );
        assert_eq!(
            parse_err(&["reorder", "doc.pdf", "1-3", "--pages", "3,1,2"]),
            "INVALID_REQUEST"
        );
        assert_eq!(
            run_request(&["remove-pages", "doc.pdf", "--pages", "5", "out.pdf"]),
            serde_json::json!({
                "input": {"path": "doc.pdf"},
                "pages": [5],
                "output": {"path": "out.pdf"}
            })
        );
        assert_eq!(
            run_request(&["extract-text", "doc.pdf", "--pages", "1-2"]),
            serde_json::json!({"input": {"path": "doc.pdf"}, "pages": [1, 2]})
        );
        assert_eq!(
            parse_err(&["compress", "doc.pdf", "--pages", "1"]),
            "INVALID_REQUEST"
        );
        assert_eq!(
            parse_err(&["merge", "a.pdf", "b.pdf", "--pages", "1"]),
            "INVALID_REQUEST"
        );
    }

    #[test]
    fn shorthand_absolute_and_tilde_paths_adapt_root() {
        let home = PathBuf::from(std::env::var("HOME").expect("HOME set"));
        // Unique per run so remove_dir_all only ever touches files this
        // test created.
        let dir = home.join(format!(
            "anpdf-abs-paths-test-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("doc.pdf");
        std::fs::write(&file, b"%PDF-1.4 test").unwrap();
        let abs = file.to_string_lossy().to_string();

        // Absolute positional: root adapts to the file's own directory.
        match parse_action(["inspect", &abs].iter().copied().map(String::from)) {
            Ok(Action::Run(o)) => {
                let request: Value = serde_json::from_str(&o.request.unwrap()).unwrap();
                assert_eq!(request, serde_json::json!({"input": {"path": "doc.pdf"}}));
                assert_eq!(o.root, std::fs::canonicalize(&dir).unwrap());
            }
            other => panic!("expected Run, got {other:?}"),
        }
        // `~` expands to HOME.
        let rel_home = file.strip_prefix(&home).unwrap().to_string_lossy();
        let tilde = format!("~/{}", rel_home.replace('\\', "/"));
        match parse_action(["inspect", &tilde].iter().copied().map(String::from)) {
            Ok(Action::Run(o)) => {
                let request: Value = serde_json::from_str(&o.request.unwrap()).unwrap();
                assert_eq!(request, serde_json::json!({"input": {"path": "doc.pdf"}}));
            }
            other => panic!("expected Run, got {other:?}"),
        }
        // Mixed relative + absolute: common-ancestor root adapts.
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("b.pdf"), b"%PDF-1.4 test").unwrap();
        let abs_b = sub.join("b.pdf").to_string_lossy().to_string();
        match parse_action(
            [
                "merge",
                &abs,
                &abs_b,
                &dir.join("out.pdf").to_string_lossy(),
            ]
            .iter()
            .copied()
            .map(String::from),
        ) {
            Ok(Action::Run(o)) => {
                let request: Value = serde_json::from_str(&o.request.unwrap()).unwrap();
                assert_eq!(
                    request,
                    serde_json::json!({
                        "inputs": [{"path": "doc.pdf"}, {"path": "sub/b.pdf"}],
                        "output": {"path": "out.pdf"}
                    })
                );
            }
            other => panic!("expected Run, got {other:?}"),
        }
        // Absolute -o output joins the same adapted root.
        let abs_out = dir.join("c.pdf").to_string_lossy().to_string();
        match parse_action(
            ["compress", &abs, "-o", &abs_out]
                .iter()
                .copied()
                .map(String::from),
        ) {
            Ok(Action::Run(o)) => {
                let request: Value = serde_json::from_str(&o.request.unwrap()).unwrap();
                assert_eq!(request["output"], serde_json::json!({"path": "c.pdf"}));
            }
            other => panic!("expected Run, got {other:?}"),
        }
        // Explicit --root pins the sandbox; absolute paths outside it fail.
        assert_eq!(
            parse_err(&["inspect", "--root", "/usr", &abs]),
            "INVALID_REQUEST"
        );
        let d = parse_detail(&["inspect", "--root", "/usr", &abs]);
        assert!(d.contains("outside the workspace root"), "{d}");
        // A path inside the pinned root relativizes normally.
        match parse_action(
            ["inspect", "--root", home.to_string_lossy().as_ref(), &abs]
                .iter()
                .copied()
                .map(String::from),
        ) {
            Ok(Action::Run(o)) => {
                let request: Value = serde_json::from_str(&o.request.unwrap()).unwrap();
                assert_eq!(
                    request["input"]["path"],
                    serde_json::json!(format!(
                        "{}/doc.pdf",
                        dir.file_name().unwrap().to_string_lossy()
                    ))
                );
            }
            other => panic!("expected Run, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn shorthand_guards_symlinks_dirs_and_pages_slots() {
        let home = PathBuf::from(std::env::var("HOME").expect("HOME set"));
        let dir = home.join(format!(
            "anpdf-guards-test-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("doc.pdf");
        std::fs::write(&file, b"%PDF-1.4 test").unwrap();
        let abs = file.to_string_lossy().to_string();

        // A pages spec stays a spec when another argument is absolute.
        let abs_out = dir.join("out.pdf").to_string_lossy().to_string();
        match parse_action(
            ["extract", &abs, "1-3", &abs_out]
                .iter()
                .copied()
                .map(String::from),
        ) {
            Ok(Action::Run(o)) => {
                let request: Value = serde_json::from_str(&o.request.unwrap()).unwrap();
                assert_eq!(request["pages"], serde_json::json!([1, 2, 3]));
                assert_eq!(request["output"], serde_json::json!({"path": "out.pdf"}));
            }
            other => panic!("expected Run, got {other:?}"),
        }
        match parse_action(
            ["reorder", &abs, "3,1,2", &abs_out]
                .iter()
                .copied()
                .map(String::from),
        ) {
            Ok(Action::Run(o)) => {
                let request: Value = serde_json::from_str(&o.request.unwrap()).unwrap();
                assert_eq!(request["pages"], serde_json::json!([3, 1, 2]));
                assert_eq!(request["output"], serde_json::json!({"path": "out.pdf"}));
            }
            other => panic!("expected Run, got {other:?}"),
        }
        // Trailing-slash destinations are directories, not files.
        let dir_out = format!("{abs}/");
        assert_eq!(
            parse_err(&["compress", &abs, "-o", &dir_out]),
            "INVALID_REQUEST"
        );
        let d = parse_detail(&["compress", &abs, "-o", &dir_out]);
        assert!(d.contains("ends with '/'"), "{d}");
        // Symlinked parent directories are refused rather than silently
        // canonicalized around the workspace's O_NOFOLLOW checks.
        let real = dir.join("real");
        let link = dir.join("link");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let via_link = link.join("x.pdf").to_string_lossy().to_string();
        assert_eq!(parse_err(&["inspect", &via_link]), "ACCESS_DENIED");
        let d = parse_detail(&["inspect", &via_link]);
        assert!(d.contains("symlink"), "{d}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn human_result_summarizes_and_json_flag_parses() {
        let v: Value = serde_json::json!({
            "contractVersion": "1.0.0", "operationId": "compressPdf",
            "result": {"path": "x-compressed.pdf", "bytes": 400, "pageCount": 3,
                       "inputBytes": 1000, "savedBytes": 600, "compressionRatio": 0.4,
                       "mode": "lossless", "outputSource": "compressed", "techniques": []},
            "warnings": [{"code": "NO_SIZE_REDUCTION", "message": "kept original"}]
        });
        let s = super::human_result(&v, None).unwrap();
        assert!(s.contains("wrote 'x-compressed.pdf'"), "{s}");
        assert!(s.contains("saved"), "{s}");
        assert!(
            s.contains("warning NO_SIZE_REDUCTION: kept original"),
            "{s}"
        );
        let split = serde_json::json!({"operationId": "splitPdf", "result": {
            "outputs": [{"path": "a.pdf", "pageCount": 1, "bytes": 10},
                        {"path": "b.pdf", "pageCount": 2, "bytes": 20}]}});
        assert!(
            super::human_result(&split, None)
                .unwrap()
                .contains("wrote 2 outputs")
        );
        // inspectPdf reports forms/signatures/actions as enum strings.
        let insp = serde_json::json!({"operationId": "inspectPdf", "result": {
            "pageCount": 2, "bytes": 9, "pdfVersion": "1.7", "structuralStatus": "valid",
            "encrypted": false, "forms": "present", "signatures": "unknown",
            "actions": "absent", "hasMetadata": true, "hasOutline": false,
            "hasAttachments": false}});
        let is = super::human_result(&insp, None).unwrap();
        assert!(
            is.contains("forms") && is.contains("signatures?") && is.contains("metadata"),
            "{is}"
        );
        // Paths escape control chars and anchor to a non-cwd root.
        let evil = serde_json::json!({"operationId": "mergePdf", "result": {
            "path": "x\u{1b}[2K.pdf", "pageCount": 1, "bytes": 1}});
        assert!(
            super::human_result(&evil, None)
                .unwrap()
                .contains("\\u{1b}")
        );
        let tmp_root = std::env::temp_dir().canonicalize().unwrap();
        assert!(
            super::human_result(&evil, Some(&tmp_root))
                .unwrap()
                .contains(tmp_root.to_str().unwrap())
        );
        // --json is a presentation flag, not request shaping.
        match parse_action(
            ["inspect", "--json", "doc.pdf"]
                .iter()
                .copied()
                .map(String::from),
        ) {
            Ok(Action::Run(o)) => assert!(o.json),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn page_list_and_derived_output() {
        assert_eq!(parse_page_list("1").unwrap(), vec![1]);
        assert_eq!(parse_page_list("1,3").unwrap(), vec![1, 3]);
        assert_eq!(parse_page_list("1-3,7").unwrap(), vec![1, 2, 3, 7]);
        assert_eq!(parse_page_list("3,1,2").unwrap(), vec![3, 1, 2]);
        assert_eq!(parse_page_list("2-3,1").unwrap(), vec![2, 3, 1]);
        assert_eq!(parse_page_list(" 2 - 4 ").unwrap(), vec![2, 3, 4]);
        for spec in ["", "a", "3-1", "1-", "-2", "1,,2"] {
            assert!(parse_page_list(spec).is_err(), "{spec}");
        }
        assert_eq!(derived_compress_output("a.pdf"), "a-compressed.pdf");
        assert_eq!(derived_compress_output("dir/a.pdf"), "dir/a-compressed.pdf");
        assert_eq!(derived_compress_output("a"), "a-compressed");
        assert_eq!(derived_compress_output(".hidden"), ".hidden-compressed");
        assert_eq!(
            derived_compress_output("dir/.hidden"),
            "dir/.hidden-compressed"
        );
    }

    #[test]
    fn request_from_positionals_rejects_non_pdf_commands() {
        let cli = super::CliArgs::default();
        for command in ["serve", "mcp", "operations", "schema"] {
            assert!(request_from_positionals(command, &["a.pdf".into()], &cli).is_err());
        }
    }

    #[test]
    fn pdfium_discovery_precedence_and_bundled_layouts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let bin = root.join("bin");
        let lib = root.join("lib/anpdf");
        fs::create_dir(&bin).unwrap();
        fs::create_dir_all(&lib).unwrap();
        let executable = bin.join("anpdf");
        fs::write(&executable, b"synthetic executable").unwrap();
        let name = if cfg!(target_os = "macos") {
            "libpdfium.dylib"
        } else {
            "libpdfium.so"
        };
        let bundled = lib.join(name);
        let flat = bin.join(name);
        assert_eq!(
            resolve_pdfium(None, None, Some(executable.clone())).unwrap(),
            None
        );
        fs::write(&flat, b"flat").unwrap();
        assert_eq!(
            resolve_pdfium(None, None, Some(executable.clone())).unwrap(),
            Some(flat.clone())
        );
        fs::write(&bundled, b"bundled").unwrap();
        let link = root.join("anpdf-link");
        std::os::unix::fs::symlink(&executable, &link).unwrap();
        assert_eq!(
            resolve_pdfium(None, Some(OsString::new()), Some(link.clone())).unwrap(),
            Some(bundled.clone())
        );
        assert_eq!(
            resolve_pdfium(
                None,
                Some(flat.clone().into_os_string()),
                Some(link.clone())
            )
            .unwrap(),
            Some(flat.clone())
        );
        assert_eq!(
            resolve_pdfium(
                Some(bundled.clone()),
                Some(flat.into_os_string()),
                Some(link.clone())
            )
            .unwrap(),
            Some(bundled.clone())
        );
        assert_eq!(
            resolve_pdfium(Some(root.join("missing")), None, Some(link)).unwrap(),
            None
        );
        assert!(resolve_pdfium(Some("relative".into()), None, None).is_err());
        assert!(resolve_pdfium(None, Some("relative".into()), None).is_err());
        assert_eq!(
            resolve_pdfium(Some("/dev/null".into()), None, None).unwrap(),
            None
        );
        assert_eq!(resolve_pdfium(None, None, None).unwrap(), None);
        fs::remove_file(&bundled).unwrap();
        std::os::unix::fs::symlink(&lib, &bundled).unwrap();
        assert_ne!(
            resolve_pdfium(None, None, Some(executable.clone())).unwrap(),
            Some(bundled.clone())
        );
        assert_eq!(resolve_pdfium(Some(bundled), None, None).unwrap(), None);
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(resolve_pdfium(None, None, Some(executable)).unwrap(), None);
    }

    #[test]
    fn resolve_qpdf_prefers_explicit_and_rejects_untrusted_path() {
        assert_eq!(
            resolve_qpdf(Some(PathBuf::from("/custom/qpdf")), None, None).unwrap(),
            PathBuf::from("/custom/qpdf")
        );
        assert!(resolve_qpdf(Some(PathBuf::from("relative/qpdf")), None, None).is_err());
        assert!(resolve_qpdf(None, Some(OsString::from("relative/qpdf")), None).is_err());
        assert_eq!(
            resolve_qpdf(None, Some(OsString::from("/env/qpdf")), None).unwrap(),
            PathBuf::from("/env/qpdf")
        );

        // Trusted directory vs world-writable directory on PATH.
        let tmp = tempfile::tempdir().unwrap();
        let writable = tmp.path().join("writable");
        let trusted = tmp.path().join("trusted");
        fs::create_dir(&writable).unwrap();
        fs::create_dir(&trusted).unwrap();
        fs::write(writable.join("qpdf"), b"#!/bin/sh\n").unwrap();
        fs::write(trusted.join("qpdf"), b"#!/bin/sh\n").unwrap();
        fs::set_permissions(writable.join("qpdf"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(trusted.join("qpdf"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&writable, fs::Permissions::from_mode(0o777)).unwrap();
        fs::set_permissions(&trusted, fs::Permissions::from_mode(0o755)).unwrap();

        let joined = std::env::join_paths([&writable, &trusted]).unwrap();
        let found = resolve_qpdf(None, None, Some(joined)).unwrap();
        // If one of STANDARD_QPDF_PATHS exists on the host, it wins; otherwise `trusted/qpdf` wins over `writable/qpdf`.
        assert!(found.is_absolute());
        assert_ne!(found, writable.join("qpdf"));
    }

    #[test]
    fn shell_completions_cover_commands_and_reject_unknown_shells() {
        for name in ["bash", "zsh", "fish"] {
            let mut buf = Vec::new();
            clap_complete::generate(
                Shell::parse(name).unwrap().clap(),
                &mut Cli::command(),
                "anpdf",
                &mut buf,
            );
            let script = String::from_utf8(buf).unwrap();
            for cmd in [
                "inspect",
                "merge",
                "extract",
                "split",
                "remove-pages",
                "reorder",
                "compress",
                "completions",
                "serve",
                "mcp",
            ] {
                assert!(script.contains(cmd), "{name} missing {cmd}");
            }
        }
        assert!(Shell::parse("pwsh").is_err());
        assert!(Shell::parse("").is_err());
    }
}
