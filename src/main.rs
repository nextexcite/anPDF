use anpdf::{
    BackendRegistry, Dispatcher,
    backend::Control,
    contract,
    error::{Error, Result},
};
use serde_json::Value;
use std::{
    ffi::OsString,
    io::{IsTerminal, Read},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
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

    fn script(self) -> &'static str {
        match self {
            Self::Bash => {
                r#"_anpdf() {
    local cur prev cmd
    local IFS=$'\n'  # keep spaced filenames as single candidates
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev=""
    if [[ $COMP_CWORD -gt 0 ]]; then
        prev="${COMP_WORDS[COMP_CWORD-1]}"
    fi

    if [[ $COMP_CWORD -eq 1 ]]; then
        COMPREPLY=( $(compgen -W "inspect validate merge extract split remove-pages compress extract-text render-page operations schema serve mcp completions help version --help -h --version -V" -- "$cur") )
        return 0
    fi

    cmd="${COMP_WORDS[1]}"
    case "$prev" in
        --root)
            COMPREPLY=( $(compgen -d -- "$cur") )
            return 0
            ;;
        --request-file|--token-file|--qpdf|--pdfium-lib|--output|-o)
            COMPREPLY=( $(compgen -f -- "$cur") )
            return 0
            ;;
        --mode)
            COMPREPLY=( $(compgen -W "lossless lossy" -- "$cur") )
            return 0
            ;;
        --on-target-miss)
            COMPREPLY=( $(compgen -W "bestEffort error" -- "$cur") )
            return 0
            ;;
        --preserve)
            COMPREPLY=( $(compgen -W "all metadata bookmarks annotations attachments" -- "$cur") )
            return 0
            ;;
    esac

    case "$cmd" in
        inspect|validate|merge|extract|split|remove-pages|compress|extract-text|render-page)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--root --qpdf --qpdf-exact --pdfium-lib --request-json --request-file --dry-run --output -o --replace --preserve --mode --jpeg-quality --level --max-bytes --on-target-miss --allow-larger --engine --dpi --help -h" -- "$cur") )
            else
                COMPREPLY=( $(compgen -f -- "$cur") )
            fi
            ;;
        serve)
            COMPREPLY=( $(compgen -W "--root --qpdf --qpdf-exact --pdfium-lib --port --token-file --help -h" -- "$cur") )
            ;;
        mcp)
            COMPREPLY=( $(compgen -W "--root --qpdf --qpdf-exact --pdfium-lib --help -h" -- "$cur") )
            ;;
        operations)
            COMPREPLY=( $(compgen -W "--pdfium-lib --help -h" -- "$cur") )
            ;;
        schema)
            COMPREPLY=( $(compgen -W "inspectPdf mergePdf extractPdf splitPdf removePages compressPdf extractText renderPage --help -h" -- "$cur") )
            ;;
        completions)
            COMPREPLY=( $(compgen -W "bash zsh fish --help -h" -- "$cur") )
            ;;
        help)
            COMPREPLY=( $(compgen -W "inspect validate merge extract split remove-pages compress extract-text render-page operations schema serve mcp completions" -- "$cur") )
            ;;
    esac
    return 0
}
complete -F _anpdf anpdf
"#
            }
            Self::Zsh => {
                r#"#compdef anpdf

_anpdf() {
    local -a commands pdf_ops schema_ops shells
    commands=(
        'inspect:Inspect PDF structure, metadata, and page geometry'
        'validate:Alias for inspect'
        'merge:Merge 2..20 PDFs in input order'
        'extract:Extract an ascending unique page subset'
        'split:Split a PDF into 2..20 page-subset outputs'
        'remove-pages:Remove selected pages and keep remaining pages'
        'compress:Compress PDF streams (lossless) or images (lossy)'
        'extract-text:Extract embedded text and bounding boxes'
        'render-page:Render one page to PNG'
        'operations:Print supported operations, schemas, and limits'
        'schema:Print JSON schemas for one operation'
        'serve:Run authenticated loopback HTTP server'
        'mcp:Run newline-delimited JSON-RPC 2.0 MCP server on stdio'
        'completions:Print shell completion script (bash, zsh, fish)'
        'help:Show help text'
        'version:Show version'
    )
    pdf_ops=(
        '--root[Workspace root]:dir:_files -/'
        '--qpdf[Absolute path to qpdf binary]:qpdf:_files'
        '--qpdf-exact[Require tested qpdf 12.4.2]'
        '--pdfium-lib[Absolute path to optional PDFium library]:library:_files'
        '--request-json[Inline JSON request payload]:json:'
        '--request-file[Read JSON request payload from file]:file:_files'
        '--dry-run[Validate request and destination policy without processing the PDF]'
        '(-o --output)'{-o,--output}'[Output path]:file:_files'
        '--replace[Allow replacing an existing output]'
        '--preserve[Required preserved aspects]:aspects:'
        '--mode[Compression mode]:mode:(lossless lossy)'
        '--jpeg-quality[JPEG quality 0-100]:quality:'
        '--level[Resample floor 1-3]:level:(1 2 3)'
        '--max-bytes[Output size ceiling in bytes]:bytes:'
        '--on-target-miss[Size-target miss behavior]:behavior:(bestEffort error)'
        '--allow-larger[Keep compressed bytes even when not smaller]'
        '--engine[Registered engine ID]:engine:'
        '--dpi[Render DPI 36-300]:dpi:'
        '(-h --help)'{-h,--help}'[Show command help]'
        '*:path:_files'
    )
    schema_ops=(inspectPdf mergePdf extractPdf splitPdf removePages compressPdf extractText renderPage)
    shells=(bash zsh fish)

    if (( CURRENT == 2 )); then
        _describe -t commands 'anpdf commands' commands
        return 0
    fi

    case "$words[2]" in
        inspect|validate|merge|extract|split|remove-pages|compress|extract-text|render-page)
            _arguments -s : $pdf_ops
            ;;
        serve)
            _arguments -s : \
                '--root[Workspace root]:dir:_files -/' \
                '--qpdf[Absolute path to qpdf binary]:qpdf:_files' \
                '--qpdf-exact[Require tested qpdf 12.4.2]' \
                '--pdfium-lib[Absolute path to optional PDFium library]:library:_files' \
                '--port[Loopback HTTP port]:port:' \
                '--token-file[Bearer token file]:file:_files' \
                '(-h --help)'{-h,--help}'[Show command help]'
            ;;
        mcp)
            _arguments -s : \
                '--root[Workspace root]:dir:_files -/' \
                '--qpdf[Absolute path to qpdf binary]:qpdf:_files' \
                '--qpdf-exact[Require tested qpdf 12.4.2]' \
                '--pdfium-lib[Absolute path to optional PDFium library]:library:_files' \
                '(-h --help)'{-h,--help}'[Show command help]'
            ;;
        operations)
            _arguments -s : '--pdfium-lib[Absolute path to optional PDFium library]:library:_files' '(-h --help)'{-h,--help}'[Show command help]'
            ;;
        schema)
            _arguments -s : \
                '(-h --help)'{-h,--help}'[Show command help]' \
                '1:operationId:('"${schema_ops[*]}"')'
            ;;
        completions)
            _arguments -s : \
                '(-h --help)'{-h,--help}'[Show command help]' \
                '1:shell:('"${shells[*]}"')'
            ;;
        help)
            _describe -t commands 'anpdf commands' commands
            ;;
    esac
}

