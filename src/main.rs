use anpdf::{
    Dispatcher,
    backend::Control,
    contract,
    error::{Error, Result},
};
use serde_json::Value;
use std::{
    io::Read,
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};
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
fn options() -> Result<Options> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or(Error("INVALID_REQUEST"))?;
    let mut o = Options {
        command,
        root: PathBuf::from("."),
        qpdf: std::env::var_os("ANPDF_QPDF")
            .map(PathBuf::from)
            .unwrap_or(PathBuf::from("/usr/bin/qpdf")),
        request: None,
        dry: false,
        port: 8787,
        token_file: None,
        schema: None,
    };
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
            "--qpdf" => o.qpdf = PathBuf::from(args.next().ok_or(Error("INVALID_REQUEST"))?),
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
        "inspect" | "validate" | "merge" | "extract" | "split" => &[
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
    Ok(o)
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
        _ => "unknown",
    }
}
#[tokio::main]
async fn main() {
    let command = std::env::args().nth(1).unwrap_or_default();
    let op = id(&command);
    let result=async{
        let o=options()?;
        if o.command=="operations"{return Ok(Some(contract::discovery()))}
        if o.command=="schema"{return contract::operation(o.schema.as_deref().unwrap()).map(|v|Some(serde_json::json!({"operation":v,"inputSchema":contract::input(o.schema.as_deref().unwrap()),"outputSchema":contract::resolve(&v["responses"]["200"]["content"]["application/json"]["schema"]),"errorSchema":contract::resolve(&contract::document()["components"]["schemas"]["Error"])}))).ok_or(Error("UNSUPPORTED_OPERATION"))}
        if !o.qpdf.is_absolute(){return Err(Error("INVALID_REQUEST"))}
        let d=Arc::new(Dispatcher::new(&o.root,o.qpdf)?);
        match o.command.as_str(){
            "serve"=>{let file=o.token_file.ok_or(Error("INVALID_REQUEST"))?;let token=read(std::fs::File::open(file)?)?.trim().to_string();anpdf::http::serve(d,o.port,token).await?;Ok(None)},
            "mcp"=>{
                let worker_dispatcher=d.clone();let mut worker=tokio::task::spawn_blocking(move||anpdf::mcp::serve(worker_dispatcher));
                tokio::select!{
                    r=&mut worker=>{r.map_err(|_|Error("INTERNAL"))??;Ok(None)},
                    _=anpdf::http::shutdown_signal()=>{d.cancel_all();while d.active_count()>0{tokio::time::sleep(std::time::Duration::from_millis(10)).await;}std::process::exit(130)}
                }
            },
            _=>{
                let request:Value=serde_json::from_str(&match o.request{Some(s)=>s,None=>read(std::io::stdin())?}).map_err(|_|Error("INVALID_REQUEST"))?;
                let control=Control::default();let cancel=control.clone();
                let mut task=tokio::task::spawn_blocking(move||d.dispatch(op,&request,o.dry,&control));
                tokio::select!{
                    result=&mut task=>result.map_err(|_|Error("INTERNAL"))?.map(Some),
                    _=anpdf::http::shutdown_signal()=>{cancel.cancelled.store(true,Ordering::Relaxed);let _=task.await;eprintln!("{}",Error("TIMEOUT").envelope(op));std::process::exit(130)}
                }
            }
        }
    }.await;
    match result {
        Ok(Some(v)) => println!("{v}"),
        Ok(None) => {}
        Err(e) => {
            eprintln!("{}", e.envelope(op));
            std::process::exit(e.exit())
        }
    }
}
