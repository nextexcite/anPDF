use anpdf::{
    Dispatcher,
    backend::Control,
    contract,
    error::{Error, Result},
};
use serde_json::Value;
use std::{
    ffi::OsString,
    io::Read,
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

enum Action {
    Help(Option<String>),
    Version,
    Run(Options),
}

struct Options {
    command: String,
    root: PathBuf,
    qpdf: PathBuf,
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
USAGE:\n  anpdf <command> [options]\n\n\
COMMANDS:\n\
  inspect | validate   Inspect PDF structure, metadata, and page geometry\n\
  merge                Merge 2..20 PDFs in input order\n\
  extract              Extract an ascending unique page subset\n\
  split                Split a PDF into 2..20 page-subset outputs\n\
  remove-pages         Remove selected pages and keep the remaining pages\n\
  compress             Compress PDF streams (lossless) or images (lossy)\n\
  operations           Print supported operations, schemas, limits, and unsupported list\n\
  schema <operationId> Print JSON schemas for one operation\n\
  serve                Run authenticated loopback HTTP server (127.0.0.1)\n\
  mcp                  Run newline-delimited JSON-RPC 2.0 MCP server on stdio\n\
  help [command]       Show this help text (--help, -h)\n\
  version              Show version (--version, -V)\n\n\
COMMON OPTIONS:\n\
  --root <dir>         Workspace root (default: .)\n\
  --qpdf <abs-path>    Absolute path to qpdf 12.4.2 (overrides ANPDF_QPDF)\n\
  --request-json <str> Inline JSON request payload (or pass JSON on stdin)\n\
  --request-file <f>   Read JSON request payload from file\n\
  --dry-run            Validate request and destination policy without running qpdf\n\n\
SERVER OPTIONS:\n\
  --port <port>        Loopback HTTP port for serve (default: 8787)\n\
  --token-file <file>  Bearer token file (>=32 ASCII graphic chars) for serve\n\n\
ENVIRONMENT:\n\
  ANPDF_QPDF           Absolute path to qpdf 12.4.2 (auto-discovers standard/trusted PATH dirs if unset)\n",
        env!("CARGO_PKG_VERSION")
    );
    match topic {
        Some("inspect" | "validate") => {
            "USAGE: anpdf inspect [--root <dir>] [--qpdf <abs-path>] [--dry-run] [--request-json <json> | --request-file <file>]\n\
Alias: validate (operationId: inspectPdf)\n\
Request: {\"input\":{\"path\":\"doc.pdf\"}}\n"
                .into()
        }
        Some("merge") => {
            "USAGE: anpdf merge [--root <dir>] [--qpdf <abs-path>] [--dry-run] [--request-json <json> | --request-file <file>]\n\
Operation: mergePdf (2..20 inputs)\n\
Request: {\"inputs\":[{\"path\":\"a.pdf\"},{\"path\":\"b.pdf\"}],\"output\":{\"path\":\"merged.pdf\"}}\n"
                .into()
        }
        Some("extract") => {
            "USAGE: anpdf extract [--root <dir>] [--qpdf <abs-path>] [--dry-run] [--request-json <json> | --request-file <file>]\n\
Operation: extractPdf (strictly increasing 1-based pages)\n\
Request: {\"input\":{\"path\":\"doc.pdf\"},\"pages\":[1,3],\"output\":{\"path\":\"subset.pdf\"}}\n"
                .into()
        }
        Some("split") => {
            "USAGE: anpdf split [--root <dir>] [--qpdf <abs-path>] [--dry-run] [--request-json <json> | --request-file <file>]\n\
Operation: splitPdf (2..20 parts)\n\
Request: {\"input\":{\"path\":\"doc.pdf\"},\"parts\":[{\"pages\":[1],\"output\":{\"path\":\"p1.pdf\"}},{\"pages\":[2,3],\"output\":{\"path\":\"p2.pdf\"}}]}\n"
                .into()
        }
        Some("remove-pages") => {
            "USAGE: anpdf remove-pages [--root <dir>] [--qpdf <abs-path>] [--dry-run] [--request-json <json> | --request-file <file>]\n\
Operation: removePages (strictly increasing 1-based pages to remove; at least 1 page must remain)\n\
Request: {\"input\":{\"path\":\"doc.pdf\"},\"pages\":[2],\"output\":{\"path\":\"trimmed.pdf\"}}\n"
                .into()
        }
        Some("compress") => {
            "USAGE: anpdf compress [--root <dir>] [--qpdf <abs-path>] [--dry-run] [--request-json <json> | --request-file <file>]\n\
Operation: compressPdf (mode: lossless | lossy, optional jpegQuality 0..100 in lossy mode)\n\
Request: {\"input\":{\"path\":\"doc.pdf\"},\"output\":{\"path\":\"compressed.pdf\"},\"mode\":\"lossless\"}\n"
                .into()
        }
        Some("serve") => "USAGE: anpdf serve --token-file <file> [--root <dir>] [--qpdf <abs-path>] [--port <port>]\n".into(),
        Some("mcp") => "USAGE: anpdf mcp [--root <dir>] [--qpdf <abs-path>]\n".into(),
        Some("operations") => "USAGE: anpdf operations\n".into(),
        Some("schema") => "USAGE: anpdf schema <operationId>\n".into(),
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
            return Err(Error("INVALID_REQUEST"));
        }
        return Ok(p);
    }
    if let Some(val) = env_qpdf.filter(|v| !v.is_empty()) {
        let p = PathBuf::from(val);
        if !p.is_absolute() {
            return Err(Error("INVALID_REQUEST"));
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

fn parse_action() -> Result<Action> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or(Error("INVALID_REQUEST"))?;
    match command.as_str() {
        "--help" | "-h" => {
            if args.next().is_some() {
                return Err(Error("INVALID_REQUEST"));
            }
            return Ok(Action::Help(None));
        }
        "help" => {
            let topic = args.next();
            if args.next().is_some() {
                return Err(Error("INVALID_REQUEST"));
            }
            return Ok(Action::Help(topic));
        }
        "--version" | "-V" | "version" => {
            if args.next().is_some() {
                return Err(Error("INVALID_REQUEST"));
            }
            return Ok(Action::Version);
        }
        _ => {}
    }
    let rest: Vec<String> = args.collect();
    if rest.iter().any(|a| a == "--help" || a == "-h") {
        let valid_cmd = matches!(
            command.as_str(),
            "inspect"
                | "validate"
                | "merge"
                | "extract"
                | "split"
                | "remove-pages"
                | "compress"
                | "serve"
                | "mcp"
                | "operations"
                | "schema"
        );
        return if valid_cmd && rest.len() == 1 {
            Ok(Action::Help(Some(command)))
        } else if !valid_cmd {
            Err(Error("UNSUPPORTED_OPERATION"))
        } else {
            Err(Error("INVALID_REQUEST"))
        };
    }

    let mut args = rest.into_iter();
    let mut o = Options {
        command,
        root: PathBuf::from("."),
        qpdf: PathBuf::new(),
        request: None,
        dry: false,
        port: 8787,
        token_file: None,
        schema: None,
    };
    let mut qpdf_flag = None;
    if o.command == "schema" {
        o.schema = Some(args.next().ok_or(Error("INVALID_REQUEST"))?)
    }
    let mut seen = std::collections::HashSet::new();
    while let Some(flag) = args.next() {
        if !seen.insert(flag.clone()) {
            return Err(Error("INVALID_REQUEST"));
        }
        match flag.as_str() {
            "--root" => o.root = PathBuf::from(args.next().ok_or(Error("INVALID_REQUEST"))?),
            "--qpdf" => {
                qpdf_flag = Some(PathBuf::from(args.next().ok_or(Error("INVALID_REQUEST"))?))
            }
            "--request-json" => o.request = Some(args.next().ok_or(Error("INVALID_REQUEST"))?),
            "--request-file" => {
                let p = args.next().ok_or(Error("INVALID_REQUEST"))?;
                let f = std::fs::File::open(p)?;
                o.request = Some(read(f)?)
            }
            "--dry-run" => o.dry = true,
            "--port" => {
                o.port = args
                    .next()
                    .ok_or(Error("INVALID_REQUEST"))?
                    .parse()
                    .map_err(|_| Error("INVALID_REQUEST"))?
            }
            "--token-file" => {
                o.token_file = Some(PathBuf::from(args.next().ok_or(Error("INVALID_REQUEST"))?))
            }
            _ => return Err(Error("INVALID_REQUEST")),
        }
    }
    if seen.contains("--request-json") && seen.contains("--request-file") {
        return Err(Error("INVALID_REQUEST"));
    }
    let allowed: &[&str] = match o.command.as_str() {
        "inspect" | "validate" | "merge" | "extract" | "split" | "remove-pages" | "compress" => &[
            "--root",
            "--qpdf",
            "--request-json",
            "--request-file",
            "--dry-run",
        ],
        "serve" => &["--root", "--qpdf", "--port", "--token-file"],
        "mcp" => &["--root", "--qpdf"],
        "operations" | "schema" => &[],
        _ => return Err(Error("UNSUPPORTED_OPERATION")),
    };
    if seen.iter().any(|s| !allowed.contains(&s.as_str())) {
        return Err(Error("INVALID_REQUEST"));
    }
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
        return Err(Error("LIMIT_EXCEEDED"));
    }
    String::from_utf8(b).map_err(|_| Error("INVALID_REQUEST"))
}

