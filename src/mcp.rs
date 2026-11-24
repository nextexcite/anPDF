use crate::{
    Dispatcher,
    backend::Control,
    contract,
    error::{Error, Result},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, Write},
    sync::{Arc, Mutex, atomic::Ordering},
};
fn send(output: &Mutex<std::io::Stdout>, v: Value) {
    let mut o = output.lock().unwrap();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}
fn protocol(output: &Mutex<std::io::Stdout>, id: Value, code: i32) {
    send(
        output,
        json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":match code{-32700=>"Parse error",-32600=>"Invalid request",-32601=>"Method not found",-32602=>"Invalid params",_=>"Internal error"}}}),
    );
}
pub fn tools() -> Value {
    tools_with_output_policy(None, None)
}
fn tools_with_output_policy(policy: Option<(&str, Option<u64>)>, writer: Option<&str>) -> Value {
    json!({"tools":contract::operations().into_iter().filter_map(|(_,op)|{
    let id=op["operationId"].as_str().unwrap();
    let read_only = matches!(id, "inspectPdf" | "extractText");
    let mut description = contract::description(op);
    if let Some((path, ceiling)) = policy.filter(|_| !read_only) {
        if id == "splitPdf" || writer.is_some_and(|writer| writer != id) {
            return None;
        }
        use std::fmt::Write as _;
        let _ = write!(description, " This session permits new output only at {path:?}; replacement is disabled.");
        if let Some(ceiling) = ceiling {
            let _ = write!(description, " Publication is limited to {ceiling} bytes.");
        }
    }
    Some(json!({"name":id,"description":description,"inputSchema":contract::input(id),
    "outputSchema":contract::resolve(&op["responses"]["200"]["content"]["application/json"]["schema"]),
    "annotations":{"readOnlyHint":read_only,"destructiveHint":!read_only,"idempotentHint":read_only,"openWorldHint":false}}))
}).collect::<Vec<_>>()})
}
pub fn serve(dispatcher: Arc<Dispatcher>) -> Result<()> {
    let output = Arc::new(Mutex::new(std::io::stdout()));
    let active: Arc<Mutex<HashMap<String, Control>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut initialized = false;
    let mut ready = false;
    let mut workers = Vec::new();
    let mut stdin = std::io::stdin().lock();
    loop {
        let mut line = Vec::new();
        let n = (&mut stdin)
            .take(1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if n > 1024 * 1024 {
            protocol(&output, Value::Null, -32700);
            break;
        }
        let v: Value = match serde_json::from_slice(&line) {
            Ok(v) => v,
            Err(_) => {
                protocol(&output, Value::Null, -32700);
                continue;
            }
        };
        let id = v.get("id").cloned();
        let method = v["method"].as_str();
        if v["jsonrpc"] != "2.0"
            || method.is_none()
            || id.as_ref().is_some_and(|i| !i.is_string() && !i.is_i64())
        {
            protocol(&output, Value::Null, -32600);
            continue;
        }
        let method = method.unwrap();
        if id.is_none() {
            match method {
                "notifications/initialized" if initialized => ready = true,
                "notifications/cancelled" => {
                    if let Some(c) = active
                        .lock()
                        .unwrap()
                        .get(&v["params"]["requestId"].to_string())
                    {
                        c.cancelled.store(true, Ordering::Relaxed);
                    }
                }
                _ => {}
            }
            continue;
        }
        let id = id.unwrap();
        if method == "initialize" {
            if initialized
                || !v["params"]["protocolVersion"].is_string()
                || !v["params"]["capabilities"].is_object()
                || !v["params"]["clientInfo"].is_object()
            {
                protocol(&output, id, -32602);
                continue;
            }
            initialized = true;
            send(
                &output,
                json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"anpdf","version":env!("CARGO_PKG_VERSION")}}}),
            );
            continue;
        }
        if method == "ping" {
            send(&output, json!({"jsonrpc":"2.0","id":id,"result":{}}));
            continue;
        }
        if !ready {
            protocol(&output, id, -32600);
            continue;
        }
        if method == "tools/list" {
            if v["params"].get("cursor").is_some() {
                protocol(&output, id, -32602)
            } else {
                let tools = tools_with_output_policy(
                    dispatcher.workspace.output_only(),
                    dispatcher.output_operation.as_deref(),
                );
                send(&output, json!({"jsonrpc":"2.0","id":id,"result":tools}))
            }
            continue;
        }
        if method != "tools/call" {
            protocol(&output, id, -32601);
            continue;
        }
        let name = match v["params"]["name"].as_str() {
            Some(n) if contract::operation(n).is_some() => n.to_owned(),
            _ => {
                protocol(&output, id, -32602);
                continue;
            }
        };
        let request = v["params"]["arguments"].clone();
        if !request.is_object() {
            protocol(&output, id, -32602);
            continue;
        }
        let control = Control::default();
        let key = id.to_string();
        {
            let mut a = active.lock().unwrap();
            if a.contains_key(&key) {
                protocol(&output, id, -32600);
                continue;
            }
            if a.len() >= 2 {
                let e = Error("LIMIT_EXCEEDED", None);
                send(
                    &output,
                    json!({"jsonrpc":"2.0","id":id,"result":{"isError":true,"content":[{"type":"text","text":e.envelope(&name).to_string()}]}}),
                );
                continue;
            }
            a.insert(key.clone(), control.clone());
        }
        let d = dispatcher.clone();
        let o = output.clone();
        let a = active.clone();
        workers.retain(|w: &std::thread::JoinHandle<()>| !w.is_finished());
        workers.push(std::thread::spawn(move||{
            let result=match d.dispatch(&name,&request,false,&control){
                Ok(v)=>json!({"isError":false,"structuredContent":v,"content":[{"type":"text","text":v.to_string()}]}),
                Err(e)=>json!({"isError":true,"content":[{"type":"text","text":e.envelope(&name).to_string()}]})};
            send(&o,json!({"jsonrpc":"2.0","id":id,"result":result}));a.lock().unwrap().remove(&key);
        }));
    }
    for c in active.lock().unwrap().values() {
        c.cancelled.store(true, Ordering::Relaxed)
    }
    for worker in workers {
        let _ = worker.join();
    }
    Ok(())
}
use std::io::Read;
