//! Long polling loop for the Telegram bot.
//!
//! Implements the core polling mechanism: continuously call `getUpdates`
//! and dispatch incoming messages to the message handler.
//!
//! Based on Hermes Agent's `gateway/platforms/telegram.py` long polling approach.

use crate::telegram::protocol::*;
use crate::telegram::client::TelegramClient;
use crate::telegram::handler::MessageHandler;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{sleep, Duration};
use tokio::sync::Mutex;

/// Long polling loop handle.
pub struct TelegramPollingLoop {
    handler: Box<dyn MessageHandler + Send + Sync>,
    client: Arc<Mutex<TelegramClient>>,
    running: Arc<AtomicBool>,
}

impl TelegramPollingLoop {
    /// Create a new polling loop.
    pub fn new<H>(_bot_token: String, handler: H, client: Arc<tokio::sync::Mutex<TelegramClient>>) -> Self
    where
        H: MessageHandler + Send + Sync + 'static,
    {
        Self {
            handler: Box::new(handler),
            client,
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Stop the polling loop.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Run the polling loop. This is the main event loop.
    pub async fn run(&self) {
        eprintln!("[Telegram] Starting long polling loop...");
        let mut offset: i64 = 0;

        while self.running.load(Ordering::SeqCst) {
            match self.poll_updates(offset).await {
                Ok((updates, new_offset)) => {
                    offset = new_offset;
                    for update in updates {
                        if let Some(ref msg) = update.message {
                            self.dispatch_message(&msg).await;
                        } else if let Some(ref cb) = update.callback_query {
                            self.dispatch_callback(cb).await;
                        } else if let Some(ref msg) = update.edited_message {
                            self.dispatch_edit(&msg).await;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[Telegram] Poll error: {}", e);
                    // Wait before retrying on error
                    sleep(Duration::from_secs(5)).await;
                }
            }

            // Small delay between polling cycles to prevent tight loops
            sleep(Duration::from_millis(500)).await;
        }

        eprintln!("[Telegram] Polling loop stopped.");
    }

    /// Poll for new updates using getUpdates.
    async fn poll_updates(&self, offset: i64) -> Result<(Vec<Update>, i64), String> {
        let client = self.client.lock().await;
        let updates = client.get_updates(10, 1, Some(offset + 1)).await?;
        drop(client);

        if updates.is_empty() {
            return Ok((Vec::new(), offset));
        }

        // Calculate next offset (last update_id + 1)
        let mut max_offset = offset;
        for update in &updates {
            if update.update_id > max_offset {
                max_offset = update.update_id;
            }
        }

        Ok((updates, max_offset))
    }

    /// Dispatch an incoming message to the handler.
    async fn dispatch_message(&self, msg: &Message) {
        let chat_id = msg.chat.id;
        let from = msg.from.clone();
        let text = msg.text.clone().unwrap_or_default();
        
        // Authorization check: reject messages from unauthorized users
        let config_path = crate::telegram::config::TelegramConfig::default_path();
        if let Ok(cfg) = crate::telegram::config::TelegramConfig::load(&config_path) {
            if !cfg.allowed_chat_ids.is_empty() && !cfg.allowed_chat_ids.contains(&chat_id) {
                eprintln!("[Telegram] Unauthorized chat_id {} blocked", chat_id);
                // Send rejection message
                let mut client = crate::telegram::client::TelegramClient::new();
                client.set_token(&cfg.bot_token);
                let request = crate::telegram::protocol::SendMessageRequest {
                    chat_id,
                    text: "⚠️ You are not authorized to use this bot.".to_string(),
                    parse_mode: None,
                    reply_markup: None,
                };
                let _ = client.send_message(request).await;
                return;
            }
        }
        
        // Auto-register first user if no allowlist configured
        let config_path = crate::telegram::config::TelegramConfig::default_path();
        if let Ok(mut cfg) = crate::telegram::config::TelegramConfig::load(&config_path) {
            if cfg.allowed_chat_ids.is_empty() {
                cfg.allowed_chat_ids.push(chat_id);
                let _ = cfg.save(&config_path);
                eprintln!("[Telegram] Auto-registered chat_id {}", chat_id);
            }
        }
        
        let username = from.as_ref().map(|u| if u.username.is_empty() { "unknown" } else { &u.username }).unwrap_or("unknown");
        eprintln!("[Telegram] Message from @{} (chat {})", username, chat_id);

        // Handle inline keyboard callbacks embedded in text
        if let Some(ref markup) = msg.reply_markup {
            for row in &markup.inline_keyboard {
                for btn in row {
                    if let Some(ref data) = btn.callback_data {
                        self.handler.on_callback_query(chat_id, data.clone(), from.clone()).await;
                    }
                }
            }
        }

        // Handle normal text messages
        self.handler.on_message(chat_id, text, from).await;
    }

    /// Dispatch an edited message.
    async fn dispatch_edit(&self, msg: &Message) {
        eprintln!("[Telegram] Edited message in chat {}", msg.chat.id);
        let text = msg.text.clone().unwrap_or_default();
        let chat_id = msg.chat.id;
        let from = msg.from.clone();
        self.handler.on_message_edited(chat_id, text, from).await;
    }

    /// Dispatch a callback query from inline button press.
    async fn dispatch_callback(&self, cb: &CallbackQuery) {
        eprintln!("[Telegram] Callback query: {}", cb.data);
        self.handler.on_callback_query(cb.from.id, cb.data.clone(), Some(cb.from.clone())).await;
    }
}