fn id(command: &str) -> &'static str {
    match command {
        "inspect" | "validate" => "inspectPdf",
        "merge" => "mergePdf",
        "extract" => "extractPdf",
        "split" => "splitPdf",
        "remove-pages" => "removePages",
        "compress" => "compressPdf",
        _ => "unknown",
    }
}

#[tokio::main]
async fn main() {
    let command = std::env::args().nth(1).unwrap_or_default();
    let op = id(&command);
    let result = async {
        match parse_action()? {
            Action::Help(topic) => {
                print!("{}", usage(topic.as_deref()));
                Ok(None)
            }
            Action::Version => {
                println!("anpdf {}", env!("CARGO_PKG_VERSION"));
                Ok(None)
            }
            Action::Run(o) => {
                if o.command == "operations" {
                    return Ok(Some(contract::discovery()));
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
                        .ok_or(Error("UNSUPPORTED_OPERATION"));
                }
                let d = Arc::new(Dispatcher::new(&o.root, o.qpdf)?);
                match o.command.as_str() {
                    "serve" => {
                        let file = o.token_file.ok_or(Error("INVALID_REQUEST"))?;
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
                            r = &mut worker => { r.map_err(|_| Error("INTERNAL"))??; Ok(None) },
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
                        .map_err(|_| Error("INVALID_REQUEST"))?;
                        let control = Control::default();
                        let cancel = control.clone();
                        let mut task = tokio::task::spawn_blocking(move || {
                            d.dispatch(op, &request, o.dry, &control)
                        });
                        tokio::select! {
                            result = &mut task => result.map_err(|_| Error("INTERNAL"))?.map(Some),
                            _ = anpdf::http::shutdown_signal() => {
                                cancel.cancelled.store(true, Ordering::Relaxed);
                                let _ = task.await;
                                eprintln!("{}", Error("TIMEOUT").envelope(op));
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
        Ok(Some(v)) => println!("{v}"),
        Ok(None) => {}
        Err(e) => {
            eprintln!("{}", e.envelope(op));
            std::process::exit(e.exit())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_qpdf;
    use std::{ffi::OsString, fs, os::unix::fs::PermissionsExt, path::PathBuf};

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
}
