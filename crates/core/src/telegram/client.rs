//! Telegram Bot API client — wraps `reqwest` calls to the Telegram Bot API.
//!
//! Provides methods for:
//! - `getMe` — verify bot token and get bot info
//! - `sendMessage` — send text messages
//! - `sendDocument` — send files/documents
//! - `answerCallbackQuery` — respond to inline button presses
//! - `getUpdates` — fetch new updates (long polling)

use crate::telegram::protocol::*;
use reqwest::Client;
use serde_json;

/// Telegram Bot API client.
pub struct TelegramClient {
    base_url: String,
    client: Client,
}

impl TelegramClient {
    /// Create a new client (token set via `set_token`).
    pub fn new() -> Self {
        Self {
            base_url: String::new(),
            client: Client::new(),
        }
    }

    /// Set the bot token.
    pub fn set_token(&mut self, token: &str) {
        self.base_url = format!("https://api.telegram.org/bot{}", token.trim());
    }

    /// Get the current bot token (for config save).
    pub fn bot_token(&self) -> Option<String> {
        if self.base_url.is_empty() {
            None
        } else {
            Some(self.base_url.strip_prefix("https://api.telegram.org/bot")
                .unwrap_or("").to_string())
        }
    }

    /// Call `getMe` to verify the bot token.
    pub async fn get_me(&self) -> Result<User, String> {
        let url = format!("{}/getMe", self.base_url);
        let resp = self.client.get(&url).send().await
            .map_err(|e| format!("getMe request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("getMe failed: {} - {}", status, body));
        }

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("getMe parse error: {}", e))?;

        let result = data.get("result")
            .ok_or("getMe: no result field")?;

        serde_json::from_value(result.clone())
            .map_err(|e| format!("getMe deserialize error: {}", e))
    }

    /// Call `sendMessage`.
    pub async fn send_message(&self, request: SendMessageRequest) -> Result<Message, String> {
        let url = format!("{}/sendMessage", self.base_url);
        let resp = self.client.post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("sendMessage request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("sendMessage failed: {} - {}", status, body));
        }

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("sendMessage parse error: {}", e))?;

        let result = data.get("result")
            .ok_or("sendMessage: no result field")?;

        serde_json::from_value(result.clone())
            .map_err(|e| format!("sendMessage deserialize error: {}", e))
    }

    /// Call `getUpdates` with long polling.
    /// 
    /// - `timeout`: max seconds to wait (default 10)
    /// - `limit`: max updates to return (default 1)
    /// - `offset`: process updates after this ID (incremented by caller)
    pub async fn get_updates(
        &self,
        timeout: i32,
        limit: i32,
        offset: Option<i64>,
    ) -> Result<Vec<Update>, String> {
        let mut builder = self.client.get(format!("{}/getUpdates", self.base_url))
            .query(&[("timeout", &timeout.to_string()), ("limit", &limit.to_string())]);

        if let Some(off) = offset {
            builder = builder.query(&[("offset", &off.to_string())]);
        }

        let resp = builder.send().await
            .map_err(|e| format!("getUpdates request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("getUpdates failed: {} - {}", status, body));
        }

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("getUpdates parse error: {}", e))?;

        let updates: Vec<Update> = data.get("result")
            .and_then(|v| v.as_array())
            .map(|v| v.iter().filter_map(|u| serde_json::from_value(u.clone()).ok()).collect())
            .unwrap_or_default();

        Ok(updates)
    }

    /// Call `answerCallbackQuery`.
    pub async fn answer_callback_query(
        &self,
        request: AnswerCallbackQueryRequest,
    ) -> Result<bool, String> {
        let url = format!("{}/answerCallbackQuery", self.base_url);
        let resp = self.client.post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("answerCallbackQuery request failed: {}", e))?;

        Ok(resp.status().is_success())
    }
}
