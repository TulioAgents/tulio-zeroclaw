use super::mission_control_client::MissionControlClient;
use super::traits::{Observer, ObserverEvent, ObserverMetric};
use std::any::Any;
use std::sync::Arc;

/// Observer backend that forwards agent lifecycle events to a Mission Control API.
///
/// All HTTP calls are fire-and-forget: spawned onto the Tokio runtime and never
/// block or panic the agent loop. Signal delivery failures are logged as warnings.
///
/// Construct via [`MissionControlObserver::new`] and register with the observer
/// factory by setting `observability.backend = "mission-control"` in config.
pub struct MissionControlObserver {
    client: Arc<MissionControlClient>,
    emit_task_signals: bool,
    emit_heartbeat: bool,
    runtime: tokio::runtime::Handle,
}

impl MissionControlObserver {
    /// Create a new observer.
    ///
    /// `runtime` must be a handle to the active Tokio runtime so the observer
    /// can spawn async tasks from the synchronous `record_event` hot path.
    pub fn new(
        client: MissionControlClient,
        emit_task_signals: bool,
        emit_heartbeat: bool,
    ) -> Self {
        let runtime = tokio::runtime::Handle::current();
        Self {
            client: Arc::new(client),
            emit_task_signals,
            emit_heartbeat,
            runtime,
        }
    }
}

impl Observer for MissionControlObserver {
    fn record_event(&self, event: &ObserverEvent) {
        match event {
            ObserverEvent::HeartbeatTick => {
                if !self.emit_heartbeat {
                    return;
                }
                let client = Arc::clone(&self.client);
                self.runtime.spawn(async move {
                    let payload = client.base_payload(
                        "instance.heartbeat",
                        Some("idle"),
                        "heartbeat",
                        "heartbeat",
                    );
                    client.post_heartbeat(payload).await;
                });
            }

            ObserverEvent::AgentStart { provider, model } => {
                if !self.emit_task_signals {
                    return;
                }
                let client = Arc::clone(&self.client);
                let msg = format!("Agent started: {provider}/{model}");
                self.runtime.spawn(async move {
                    let payload = client.base_payload(
                        "agent.lifecycle",
                        Some("in_progress"),
                        &msg,
                        "hook",
                    );
                    client.post_event(payload).await;
                });
            }

            ObserverEvent::AgentEnd {
                provider,
                model,
                tokens_used,
                ..
            } => {
                if !self.emit_task_signals {
                    return;
                }
                let client = Arc::clone(&self.client);
                let tokens_str = tokens_used
                    .map(|t| format!(", {t} tokens"))
                    .unwrap_or_default();
                let msg = format!("Agent session ended: {provider}/{model}{tokens_str}");
                self.runtime.spawn(async move {
                    let payload =
                        client.base_payload("task.status", Some("done"), &msg, "hook");
                    client.post_event(payload).await;
                });
            }

            ObserverEvent::TurnComplete => {
                if !self.emit_task_signals {
                    return;
                }
                let client = Arc::clone(&self.client);
                self.runtime.spawn(async move {
                    let payload = client.base_payload(
                        "task.status",
                        Some("in_progress"),
                        "Turn completed",
                        "hook",
                    );
                    client.post_event(payload).await;
                });
            }

            ObserverEvent::Error { component, message } => {
                // Suppress noisy provider-level errors (rate limits, timeouts) to
                // avoid flooding the Mission Control activity feed.
                if component == "provider" {
                    return;
                }
                if !self.emit_task_signals {
                    return;
                }
                let client = Arc::clone(&self.client);
                let msg = format!("[{component}] {message}");
                self.runtime.spawn(async move {
                    let payload =
                        client.base_payload("error.runtime", None, &msg, "hook");
                    client.post_event(payload).await;
                });
            }

            // Not forwarded: too granular / not meaningful to the dashboard.
            ObserverEvent::LlmRequest { .. }
            | ObserverEvent::LlmResponse { .. }
            | ObserverEvent::ToolCallStart { .. }
            | ObserverEvent::ToolCall { .. }
            | ObserverEvent::ChannelMessage { .. } => {}
        }
    }

    fn record_metric(&self, _metric: &ObserverMetric) {
        // Mission Control does not consume raw metrics — signals carry all needed state.
    }

    fn name(&self) -> &str {
        "mission-control"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::mission_control_client::MissionControlClient;
    use std::time::Duration;

    // All tests use #[tokio::test] so Handle::current() works inside MissionControlObserver::new.

    fn make_observer() -> MissionControlObserver {
        MissionControlObserver::new(
            MissionControlClient::new(
                "http://127.0.0.1:19999",
                "test-token",
                "test-instance",
                "manager",
            ),
            true,
            true,
        )
    }

    #[tokio::test]
    async fn name_is_mission_control() {
        assert_eq!(make_observer().name(), "mission-control");
    }

    #[tokio::test]
    async fn records_all_events_without_panic() {
        let obs = make_observer();

        obs.record_event(&ObserverEvent::AgentStart {
            provider: "openrouter".into(),
            model: "claude-sonnet".into(),
        });
        obs.record_event(&ObserverEvent::AgentEnd {
            provider: "openrouter".into(),
            model: "claude-sonnet".into(),
            duration: Duration::from_millis(500),
            tokens_used: Some(100),
            cost_usd: Some(0.001),
        });
        obs.record_event(&ObserverEvent::TurnComplete);
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_event(&ObserverEvent::Error {
            component: "gateway".into(),
            message: "test error".into(),
        });
        // Provider errors must be suppressed.
        obs.record_event(&ObserverEvent::Error {
            component: "provider".into(),
            message: "rate limit".into(),
        });
        // These are no-ops.
        obs.record_event(&ObserverEvent::LlmRequest {
            provider: "openrouter".into(),
            model: "claude-sonnet".into(),
            messages_count: 2,
        });
        obs.record_event(&ObserverEvent::ToolCall {
            tool: "shell".into(),
            duration: Duration::from_millis(10),
            success: true,
        });
        obs.record_event(&ObserverEvent::ChannelMessage {
            channel: "telegram".into(),
            direction: "inbound".into(),
        });
    }

    #[tokio::test]
    async fn disabled_heartbeat_skips_spawn() {
        let obs = MissionControlObserver::new(
            MissionControlClient::new("http://127.0.0.1:19999", "", "", ""),
            true,
            false, // emit_heartbeat disabled
        );
        obs.record_event(&ObserverEvent::HeartbeatTick);
    }

    #[tokio::test]
    async fn disabled_task_signals_skips_spawn() {
        let obs = MissionControlObserver::new(
            MissionControlClient::new("http://127.0.0.1:19999", "", "", ""),
            false, // emit_task_signals disabled
            true,
        );
        obs.record_event(&ObserverEvent::AgentStart {
            provider: "openrouter".into(),
            model: "claude-sonnet".into(),
        });
        obs.record_event(&ObserverEvent::TurnComplete);
        obs.record_event(&ObserverEvent::Error {
            component: "gateway".into(),
            message: "boom".into(),
        });
    }

    #[tokio::test]
    async fn record_metric_is_noop() {
        let obs = make_observer();
        obs.record_metric(&ObserverMetric::TokensUsed(42));
        obs.record_metric(&ObserverMetric::ActiveSessions(1));
    }

    #[tokio::test]
    async fn as_any_downcasts() {
        assert!(make_observer()
            .as_any()
            .downcast_ref::<MissionControlObserver>()
            .is_some());
    }
}
