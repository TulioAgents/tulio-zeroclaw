//! Tool that lets an agent enqueue a task for another agent.
//!
//! Agents call this to hand off work to a specialist or to schedule
//! the next pipeline step without blocking on a synchronous response.
//!
//! The `project` and `task_id` fields are embedded into the `context`
//! column so callers can query the queue by project/task without tokens.

use super::traits::{Tool, ToolResult};
use crate::queue::{self, EnqueueRequest};
use crate::security::policy::ToolOperation;
use crate::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

pub struct QueueEnqueueTool {
    workspace_dir: PathBuf,
    security: Arc<SecurityPolicy>,
}

impl QueueEnqueueTool {
    pub fn new(workspace_dir: PathBuf, security: Arc<SecurityPolicy>) -> Self {
        Self {
            workspace_dir,
            security,
        }
    }
}

#[async_trait]
impl Tool for QueueEnqueueTool {
    fn name(&self) -> &str {
        "queue_enqueue"
    }

    fn description(&self) -> &str {
        "Enqueue a task for another agent to execute on its next drain cycle. \
         Use this to hand off work after completing your own step, or to \
         schedule the next stage of a delivery pipeline."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "Target agent name (must match a configured agent, e.g. 'guaripolo', 'qa', 'sr-fullstack')"
                },
                "project": {
                    "type": "string",
                    "description": "Project identifier (e.g. 'helloworld2'). Stored in context for CLI lookup."
                },
                "task_id": {
                    "type": "string",
                    "description": "Caller-assigned task identifier (e.g. 'hw2-003'). Stored in context for CLI lookup."
                },
                "task": {
                    "type": "string",
                    "description": "The task description the target agent should execute."
                },
                "context": {
                    "type": "string",
                    "description": "Optional additional context or handoff notes for the target agent."
                },
                "priority": {
                    "type": "integer",
                    "description": "Priority (lower number = higher priority). Default 0."
                }
            },
            "required": ["agent", "task"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        if let Err(e) = self
            .security
            .enforce_tool_operation(ToolOperation::Act, "queue_enqueue")
        {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(e.to_string()),
            });
        }

        let agent = match args.get("agent").and_then(|v| v.as_str()) {
            Some(a) if !a.trim().is_empty() => a.trim().to_string(),
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing required field: 'agent'".into()),
                })
            }
        };

        let task = match args.get("task").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.to_string(),
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing required field: 'task'".into()),
                })
            }
        };

        let project = args
            .get("project")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let task_id = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let extra_context = args
            .get("context")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let priority = args
            .get("priority")
            .and_then(|v| v.as_i64())
            .map(|n| n as i32);

        // Build context: project + task_id header, then caller context
        let context = {
            let mut parts = String::new();
            if let Some(p) = &project {
                parts.push_str(&format!("project: {p}\n"));
            }
            if let Some(t) = &task_id {
                parts.push_str(&format!("task_id: {t}\n"));
            }
            if let Some(c) = &extra_context {
                if !parts.is_empty() {
                    parts.push('\n');
                }
                parts.push_str(c);
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts)
            }
        };

        let req = EnqueueRequest {
            task,
            context,
            from_agent: None,
            priority,
        };

        match queue::enqueue(&self.workspace_dir, &agent, req) {
            Ok(item) => Ok(ToolResult {
                success: true,
                output: format!(
                    "Enqueued task for agent '{}' (id: {}{}{})",
                    item.agent,
                    item.id,
                    project
                        .as_deref()
                        .map(|p| format!(", project: {p}"))
                        .unwrap_or_default(),
                    task_id
                        .as_deref()
                        .map(|t| format!(", task_id: {t}"))
                        .unwrap_or_default(),
                ),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to enqueue task: {e}")),
            }),
        }
    }
}
