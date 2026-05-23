//! `TelegramApprover` — implements `ApprovalSink` by routing the
//! prompt through Telegram inline keyboard buttons.
//!
//! Plan-07 Phase 3. When `PermissionMode::TelegramGated` is active,
//! the agent's permission gate calls `TelegramApprover::approve(req)`
//! which:
//! 1. Registers a fresh `request_id` against a `oneshot::Sender`.
//! 2. Sends an approval prompt with inline keyboard buttons
//!    ([✅ Approve] [🚫 Deny] [⏸ Deny Session]) back to the
//!    Telegram chat.
//! 3. Awaits the user's callback query response.
//! 4. On timeout (default 60 s) auto-denies + sends a follow-up
//!    notice so the user isn't left wondering.
//!
//! Callback queries are routed through
//! `TelegramPollingLoop::dispatch_callback` which calls
//! `record_decision_from_callback` to resolve the pending approval.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::oneshot;

use crate::permissions::{ApprovalDecision, ApprovalRequest, ApprovalSink};

/// Cap on how long we'll wait for the user to respond before
/// auto-denying. 60 s matches LINE and plan-07 defaults.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// First N chars of a tool's input preview rendered in Telegram.
const INPUT_PREVIEW_CHARS: usize = 200;

/// What the user tapped in response to an approval prompt.
/// Callback `data` strings shape as `tool:<verb>:<req_id>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalReply {
    Allow,
    AllowForSession,
    Deny,
    DenyForSession,
    /// User tapped something we can't classify.
    Unrecognised,
}

impl ApprovalReply {
    /// Parse a callback `data` field. Shape is
    /// `tool:<verb>:<request_id>` for consistency with LINE.
    pub fn parse_callback_data(data: &str) -> (Self, Option<String>) {
        let parts: Vec<&str> = data.split(':').collect();
        let (verb, req_id) = match parts.as_slice() {
            ["tool", verb, req] => (*verb, Some((*req).to_string())),
            [verb, req] => (*verb, Some((*req).to_string())),
            [verb] => (*verb, None),
            _ => return (Self::Unrecognised, None),
        };
        let decision = match verb.to_lowercase().as_str() {
            "allow" | "approve" | "yes" => Self::Allow,
            "allow_session" | "allow session" => Self::AllowForSession,
            "deny" | "reject" | "no" => Self::Deny,
            "session" => Self::DenyForSession,
            _ => Self::Unrecognised,
        };
        (decision, req_id)
    }
}

#[derive(Default)]
struct Pending {
    /// Inflight approval requests keyed by `request_id`.
    map: HashMap<String, oneshot::Sender<ApprovalDecision>>,
    /// Insertion order so we can iterate deterministically if needed.
    order: Vec<String>,
}

impl Pending {
    fn has_any(&self) -> bool {
        !self.map.is_empty()
    }

    fn insert(&mut self, id: String, tx: oneshot::Sender<ApprovalDecision>) {
        self.map.insert(id.clone(), tx);
        self.order.push(id);
    }

    fn take_by_id(&mut self, id: &str) -> Option<oneshot::Sender<ApprovalDecision>> {
        self.map.remove(id).map(|tx| {
            self.order.retain(|oid| oid != id);
            tx
        })
    }
}

/// Routes tool-approval prompts through Telegram inline keyboard buttons.
#[derive(Clone)]
pub struct TelegramApprover {
    /// Telegram HTTP client for sending messages with inline keyboards.
    client: Arc<tokio::sync::Mutex<Option<crate::telegram::client::TelegramClient>>>,
    /// Pending approval requests awaiting user response.
    pending: Arc<Mutex<Pending>>,
    /// Timeout before auto-denying.
    timeout: Duration,
}

impl TelegramApprover {
    /// Create a new approver. The client is provided as an `Option`
    /// because the polling loop may not have initialised yet.
    pub fn new(client: Option<crate::telegram::client::TelegramClient>) -> Self {
        Self {
            client: Arc::new(tokio::sync::Mutex::new(client)),
            pending: Arc::new(Mutex::new(Pending::default())),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, dur: Duration) -> Self {
        self.timeout = dur;
        self
    }

    /// True when at least one approval is waiting for a reply.
    pub fn has_pending(&self) -> bool {
        self.pending.lock().map(|p| p.has_any()).unwrap_or(false)
    }

    /// Resolve the pending approval whose `request_id` matches.
    /// Returns `Some(decision)` when resolved, `None` when unknown.
    pub fn record_decision_from_callback(
        &self,
        callback_data: &str,
    ) -> Option<ApprovalDecision> {
        let (reply, req_id) = ApprovalReply::parse_callback_data(callback_data);
        let req_id = req_id?;
        
        let decision = match reply {
            ApprovalReply::Allow => ApprovalDecision::Allow,
            ApprovalReply::AllowForSession => ApprovalDecision::AllowForSession,
            ApprovalReply::Deny => ApprovalDecision::Deny,
            ApprovalReply::DenyForSession => ApprovalDecision::Deny,
            ApprovalReply::Unrecognised => return None,
        };

        let tx = self
            .pending
            .lock()
            .ok()
            .and_then(|mut p| p.take_by_id(&req_id))?;
        
        if tx.send(decision).is_ok() {
            Some(decision)
        } else {
            None
        }
    }

    /// Build a human-readable prompt for a tool approval request.
    fn build_prompt(req: &ApprovalRequest) -> String {
        let input_str = req.input.to_string();
        let preview: String = input_str.chars().take(INPUT_PREVIEW_CHARS).collect();
        
        let mut prompt = format!(
            "🔧 **Tool Approval Required**\n\n"
        );
        prompt.push_str(&format!("**Tool:** `{}`\n", req.tool_name));
        
        if !preview.is_empty() {
            prompt.push_str(&format!("**Input:** `{}`\n\n", preview));
        }
        
        prompt.push_str("Tap a button below to approve or deny.");
        prompt
    }

    /// Build inline keyboard buttons for approval.
    fn build_buttons(request_id: &str) -> serde_json::Value {
        serde_json::json!({
            "inline_keyboard": [
                [
                    {
                        "text": "✅ Approve",
                        "callback_data": format!("tool:allow:{}", request_id)
                    },
                    {
                        "text": "✅✅ Allow Session",
                        "callback_data": format!("tool:allow_session:{}", request_id)
                    }
                ],
                [
                    {
                        "text": "🚫 Deny",
                        "callback_data": format!("tool:deny:{}", request_id)
                    }
                ],
                [
                    {
                        "text": "⏸ Deny for Session",
                        "callback_data": format!("tool:session:{}", request_id)
                    }
                ]
            ]
        })
    }

    /// Update the client reference (called when Telegram connects).
    pub async fn update_client(&self, client: crate::telegram::client::TelegramClient) {
        let mut guard = self.client.lock().await;
        *guard = Some(client);
    }
}

#[async_trait]
impl ApprovalSink for TelegramApprover {
    async fn approve(&self, req: &ApprovalRequest) -> ApprovalDecision {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(request_id.clone(), tx);
        }

