//! WebSocket agent chat handler.
//!
//! Protocol:
//! ```text
//! Client -> Server: {"type":"message","content":"Hello"}
//! Server -> Client: {"type":"chunk","content":"Hi! "}
//! Server -> Client: {"type":"tool_call","name":"shell","args":{...}}
//! Server -> Client: {"type":"tool_result","name":"shell","output":"..."}
//! Server -> Client: {"type":"done","full_response":"..."}
//! ```

use super::AppState;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    http::HeaderMap,
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;

/// The sub-protocol we support for the chat WebSocket.
const WS_PROTOCOL: &str = "zeroclaw.v1";

#[derive(Deserialize)]
pub struct WsQuery {
    pub token: Option<String>,
    pub session_id: Option<String>,
    /// Optional delegate agent ID (key in `[agents.*]` config section).
    /// When set, the WebSocket session uses that agent's provider, model,
    /// and system prompt instead of the gateway defaults.
    pub agent_id: Option<String>,
}

/// GET /ws/chat — WebSocket upgrade for agent chat
pub async fn handle_ws_chat(
    State(state): State<AppState>,
    Query(params): Query<WsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Auth via query param (browser WebSocket limitation)
    if state.pairing.require_pairing() {
        let token = params.token.as_deref().unwrap_or("");
        if !state.pairing.is_authenticated(token) {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                "Unauthorized — provide ?token=<bearer_token>",
            )
                .into_response();
        }
    }

    // Echo Sec-WebSocket-Protocol if the client requests our sub-protocol.
    let ws = if headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .map_or(false, |protos| {
            protos.split(',').any(|p| p.trim() == WS_PROTOCOL)
        }) {
        ws.protocols([WS_PROTOCOL])
    } else {
        ws
    };

    let session_id = params.session_id.clone();
    let agent_id = params.agent_id.clone();
    ws.on_upgrade(move |socket| handle_socket(socket, state, session_id, agent_id))
        .into_response()
}

async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    _session_id: Option<String>,
    agent_id: Option<String>,
) {
    let (mut sender, mut receiver) = socket.split();

    // Resolve delegate agent config once, before the message loop.
    // If agent_id is given but unknown, close immediately with an error.
    struct AgentCtx {
        provider: std::sync::Arc<dyn crate::providers::Provider>,
        model: String,
        temperature: f64,
        system_prompt: String,
        provider_label: String,
    }

    let agent_ctx: AgentCtx = if let Some(ref id) = agent_id {
        let delegate_opt = {
            let cfg = state.config.lock();
            cfg.agents.get(id).cloned()
        };
        let delegate = match delegate_opt {
            Some(a) => a,
            None => {
                let err = serde_json::json!({
                    "type": "error",
                    "message": format!("Agent '{id}' not found in configuration")
                });
                let _ = sender.send(Message::Text(err.to_string().into())).await;
                return;
            }
        };

        let mut provider_name = delegate.provider.clone();
        let mut model = delegate.model.clone();
        let temperature = delegate.temperature.unwrap_or(state.temperature);
        let system_prompt = delegate.system_prompt.clone().unwrap_or_default();
        let api_key = delegate.api_key.clone();

        // Resolve hint:* model aliases via [[model_routes]]
        if let Some(hint) = model.strip_prefix("hint:") {
            let cfg = state.config.lock();
            if let Some(route) = cfg.model_routes.iter().find(|r| r.hint == hint) {
                provider_name = route.provider.clone();
                model = route.model.clone();
            }
        }

        let provider = match crate::providers::create_provider(
            &provider_name,
            api_key.as_deref(),
        ) {
            Ok(p) => std::sync::Arc::from(p),
            Err(e) => {
                let err = serde_json::json!({
                    "type": "error",
                    "message": format!("Failed to create provider for agent '{id}': {e}")
                });
                let _ = sender.send(Message::Text(err.to_string().into())).await;
                return;
            }
        };

        AgentCtx {
            provider,
            model,
            temperature,
            system_prompt,
            provider_label: provider_name,
        }
    } else {
        // Default gateway agent
        let (provider_label, system_prompt) = {
            let config_guard = state.config.lock();
            let label = config_guard
                .default_provider
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let prompt = crate::channels::build_system_prompt(
                &config_guard.workspace_dir,
                &state.model,
                &[],
                &[],
                Some(&config_guard.identity),
                None,
            );
            (label, prompt)
        };
        AgentCtx {
            provider: state.provider.clone(),
            model: state.model.clone(),
            temperature: state.temperature,
            system_prompt,
            provider_label,
        }
    };

    while let Some(msg) = receiver.next().await {
        let msg = match msg {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) | Err(_) => break,
            _ => continue,
        };

        // Parse incoming message
        let parsed: serde_json::Value = match serde_json::from_str(&msg) {
            Ok(v) => v,
            Err(_) => {
                let err = serde_json::json!({"type": "error", "message": "Invalid JSON"});
                let _ = sender.send(Message::Text(err.to_string().into())).await;
                continue;
            }
        };

        let msg_type = parsed["type"].as_str().unwrap_or("");
        if msg_type != "message" {
            continue;
        }

        let content = parsed["content"].as_str().unwrap_or("").to_string();
        if content.is_empty() {
            continue;
        }

        // Broadcast agent_start event
        let _ = state.event_tx.send(serde_json::json!({
            "type": "agent_start",
            "provider": agent_ctx.provider_label,
            "model": agent_ctx.model,
        }));

        let messages = vec![
            crate::providers::ChatMessage::system(&agent_ctx.system_prompt),
            crate::providers::ChatMessage::user(&content),
        ];

        let multimodal_config = state.config.lock().multimodal.clone();
        let prepared =
            match crate::multimodal::prepare_messages_for_provider(&messages, &multimodal_config)
                .await
            {
                Ok(p) => p,
                Err(e) => {
                    let err = serde_json::json!({
                        "type": "error",
                        "message": format!("Multimodal prep failed: {e}")
                    });
                    let _ = sender.send(Message::Text(err.to_string().into())).await;
                    continue;
                }
            };

        match agent_ctx
            .provider
            .chat_with_history(&prepared.messages, &agent_ctx.model, agent_ctx.temperature)
            .await
        {
            Ok(response) => {
                let done = serde_json::json!({
                    "type": "done",
                    "full_response": response,
                });
                let _ = sender.send(Message::Text(done.to_string().into())).await;

                let _ = state.event_tx.send(serde_json::json!({
                    "type": "agent_end",
                    "provider": agent_ctx.provider_label,
                    "model": agent_ctx.model,
                }));
            }
            Err(e) => {
                let sanitized = crate::providers::sanitize_api_error(&e.to_string());
                let err = serde_json::json!({
                    "type": "error",
                    "message": sanitized,
                });
                let _ = sender.send(Message::Text(err.to_string().into())).await;

                let _ = state.event_tx.send(serde_json::json!({
                    "type": "error",
                    "component": "ws_chat",
                    "message": sanitized,
                }));
            }
        }
    }
}