if [ "$funcstack[1]" = "_anpdf" ]; then
    _anpdf "$@"
else
    compdef _anpdf anpdf
fi
"#
            }
            Self::Fish => {
                r#"set -l anpdf_cmds inspect validate merge extract split remove-pages compress extract-text render-page operations schema serve mcp completions help version
set -l anpdf_pdf_cmds inspect validate merge extract split remove-pages compress extract-text render-page
set -l anpdf_schema_ops inspectPdf mergePdf extractPdf splitPdf removePages compressPdf extractText renderPage

complete -c anpdf -f
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a inspect -d 'Inspect PDF structure, metadata, and page geometry'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a validate -d 'Alias for inspect'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a merge -d 'Merge 2..20 PDFs in input order'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a extract -d 'Extract an ascending unique page subset'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a split -d 'Split a PDF into 2..20 page-subset outputs'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a remove-pages -d 'Remove selected pages and keep remaining pages'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a compress -d 'Compress PDF streams or images'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a extract-text -d 'Extract embedded text and bounding boxes'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a render-page -d 'Render one page to PNG'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a operations -d 'Print supported operations, schemas, and limits'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a schema -d 'Print JSON schemas for one operation'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a serve -d 'Run authenticated loopback HTTP server'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a mcp -d 'Run MCP server on stdio'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a completions -d 'Print shell completion script'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a help -d 'Show help text'
complete -c anpdf -n "not __fish_seen_subcommand_from $anpdf_cmds" -a version -d 'Show version'

complete -c anpdf -n "__fish_seen_subcommand_from $anpdf_pdf_cmds serve mcp" -l root -r -F -d 'Workspace root directory'
complete -c anpdf -n "__fish_seen_subcommand_from $anpdf_pdf_cmds serve mcp" -l qpdf -r -F -d 'Absolute path to qpdf'
complete -c anpdf -n "__fish_seen_subcommand_from $anpdf_pdf_cmds serve mcp" -l qpdf-exact -d 'Require tested qpdf 12.4.2'
complete -c anpdf -n "__fish_seen_subcommand_from $anpdf_pdf_cmds serve mcp" -l pdfium-lib -r -F -d 'Absolute path to optional PDFium library'
complete -c anpdf -n "__fish_seen_subcommand_from operations" -l pdfium-lib -r -F -d 'Absolute path to optional PDFium library'
complete -c anpdf -n "__fish_seen_subcommand_from $anpdf_pdf_cmds" -l request-json -r -d 'Inline JSON request payload'
complete -c anpdf -n "__fish_seen_subcommand_from $anpdf_pdf_cmds" -l request-file -r -F -d 'Read JSON request from file'
complete -c anpdf -n "__fish_seen_subcommand_from $anpdf_pdf_cmds" -l dry-run -d 'Validate request without processing the PDF'
complete -c anpdf -n "__fish_seen_subcommand_from $anpdf_pdf_cmds" --force-files
complete -c anpdf -n "__fish_seen_subcommand_from $anpdf_pdf_cmds" -s o -l output -r -F -d 'Output path'
complete -c anpdf -n "__fish_seen_subcommand_from $anpdf_pdf_cmds" -l replace -d 'Allow replacing an existing output'
complete -c anpdf -n "__fish_seen_subcommand_from merge extract split remove-pages" -l preserve -r -d 'Required aspects: metadata,bookmarks,annotations,attachments or all'
complete -c anpdf -n "__fish_seen_subcommand_from compress" -l mode -r -a 'lossless lossy' -d 'Compression mode'
complete -c anpdf -n "__fish_seen_subcommand_from compress" -l jpeg-quality -r -d 'JPEG quality 0-100'
complete -c anpdf -n "__fish_seen_subcommand_from compress" -l level -r -a '1 2 3' -d 'Resample floor'
complete -c anpdf -n "__fish_seen_subcommand_from compress" -l max-bytes -r -d 'Output size ceiling in bytes'
complete -c anpdf -n "__fish_seen_subcommand_from compress" -l on-target-miss -r -a 'bestEffort error' -d 'Size-target miss behavior'
complete -c anpdf -n "__fish_seen_subcommand_from compress" -l allow-larger -d 'Keep compressed bytes even when not smaller'
complete -c anpdf -n "__fish_seen_subcommand_from extract-text render-page" -l engine -r -d 'Registered engine ID'
complete -c anpdf -n "__fish_seen_subcommand_from render-page" -l dpi -r -d 'Render DPI 36-300'
complete -c anpdf -n "__fish_seen_subcommand_from serve" -l port -r -d 'Loopback HTTP port'
complete -c anpdf -n "__fish_seen_subcommand_from serve" -l token-file -r -F -d 'Bearer token file'
complete -c anpdf -n "__fish_seen_subcommand_from schema" -a "$anpdf_schema_ops" -d 'Operation ID'
complete -c anpdf -n "__fish_seen_subcommand_from completions" -a 'bash zsh fish' -d 'Shell name'
complete -c anpdf -n "__fish_seen_subcommand_from help" -a "$anpdf_cmds" -d 'Command name'
complete -c anpdf -n "__fish_seen_subcommand_from $anpdf_cmds" -s h -l help -d 'Show command help'
"#
            }
        }
    }
}