        // Send approval prompt with inline keyboard
        let client_guard = self.client.lock().await;
        if let Some(client) = client_guard.as_ref() {
            let prompt = Self::build_prompt(req);
            let buttons = Self::build_buttons(&request_id);

            // We need chat_id to send the approval prompt.
            // For now, we'll send to the first authorized chat_id.
            // TODO: Track the active Telegram chat_id in state.
            let config_path = crate::telegram::config::TelegramConfig::default_path();
            if let Ok(cfg) = crate::telegram::config::TelegramConfig::load(&config_path) {
                if let Some(&chat_id) = cfg.allowed_chat_ids.first() {
                    let request = crate::telegram::protocol::SendMessageRequest {
                        chat_id,
                        text: prompt,
                        parse_mode: Some("Markdown".to_string()),
                        reply_markup: Some(buttons),
                    };

                    if let Err(e) = client.send_message(request).await {
                        eprintln!("[telegram] approval prompt failed to send: {e}; auto-denying");
                        self.pending
                            .lock()
                            .ok()
                            .and_then(|mut p| p.take_by_id(&request_id));
                        return ApprovalDecision::Deny;
                    }
                } else {
                    eprintln!("[telegram] no authorized chat_ids; auto-denying approval");
                    return ApprovalDecision::Deny;
                }
            } else {
                eprintln!("[telegram] config load failed; auto-denying approval");
                return ApprovalDecision::Deny;
            }
        } else {
            eprintln!("[telegram] no client available; auto-denying approval");
            return ApprovalDecision::Deny;
        }
        drop(client_guard);

        // Wait for user response or timeout
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(_canceled)) => {
                // Sender dropped without sending. Treat as deny.
                ApprovalDecision::Deny
            }
            Err(_elapsed) => {
                eprintln!(
                    "[telegram] approval for {} timed out after {:?}; auto-denying",
                    req.tool_name, self.timeout
                );
                // Drop the pending entry so a late reply doesn't
                // resurrect an already-denied decision.
                if let Ok(mut pending) = self.pending.lock() {
                    let _ = pending.take_by_id(&request_id);
                }
                // Send timeout notice
                let client_guard = self.client.lock().await;
                if let Some(client) = client_guard.as_ref() {
                    let config_path = crate::telegram::config::TelegramConfig::default_path();
                    if let Ok(cfg) = crate::telegram::config::TelegramConfig::load(&config_path) {
                        if let Some(&chat_id) = cfg.allowed_chat_ids.first() {
                            let notice = format!(
                                "⏰ Approval for `{}` timed out; auto-denied.",
                                req.tool_name
                            );
                            let request = crate::telegram::protocol::SendMessageRequest {
                                chat_id,
                                text: notice,
                                parse_mode: Some("Markdown".to_string()),
                                reply_markup: None,
                            };
                            let _ = client.send_message(request).await;
                        }
                    }
                }
                ApprovalDecision::Deny
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_callback_data_allows() {
        let (reply, req_id) = ApprovalReply::parse_callback_data("tool:allow:abc123");
        assert_eq!(reply, ApprovalReply::Allow);
        assert_eq!(req_id, Some("abc123".to_string()));
    }

    #[test]
    fn parse_callback_data_denies() {
        let (reply, req_id) = ApprovalReply::parse_callback_data("tool:deny:def456");
        assert_eq!(reply, ApprovalReply::Deny);
        assert_eq!(req_id, Some("def456".to_string()));
    }

    #[test]
    fn parse_callback_data_deny_session() {
        let (reply, req_id) = ApprovalReply::parse_callback_data("tool:session:ghi789");
        assert_eq!(reply, ApprovalReply::DenyForSession);
        assert_eq!(req_id, Some("ghi789".to_string()));
    }

    #[test]
    fn parse_callback_data_unrecognised() {
        let (reply, req_id) = ApprovalReply::parse_callback_data("garbage");
        assert_eq!(reply, ApprovalReply::Unrecognised);
        assert_eq!(req_id, None);
    }

    #[test]
    fn pending_has_any_tracks_correctly() {
        let approver = TelegramApprover::new(None);
        assert!(!approver.has_pending());

        let (_tx, rx) = oneshot::channel();
        approver.pending.lock().unwrap().insert("test".to_string(), _tx);
        assert!(approver.has_pending());

        drop(rx);
    }
}
