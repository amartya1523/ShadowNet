//! HTTP and WebSocket surface. Thin on purpose: every handler either reads
//! a cached blob or forwards to the simulation thread.

use std::path::PathBuf;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use shadow_graph::RouterId;
use shadow_sim::Command;
use tokio::sync::broadcast::error::RecvError;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing::debug;

use crate::runner::Handle;

pub fn router(handle: Handle, static_dir: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/topology", get(topology))
        .route("/api/history", get(history))
        .route("/api/scenarios", get(scenarios))
        .route("/api/command", post(command))
        .route("/api/trace", get(trace))
        .route("/api/router/{id}", get(router_info))
        .route("/ws", get(ws))
        .with_state(handle)
        .layer(CompressionLayer::new())
        // The dashboard dev server runs on another port; nothing here is
        // sensitive, so any origin may read it.
        .layer(CorsLayer::permissive());

    match static_dir {
        Some(dir) => {
            let index = dir.join("index.html");
            api.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => api.fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                "dashboard not built: run `npm run build` in dashboard/ or use `npm run dev` on :3000",
            )
        }),
    }
}

fn json_blob(body: impl Into<String>) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], body.into()).into_response()
}

fn unavailable() -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "ok": false, "error": "simulation thread is gone" })))
        .into_response()
}

async fn health(State(h): State<Handle>) -> Json<Value> {
    Json(json!({ "ok": true, "runId": h.run_id.load(std::sync::atomic::Ordering::Relaxed) }))
}

async fn topology(State(h): State<Handle>) -> Response {
    let body = h.topology.read().unwrap().clone();
    json_blob(body.to_string())
}

async fn history(State(h): State<Handle>) -> Response {
    match h.query(|w| serde_json::to_value(w.history()).unwrap()).await {
        Some(v) => Json(v).into_response(),
        None => unavailable(),
    }
}

async fn scenarios(State(h): State<Handle>) -> Json<Value> {
    Json(
        h.scenarios
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "title": s.title,
                    "description": s.description,
                    "durationSecs": s.duration,
                })
            })
            .collect(),
    )
}

async fn command(State(h): State<Handle>, Json(cmd): Json<Command>) -> Response {
    debug!(?cmd, "command");
    match h.command(cmd).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": error }))).into_response(),
    }
}

#[derive(Deserialize)]
struct TraceParams {
    src: u32,
    dst: u32,
}

async fn trace(State(h): State<Handle>, Query(p): Query<TraceParams>) -> Response {
    let res = h
        .query(move |w| {
            let n = w.topo.routers.len() as u32;
            if p.src >= n || p.dst >= n {
                return json!({ "error": "no such router" });
            }
            serde_json::to_value(w.trace(RouterId(p.src), RouterId(p.dst))).unwrap()
        })
        .await;
    match res {
        Some(v) if v.get("error").is_some() => (StatusCode::NOT_FOUND, Json(v)).into_response(),
        Some(v) => Json(v).into_response(),
        None => unavailable(),
    }
}

async fn router_info(State(h): State<Handle>, Path(id): Path<u32>) -> Response {
    let res = h
        .query(move |w| {
            if id as usize >= w.topo.routers.len() {
                return Value::Null;
            }
            serde_json::to_value(w.router_info(RouterId(id))).unwrap()
        })
        .await;
    match res {
        Some(Value::Null) => (StatusCode::NOT_FOUND, Json(json!({ "error": "no such router" }))).into_response(),
        Some(v) => Json(v).into_response(),
        None => unavailable(),
    }
}

async fn ws(State(h): State<Handle>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| stream(socket, h))
}

async fn stream(mut socket: WebSocket, h: Handle) {
    let mut frames = h.frames.subscribe();
    if socket.send(Message::Text(h.hello().into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            frame = frames.recv() => match frame {
                Ok(json) => {
                    if socket.send(Message::Text(json.to_string().into())).await.is_err() {
                        break;
                    }
                }
                // A slow client misses frames rather than slowing everyone
                // down. Events in skipped frames are lost to it; the next
                // frame is complete state otherwise.
                Err(RecvError::Lagged(n)) => debug!("ws client lagged by {n} frames"),
                Err(RecvError::Closed) => break,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
        }
    }
}
