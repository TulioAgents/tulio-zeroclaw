use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

/// Thin HTTP client for the Mission Control signals API.
///
/// All methods are async and intended to be spawned fire-and-forget from the
/// observer — failures are logged and never propagated to the caller.
pub struct MissionControlClient {
    http: Client,
    pub api_base: String,
    pub auth_token: String,
    pub instance_id: String,
    pub agent_id: String,
}

impl MissionControlClient {
    pub fn new(
        api_base: impl Into<String>,
        auth_token: impl Into<String>,
        instance_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("failed to build Mission Control HTTP client");

        Self {
            http,
            api_base: api_base.into(),
            auth_token: auth_token.into(),
            instance_id: instance_id.into(),
            agent_id: agent_id.into(),
        }
    }

    /// POST a lifecycle event to `/api/v1/signals/events`.
    pub async fn post_event(&self, payload: Value) {
        let url = format!(
            "{}/api/v1/signals/events",
            self.api_base.trim_end_matches('/')
        );
        self.post(url, payload).await;
    }

    /// POST an `instance.heartbeat` signal to `/api/v1/signals/heartbeat`.
    pub async fn post_heartbeat(&self, payload: Value) {
        let url = format!(
            "{}/api/v1/signals/heartbeat",
            self.api_base.trim_end_matches('/')
        );
        self.post(url, payload).await;
    }

    /// GET the instance Program from `/api/v1/programs/<instance_id>`.
    ///
    /// Returns the program markdown on success, `None` on any error (caller logs warnings).
    pub async fn get_program(&self) -> Option<String> {
        let url = format!(
            "{}/api/v1/programs/{}",
            self.api_base.trim_end_matches('/'),
            self.instance_id
        );
        let result = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .send()
            .await;

        match result {
            Ok(resp) if resp.status().is_success() => match resp.text().await {
                Ok(text) if !text.trim().is_empty() => Some(text),
                _ => None,
            },
            Ok(resp) => {
                tracing::warn!(
                    status = %resp.status(),
                    url = %url,
                    "Mission Control program fetch returned non-success status"
                );
                None
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    url = %url,
                    "Mission Control program fetch failed"
                );
                None
            }
        }
    }

    /// GET and atomically consume the agent inbox.
    ///
    /// Returns a list of command strings, empty on any error.
    pub async fn get_inbox(&self) -> Vec<String> {
        let url = format!(
            "{}/api/v1/agents/{}/{}/inbox",
            self.api_base.trim_end_matches('/'),
            self.instance_id,
            self.agent_id,
        );
        let result = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .send()
            .await;

        match result {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<Value>().await {
                    Ok(Value::Array(items)) => items
                        .into_iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect(),
                    Ok(v) => {
                        tracing::warn!(
                            "Mission Control inbox returned unexpected shape: {v}"
                        );
                        vec![]
                    }
                    Err(e) => {
                        tracing::warn!("Mission Control inbox parse error: {e}");
                        vec![]
                    }
                }
            }
            Ok(resp) => {
                tracing::warn!(
                    status = %resp.status(),
                    "Mission Control inbox fetch returned non-success status"
                );
                vec![]
            }
            Err(e) => {
                tracing::warn!(error = %e, "Mission Control inbox fetch failed");
                vec![]
            }
        }
    }

    /// Build a base signal payload with the shared required fields.
    pub fn base_payload(&self, kind: &str, state: Option<&str>, message: &str, source: &str) -> Value {
        let mut payload = json!({
            "kind": kind,
            "instanceId": self.instance_id,
            "agentId": self.agent_id,
            "message": message,
            "source": source,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        if let Some(s) = state {
            payload["state"] = json!(s);
        }
        payload
    }

    async fn post(&self, url: String, payload: Value) {
        let result = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .json(&payload)
            .send()
            .await;

        match result {
            Ok(resp) if resp.status().is_success() => {}
            Ok(resp) => {
                tracing::warn!(
                    status = %resp.status(),
                    url = %url,
                    "Mission Control signal rejected"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    url = %url,
                    "Mission Control signal delivery failed"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_client() -> MissionControlClient {
        MissionControlClient::new(
            "http://127.0.0.1:4010",
            "test-token",
            "test-instance",
            "manager",
        )
    }

    #[test]
    fn base_payload_includes_required_fields() {
        let client = make_client();
        let payload = client.base_payload("task.status", Some("in_progress"), "working", "hook");

        assert_eq!(payload["kind"], "task.status");
        assert_eq!(payload["instanceId"], "test-instance");
        assert_eq!(payload["agentId"], "manager");
        assert_eq!(payload["state"], "in_progress");
        assert_eq!(payload["source"], "hook");
        assert!(payload["timestamp"].is_string());
    }

    #[test]
    fn base_payload_without_state() {
        let client = make_client();
        let payload = client.base_payload("instance.heartbeat", None, "idle", "heartbeat");

        assert_eq!(payload["kind"], "instance.heartbeat");
        assert!(payload.get("state").is_none() || payload["state"].is_null());
    }

    #[test]
    fn client_construction_succeeds() {
        let client = make_client();
        assert_eq!(client.api_base, "http://127.0.0.1:4010");
        assert_eq!(client.instance_id, "test-instance");
        assert_eq!(client.agent_id, "manager");
    }
}
