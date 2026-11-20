use crate::{
    Dispatcher,
    backend::Control,
    contract,
    error::{Error, Result},
};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::Value;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, atomic::Ordering},
};
#[derive(Clone)]
struct App {
    dispatcher: Arc<Dispatcher>,
    bearer: String,
    host: String,
}
struct Cancel(Control);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.cancelled.store(true, Ordering::Relaxed);
    }
}
fn response(value: Value, status: u16) -> Response {
    (StatusCode::from_u16(status).unwrap(), Json(value)).into_response()
}
async fn handle(State(state): State<App>, request: Request) -> Response {
    let path = request.uri().path();
    let op = contract::operations()
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, o)| o["operationId"].as_str().unwrap())
        .unwrap_or("unknown");
    let fail = |code| {
        let e = Error(code, None);
        response(e.envelope(op), e.http())
    };
    if request.headers().contains_key("origin")
        || request.headers().get("host").and_then(|v| v.to_str().ok()) != Some(state.host.as_str())
    {
        return fail("ACCESS_DENIED");
    }
    let received = request
        .headers()
        .get("authorization")
        .map(|v| v.as_bytes())
        .unwrap_or(&[]);
    let expected = state.bearer.as_bytes();
    let mismatch = received
        .iter()
        .zip(expected)
        .fold(0u8, |a, (x, y)| a | (x ^ y));
    if received.len() != expected.len() || mismatch != 0 {
        return fail("UNAUTHENTICATED");
    }
    if request.uri().query().is_some() {
        return fail("INVALID_REQUEST");
    }
    if request.method() == "GET" {
        return match path {
            "/v1/openapi.json" => response(contract::document().clone(), 200),
            "/v1/operations" => response(state.dispatcher.discovery(), 200),
            _ => fail("NOT_FOUND"),
        };
    }
    if request.method() != "POST" || op == "unknown" {
        return fail("NOT_FOUND");
    }
    if request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_none_or(|s| s.split(';').next() != Some("application/json"))
    {
        return fail("INVALID_REQUEST");
    }
    let bytes = match to_bytes(request.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return fail("LIMIT_EXCEEDED"),
    };
    let value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return fail("INVALID_REQUEST"),
    };
    let control = Control::default();
    let _cancel = Cancel(control.clone());
    let dispatcher = state.dispatcher.clone();
    match tokio::task::spawn_blocking(move || dispatcher.dispatch(op, &value, false, &control))
        .await
    {
        Ok(Ok(v)) => response(v, 200),
        Ok(Err(e)) => response(e.envelope(op), e.http()),
        Err(_) => fail("INTERNAL"),
    }
}
pub async fn shutdown_signal() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("signal registration");
    tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=terminate.recv()=>{} }
}
pub async fn serve(dispatcher: Arc<Dispatcher>, port: u16, token: String) -> Result<()> {
    if token.len() < 32 || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(Error("INVALID_REQUEST", None));
    }
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|_| Error("ACCESS_DENIED", None))?;
    let host = listener
        .local_addr()
        .map_err(|_| Error("INTERNAL", None))?
        .to_string();
    let shutdown_dispatcher = dispatcher.clone();
    let app = Router::new().fallback(handle).with_state(App {
        dispatcher,
        bearer: format!("Bearer {token}"),
        host,
    });
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown_dispatcher.cancel_all();
        })
        .await
        .map_err(|_| Error("INTERNAL", None))
}