#[derive(Debug)]
enum Action {
    Help(Option<String>),
    Version,
    Completions(Shell),
    Run(Options),
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
    port: u16,
    token_file: Option<PathBuf>,
    schema: Option<String>,
}

fn usage(topic: Option<&str>) -> String {
    let overview = format!(
        "anpdf {}\n\
Local, contract-validated PDF toolkit (CLI, loopback HTTP, MCP stdio).\n\n\
USAGE:\n  anpdf <command> [options]\n  anpdf <operation> <doc.pdf> [...] [-o <output.pdf>]\n\n\
COMMANDS:\n\
  inspect | validate   Inspect PDF structure, metadata, and page geometry\n\
  merge                Merge 2..20 PDFs in input order\n\
  extract              Extract an ascending unique page subset\n\
  split                Split a PDF into 2..20 page-subset outputs\n\
  remove-pages         Remove selected pages and keep the remaining pages\n\
  compress             Compress PDF streams (lossless) or images (lossy)\n\
  extract-text         Extract embedded text and bounding boxes (optional engine)\n\
  render-page          Render one page to PNG (optional engine)\n\
  operations           Print supported operations, schemas, limits, and unsupported list\n\
  schema <operationId> Print JSON schemas for one operation\n\
  serve                Run authenticated loopback HTTP server (127.0.0.1)\n\
  mcp                  Run newline-delimited JSON-RPC 2.0 MCP server on stdio\n\
  completions <shell>  Print shell completion script (bash, zsh, fish)\n\
  help [command]       Show this help text (--help, -h)\n\
  version              Show version (--version, -V)\n\n\
SHORTHAND (paths workspace-relative to --root, or absolute/~ adapting it):\n\
  anpdf inspect <doc.pdf>\n\
  anpdf merge <a.pdf> <b.pdf> [...] [-o <merged.pdf>]\n\
  anpdf extract <doc.pdf> <pages> [-o <subset.pdf>]\n\
  anpdf split <doc.pdf> <part.pdf=pages> [<part.pdf=pages> ...]\n\
  anpdf remove-pages <doc.pdf> <pages> [-o <trimmed.pdf>]\n\
  anpdf compress <doc.pdf> [<out.pdf>] [--mode lossy] [--level 1-3] [--max-bytes N]\n\
  anpdf extract-text <doc.pdf> <pages> [--engine <id>]\n\
  anpdf render-page <doc.pdf> <page> [-o <page.png>] [--dpi 36-300] [--engine <id>]\n\n\
<pages> are 1-based numbers or ranges: \"1,3\", \"2-5\" (positional or --pages).\n\
The output is -o/--output, a trailing positional path, or <name>-compressed.pdf\n\
for compress; --replace allows replacing an existing output. Inputs are never\n\
overwritten. Shorthand paths may be absolute or start with ~: the workspace\n\
root adapts to the files' common directory (--root pins it); JSON requests\n\
stay relative to --root. A full JSON request may instead be passed via\n\
--request-json, --request-file or piped stdin; a bare operation on a\n\
terminal prints usage instead of blocking on stdin.\n\n\
COMMON OPTIONS:\n\
  -o, --output <path>  Output path\n\
  --pages <spec>       Page spec for extract/remove-pages/extract-text\n\
  --replace            Allow replacing an existing output\n\
  --preserve <list>    Required aspects: metadata,bookmarks,annotations,attachments (or \"all\")\n\
  --root <dir>         Workspace root (default: .; adapts to absolute shorthand paths)\n\
  --qpdf <abs-path>    Absolute path to qpdf 12.x >=12.4.2 (tested: 12.4.2)\n\
  --qpdf-exact         Require exactly qpdf 12.4.2\n\
  --pdfium-lib <path>  Optional PDFium dynamic library (overrides ANPDF_PDFIUM_LIB)\n\
  --request-json <str> Inline JSON request payload (or pass JSON on stdin)\n\
  --request-file <f>   Read JSON request payload from file\n\
  --dry-run            Validate request and destination policy without processing the PDF\n\
  --                   End of flags; following arguments are positional paths\n\n\
SERVER OPTIONS:\n\
  --port <port>        Loopback HTTP port for serve (default: 8787)\n\
  --token-file <file>  Bearer token file (>=32 ASCII graphic chars) for serve\n\n\
ENVIRONMENT:\n\
  ANPDF_QPDF           Absolute path to qpdf 12.x >=12.4.2 (auto-discovers trusted paths)\n\
  ANPDF_QPDF_EXACT     Set to 1 to require tested qpdf 12.4.2\n\
  ANPDF_PDFIUM_LIB     Optional absolute PDFium library path (requires pdfium feature)\n\
\n\
EXIT CODES:\n\
  0 ok · 2 invalid request · 3 not found / access denied\n\
  4 output exists / input changed · 5 processing rejected · 6 unsupported\n\
  7 limit or timeout · 8 backend unavailable / internal\n",
        env!("CARGO_PKG_VERSION")
    );
    match topic {
        Some("inspect" | "validate") => {
            "USAGE: anpdf inspect <doc.pdf> [--root <dir>] [--qpdf <abs-path>] [--dry-run]\n\
       anpdf inspect [--request-json <json> | --request-file <file>] ...\n\
Alias: validate (operationId: inspectPdf)\n\
Request: {\"input\":{\"path\":\"doc.pdf\"}}\n"
                .into()
        }
        Some("merge") => {
            "USAGE: anpdf merge <a.pdf> <b.pdf> [...] [-o <merged.pdf> | <merged.pdf>]\n\
       anpdf merge [--request-json <json> | --request-file <file>] ...\n\
       [--preserve <list>] [--replace] [--root <dir>] [--dry-run]\n\
Operation: mergePdf (2..20 inputs); without -o the last positional is the output.\n\
Request: {\"inputs\":[{\"path\":\"a.pdf\"},{\"path\":\"b.pdf\"}],\"output\":{\"path\":\"merged.pdf\"}}\n"
                .into()
        }
        Some("extract") => {
            "USAGE: anpdf extract <doc.pdf> <pages> [--pages <spec>] [-o <subset.pdf> | <subset.pdf>]\n\
       anpdf extract [--request-json <json> | --request-file <file>] ...\n\
       [--preserve <list>] [--replace] [--root <dir>] [--dry-run]\n\
Operation: extractPdf (strictly increasing 1-based pages; e.g. \"1,3\" or \"2-5\")\n\
Request: {\"input\":{\"path\":\"doc.pdf\"},\"pages\":[1,3],\"output\":{\"path\":\"subset.pdf\"}}\n"
                .into()
        }
        Some("split") => {
            "USAGE: anpdf split <doc.pdf> <part.pdf=pages> [<part.pdf=pages> ...]\n\
       anpdf split [--request-json <json> | --request-file <file>] ...\n\
       [--preserve <list>] [--replace] [--root <dir>] [--dry-run]\n\
Operation: splitPdf (2..20 parts); each part is written as <output>=<pages>.\n\
Request: {\"input\":{\"path\":\"doc.pdf\"},\"parts\":[{\"pages\":[1],\"output\":{\"path\":\"p1.pdf\"}},{\"pages\":[2,3],\"output\":{\"path\":\"p2.pdf\"}}]}\n"
                .into()
        }
        Some("remove-pages") => {
            "USAGE: anpdf remove-pages <doc.pdf> <pages> [--pages <spec>] [-o <trimmed.pdf> | <trimmed.pdf>]\n\
       anpdf remove-pages [--request-json <json> | --request-file <file>] ...\n\
       [--preserve <list>] [--replace] [--root <dir>] [--dry-run]\n\
Operation: removePages (strictly increasing 1-based pages to remove; at least 1 page must remain)\n\
Request: {\"input\":{\"path\":\"doc.pdf\"},\"pages\":[2],\"output\":{\"path\":\"trimmed.pdf\"}}\n"
                .into()
        }
        Some("compress") => {
            "USAGE: anpdf compress <doc.pdf> [<out.pdf> | -o <out.pdf>]\n\
       [--mode lossless|lossy] [--jpeg-quality 0-100] [--level 1-3]\n\
       [--max-bytes N] [--on-target-miss bestEffort|error] [--allow-larger]\n\
       [--replace] [--root <dir>] [--dry-run]\n\
       anpdf compress [--request-json <json> | --request-file <file>] ...\n\
Operation: compressPdf; output defaults to <name>-compressed.pdf.\n\
mode: lossless (streams) | lossy (adds image optimization); level 1..3 sets the\n\
resample floor (1 lossless, 2 ≈200dpi, 3 ≈120dpi); --max-bytes descends a\n\
DPI/quality ladder toward the floor, --on-target-miss picks bestEffort|error.\n\
Request: {\"input\":{\"path\":\"doc.pdf\"},\"output\":{\"path\":\"compressed.pdf\"},\"mode\":\"lossless\"}\n"
                .into()
        }
        Some("extract-text") => {
            "USAGE: anpdf extract-text <doc.pdf> <pages> [--pages <spec>] [--engine <id>]\n\
       anpdf extract-text [--request-json <json> | --request-file <file>] ...\n\
       [--root <dir>] [--dry-run]\n\
Operation: extractText (optional text engine required)\n\
Request: {\"input\":{\"path\":\"doc.pdf\"},\"pages\":[1,3]}\n"
                .into()
        }
        Some("render-page") => {
            "USAGE: anpdf render-page <doc.pdf> <page> [-o <page.png> | <page.png>]\n\
       [--dpi 36-300] [--engine <id>] [--replace] [--root <dir>] [--dry-run]\n\
       anpdf render-page [--request-json <json> | --request-file <file>] ...\n\
Operation: renderPage (optional rendering engine required)\n\
Request: {\"input\":{\"path\":\"doc.pdf\"},\"page\":1,\"output\":{\"path\":\"page.png\"}}\n"
                .into()
        }
        Some("serve") => {
            "USAGE: anpdf serve --token-file <file> [--root <dir>] [--qpdf <abs-path>] [--port <port>]\n\
--token-file is required: a private file holding >=32 ASCII graphic characters.\n"
                .into()
        }
        Some("mcp") => "USAGE: anpdf mcp [--root <dir>] [--qpdf <abs-path>]\n".into(),
        Some("operations") => "USAGE: anpdf operations\n".into(),
        Some("schema") => "USAGE: anpdf schema <operationId>\n".into(),
        Some("completions") => "USAGE: anpdf completions <bash|zsh|fish>\n".into(),
        _ => overview,
    }
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
    "compress",
    "extract-text",
    "render-page",
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
    let expanded: Vec<String> = positionals
        .iter()
        .enumerate()
        .map(|(i, p)| expand_home(&path_part(i, p)))
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
        let dir = canonical_dir(Path::new(p))?;
        let name = Path::new(p)
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
        for p in expanded.iter().chain(expanded_output.iter()) {
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
fn derived_compress_output(input: &str) -> String {
    let basename_start = input.rfind('/').map_or(0, |i| i + 1);
    match input.rfind('.') {
        Some(dot) if dot > basename_start => {
            format!("{}-compressed{}", &input[..dot], &input[dot..])
        }
        _ => format!("{input}-compressed"),
    }
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
    let looks_like_pages = |s: &String| {
        s.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ',' | '-' | ' '))
    };
    let pages_for = |positional: Option<&String>| -> Result<Value> {
        match (&cli.pages, positional) {
            (Some(_), Some(pos)) if looks_like_pages(pos) => {
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
            serde_json::json!({"input": {"path": positionals[0]}})
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
        "extract" | "remove-pages" => {
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

fn parse_action<I: Iterator<Item = String>>(mut args: I) -> Result<Action> {
    let Some(command) = args.next() else {
        return Ok(Action::Help(None));
    };
    match command.as_str() {
        "--help" | "-h" => {
            if args.next().is_some() {
                return Err(bad("unexpected argument after --help"));
            }
            return Ok(Action::Help(None));
        }
        "help" => {
            let topic = args.next();
            if args.next().is_some() {
                return Err(bad("help takes at most one command topic"));
            }
            return Ok(Action::Help(topic));
        }
        "--version" | "-V" | "version" => {
            if args.next().is_some() {
                return Err(bad("version takes no arguments"));
            }
            return Ok(Action::Version);
        }
        "completions" => {
            let arg = args
                .next()
                .ok_or(bad("completions expects a shell: bash, zsh or fish"))?;
            if args.next().is_some() {
                return Err(bad("unexpected argument after the shell name"));
            }
            if arg == "--help" || arg == "-h" {
                return Ok(Action::Help(Some("completions".into())));
            }
            return Ok(Action::Completions(Shell::parse(&arg)?));
        }
        _ => {}
    }
    let rest: Vec<String> = args.collect();

    let mut args = rest.into_iter();
    let mut o = Options {
        command,
        root: PathBuf::from("."),
        qpdf: PathBuf::new(),
        qpdf_exact: false,
        pdfium_lib: None,
        request: None,
        dry: false,
        port: 8787,
        token_file: None,
        schema: None,
    };
    let mut cli = CliArgs::default();
    let mut positionals: Vec<String> = Vec::new();
    let mut qpdf_flag = None;
    let mut request_file: Option<PathBuf> = None;
    if o.command == "schema" {
        let arg = args.next().ok_or(bad(
            "schema expects an operationId — see 'anpdf operations'",
        ))?;
        if arg == "--help" || arg == "-h" {
            return Ok(Action::Help(Some("schema".into())));
        }
        o.schema = Some(arg);
    }
    let mut seen = std::collections::HashSet::new();
    while let Some(flag) = args.next() {
        if flag == "--" {
            positionals.extend(args);
            break;
        }
        if !flag.starts_with('-') {
            positionals.push(flag);
            continue;
        }
        let canonical = if flag == "-o" {
            "--output"
        } else {
            flag.as_str()
        };
        if canonical == "--help" || canonical == "-h" {
            return if KNOWN_COMMANDS.contains(&o.command.as_str()) {
                Ok(Action::Help(Some(o.command.clone())))
            } else {
                Err(Error(
                    "UNSUPPORTED_OPERATION",
                    Some(format!(
                        "unknown command '{}' — see 'anpdf help'",
                        o.command
                    )),
                ))
            };
        }
        if !seen.insert(canonical.to_string()) {
            return Err(bad(format!("{canonical} given twice")));
        }
        match canonical {
            "--pdfium-lib" => {
                o.pdfium_lib = Some(PathBuf::from(args.next().ok_or(need(canonical))?))
            }
            "--root" => o.root = PathBuf::from(args.next().ok_or(need(canonical))?),
            "--qpdf" => qpdf_flag = Some(PathBuf::from(args.next().ok_or(need(canonical))?)),
            "--qpdf-exact" => o.qpdf_exact = true,
            "--request-json" => o.request = Some(args.next().ok_or(need(canonical))?),
            "--request-file" => {
                request_file = Some(PathBuf::from(args.next().ok_or(need(canonical))?))
            }
            "--dry-run" => o.dry = true,
            "--port" => {
                o.port = args
                    .next()
                    .ok_or(need(canonical))?
                    .parse()
                    .map_err(|_| bad("--port expects a port number 0..65535"))?
            }
            "--token-file" => {
                o.token_file = Some(PathBuf::from(args.next().ok_or(need(canonical))?))
            }
            "--output" => cli.output = Some(args.next().ok_or(need(canonical))?),
            "--replace" => cli.replace = true,
            "--preserve" => {
                cli.preserve = Some(
                    args.next()
                        .ok_or(need(canonical))?
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                )
            }
            "--mode" => cli.mode = Some(args.next().ok_or(need(canonical))?),
            "--jpeg-quality" => {
                cli.jpeg_quality = Some(
                    args.next()
                        .ok_or(need(canonical))?
                        .parse()
                        .map_err(|_| bad("--jpeg-quality expects an integer 0-100"))?,
                )
            }
            "--level" => {
                cli.level = Some(
                    args.next()
                        .ok_or(need(canonical))?
                        .parse()
                        .map_err(|_| bad("--level expects 1, 2 or 3"))?,
                )
            }
            "--max-bytes" => {
                cli.max_bytes = Some(
                    args.next()
                        .ok_or(need(canonical))?
                        .parse()
                        .map_err(|_| bad("--max-bytes expects a byte count"))?,
                )
            }
            "--on-target-miss" => cli.on_miss = Some(args.next().ok_or(need(canonical))?),
            "--allow-larger" => cli.allow_larger = true,
            "--engine" => cli.engine = Some(args.next().ok_or(need(canonical))?),
            "--dpi" => {
                cli.dpi = Some(
                    args.next()
                        .ok_or(need(canonical))?
                        .parse()
                        .map_err(|_| bad("--dpi expects 36..300"))?,
                )
            }
            "--pages" => cli.pages = Some(args.next().ok_or(need(canonical))?),
            _ => {
                return Err(bad(format!(
                    "{canonical} is not a recognized option for '{}' — see 'anpdf help {}'",
                    o.command, o.command
                )));
            }
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
    let base: &[&str] = &[
        "--root",
        "--qpdf",
        "--qpdf-exact",
        "--pdfium-lib",
        "--request-json",
        "--request-file",
        "--dry-run",
    ];
    let allowed: &[&str] = match o.command.as_str() {
        "inspect" | "validate" => base,
        "extract-text" => &[base, &["--engine", "--pages"]].concat(),
        "merge" => &[base, &["--output", "--replace", "--preserve"]].concat(),
        "remove-pages" | "extract" => {
            &[base, &["--output", "--replace", "--preserve", "--pages"]].concat()
        }
        "render-page" => &[base, &["--output", "--replace", "--engine", "--dpi"]].concat(),
        "split" => &[base, &["--preserve", "--replace"]].concat(),
        "compress" => &[
            base,
            &[
                "--output",
                "--replace",
                "--mode",
                "--jpeg-quality",
                "--level",
                "--max-bytes",
                "--on-target-miss",
                "--allow-larger",
            ],
        ]
        .concat(),
        "serve" => &[
            "--root",
            "--qpdf",
            "--qpdf-exact",
            "--pdfium-lib",
            "--port",
            "--token-file",
        ],
        "mcp" => &["--root", "--qpdf", "--qpdf-exact", "--pdfium-lib"],
        "operations" => &["--pdfium-lib"],
        "schema" => &[],
        _ => {
            return Err(Error(
                "UNSUPPORTED_OPERATION",
                Some(format!(
                    "unknown command '{}' — see 'anpdf help'",
                    o.command
                )),
            ));
        }
    };
    if let Some(disallowed) = seen.iter().find(|s| !allowed.contains(&s.as_str())) {
        return Err(bad(format!(
            "{disallowed} is not valid for '{}' — see 'anpdf help {}'",
            o.command, o.command
        )));
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
            .filter(|f| !base.contains(&f.as_str()))
            .map(|f| f.as_str())
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
                | "compress"
                | "extract-text"
                | "render-page"
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
    Ok(Action::Run(o))
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
        "compress" => "compressPdf",
        "extract-text" => "extractText",
        "render-page" => "renderPage",
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
                .register_renderer("pdfium", provider)
                .expect("unique engine");
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

/// Report an error and exit. Piped stderr keeps the canonical JSON
/// envelope; on a terminal a person gets a readable `anpdf <cmd>: <reason>`
/// line, plus usage when the failure is arg-shape misuse.
fn report_error(e: &Error, command: &str, op: &str) -> ! {
    if std::io::stderr().is_terminal() {
        let topic = KNOWN_COMMANDS.contains(&command).then_some(command);
        eprintln!(
            "anpdf{}: {}",
            topic.map(|t| format!(" {t}")).unwrap_or_default(),
            e.message()
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
                print!("{}", shell.script());
                Ok(None)
            }
            Action::Run(o) => {
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
                let d = Arc::new(Dispatcher::with_registry(
                    &o.root,
                    configured_registry(o.qpdf, o.qpdf_exact, o.pdfium_lib)?,
                )?);
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
                        let request: Value = serde_json::from_str(&match o.request {
                            Some(s) => s,
                            None => read(std::io::stdin())?,
                        })
                        .map_err(|e| bad(format!("request is not valid JSON: {e}")))?;
                        let control = Control::default();
                        let cancel = control.clone();
                        let mut task = tokio::task::spawn_blocking(move || {
                            d.dispatch(op, &request, o.dry, &control)
                        });
                        tokio::select! {
                            result = &mut task => result.map_err(|_| Error("INTERNAL", None))?.map(Some),
                            _ = anpdf::http::shutdown_signal() => {
                                cancel.cancelled.store(true, Ordering::Relaxed);
                                let _ = task.await;
                                eprintln!("{}", Error("TIMEOUT", None).envelope(op));
                                std::process::exit(130)
                            }
                        }
                    }
                }
            }
        }
    }
    .await;
    match result {
        Ok(Some(v)) => {
            // On a terminal the result is pretty-printed; piped stdout
            // keeps the compact envelope bytes.
            if std::io::stdout().is_terminal() {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string())
                );
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
        Action, Shell, derived_compress_output, parse_action, parse_page_list,
        request_from_positionals, resolve_pdfium, resolve_qpdf,
    };
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
        let dir = home.join("anpdf-abs-paths-test");
        std::fs::create_dir_all(&dir).unwrap();
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
                    serde_json::json!("anpdf-abs-paths-test/doc.pdf")
                );
            }
            other => panic!("expected Run, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn page_list_and_derived_output() {
        assert_eq!(parse_page_list("1").unwrap(), vec![1]);
        assert_eq!(parse_page_list("1,3").unwrap(), vec![1, 3]);
        assert_eq!(parse_page_list("1-3,7").unwrap(), vec![1, 2, 3, 7]);
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
            let script = Shell::parse(name).unwrap().script();
            for cmd in [
                "inspect",
                "merge",
                "extract",
                "split",
                "remove-pages",
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
