// SPDX-License-Identifier: GPL-3.0-only
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::Json;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};
use crate::api::handlers::ApiHandlers;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum WsMessage {
    InstallMap { url: String, name: Option<String> },
    UninstallMap { id: String },
    ListMaps,
    GetMap { id: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct WsResponse {
    success: bool,
    data: Option<serde_json::Value>,
    error: Option<String>,
}

pub struct WebSocketServer {
    handlers: Arc<ApiHandlers>,
}

impl WebSocketServer {
    pub fn new(handlers: Arc<ApiHandlers>) -> Self {
        Self { handlers }
    }
    
    pub fn router(&self) -> Router {
        let handlers = self.handlers.clone();
        Router::new().route(
            "/ws",
            get(move |ws: WebSocketUpgrade| async move {
                ws.on_upgrade(move |socket| handle_socket(socket, handlers))
            }),
        )
    }
}

async fn handle_socket(socket: WebSocket, handlers: Arc<ApiHandlers>) {
    let (sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel();
    
    let mut send_task = tokio::spawn(async move {
        let mut sender = sender;
        while let Some(msg) = rx.recv().await {
            if let Err(e) = sender.send(msg).await {
                error!(error = %e, "Failed to send WebSocket message");
                break;
            }
        }
    });
    
    let handlers_clone = handlers.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    if let Err(e) = handle_message(text, &handlers_clone, &tx).await {
                        error!(error = %e, "Failed to handle WebSocket message");
                    }
                }
                Message::Close(_) => {
                    info!("WebSocket connection closed");
                    break;
                }
                _ => {}
            }
        }
    });
    
    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    };
}

async fn handle_message(
    text: String,
    handlers: &ApiHandlers,
    tx: &mpsc::UnboundedSender<Message>,
) -> anyhow::Result<()> {
    let msg: WsMessage = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            let response = WsResponse {
                success: false,
                data: None,
                error: Some(format!("Invalid message format: {}", e)),
            };
            tx.send(Message::Text(serde_json::to_string(&response)?))?;
            return Ok(());
        }
    };
    
    let response = match msg {
        WsMessage::ListMaps => {
            match handlers.list_maps().await {
                Ok(Json(api_response)) => WsResponse {
                    success: api_response.success,
                    data: api_response.data.map(|v| serde_json::to_value(v).unwrap_or_default()),
                    error: api_response.error,
                },
                Err(_) => WsResponse {
                    success: false,
                    data: None,
                    error: Some("Internal server error".to_string()),
                },
            }
        }
        WsMessage::GetMap { id } => {
            match handlers.get_map(axum::extract::Path(id.clone())).await {
                Ok(Json(api_response)) => WsResponse {
                    success: api_response.success,
                    data: api_response.data.map(|v| serde_json::to_value(v).unwrap_or_default()),
                    error: api_response.error,
                },
                Err(_) => WsResponse {
                    success: false,
                    data: None,
                    error: Some("Map not found".to_string()),
                },
            }
        }
        WsMessage::InstallMap { url, name } => {
            let request = crate::api::handlers::InstallMapRequest { url, name };
            match handlers.install_map(axum::Json(request)).await {
                Ok(Json(api_response)) => WsResponse {
                    success: api_response.success,
                    data: api_response.data.map(|v| serde_json::to_value(v).unwrap_or_default()),
                    error: api_response.error,
                },
                Err(_) => WsResponse {
                    success: false,
                    data: None,
                    error: Some("Failed to install map".to_string()),
                },
            }
        }
        WsMessage::UninstallMap { id } => {
            match handlers.uninstall_map(axum::extract::Path(id.clone())).await {
                Ok(Json(api_response)) => WsResponse {
                    success: api_response.success,
                    data: api_response.data.map(|v| serde_json::to_value(v).unwrap_or_default()),
                    error: api_response.error,
                },
                Err(_) => WsResponse {
                    success: false,
                    data: None,
                    error: Some("Failed to uninstall map".to_string()),
                },
            }
        }
    };
    
    tx.send(Message::Text(serde_json::to_string(&response)?))?;
    Ok(())
}

