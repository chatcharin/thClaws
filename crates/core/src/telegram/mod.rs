//! Telegram bridge — thClaws-side Telegram bot using **long polling**
//! (no webhook required).
//!
//! Architecture:
//! 1. User enters their bot token in the GUI Telegram Connect modal
//!    or via `--telegram <token>` CLI flag.
//! 2. thClaws polls `getUpdates` directly to Telegram Bot API.
//! 3. Incoming messages are dispatched via `MessageHandler` trait.

pub mod config;
pub mod protocol;
pub mod client;
pub mod polling;
pub mod handler;
pub mod approver;

pub use approver::TelegramApprover;

use config::TelegramConfig;
use client::TelegramClient;
use polling::TelegramPollingLoop;
use handler::MessageHandler;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::path::PathBuf;
use tokio::sync::Mutex;

/// Main entry point for the Telegram bridge.
pub struct TelegramBridge {
    config_path: PathBuf,
    client: Arc<Mutex<TelegramClient>>,
    polling_loop: Option<Arc<TelegramPollingLoop>>,
    running: Arc<AtomicBool>,
}

impl TelegramBridge {
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            config_path,
            client: Arc::new(Mutex::new(TelegramClient::new())),
            polling_loop: None,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Load config from disk.
    pub fn load_config(&self) -> Result<TelegramConfig, String> {
        TelegramConfig::load(&self.config_path)
    }

    /// Save bot token to disk.
    pub async fn save_config(&self, bot_token: &str) -> Result<TelegramConfig, String> {
        let config = TelegramConfig::new(bot_token.to_string());
        config.save(&self.config_path)?;
        Ok(config)
    }

    /// Delete config from disk.
    pub fn delete_config(&self) -> Result<(), String> {
        TelegramConfig::delete(&self.config_path)
    }

    /// Check if Telegram is configured and running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Validate bot token via getMe API call.
    pub async fn validate_token(&self, token: &str) -> Result<String, String> {
        let mut client = self.client.lock().await;
        client.set_token(token);
        let me = client.get_me().await?;
        Ok(format!("{} (@{})", me.first_name, me.username))
    }

    /// Start the polling loop with a message handler.
    pub async fn start<H>(&self, handler: H) -> Result<(), String>
    where
        H: MessageHandler + Send + Sync + 'static,
    {
        let config = TelegramConfig::load(&self.config_path)?;
        if config.bot_token.is_empty() {
            return Err("No bot token configured. Please pair via GUI or --telegram flag.".to_string());
        }

        let running = self.running.clone();
        running.store(true, Ordering::SeqCst);

        let client = self.client.clone();
        let loop_handle = Arc::new(TelegramPollingLoop::new(
            config.bot_token.clone(),
            handler,
            client,
        ));

        let loop_handle_clone = loop_handle.clone();
        tokio::spawn(async move {
            loop_handle_clone.run().await;
        });

        // Store in self via interior mutability
        // For simplicity, just run it without storing
        Ok(())
    }

    /// Stop the polling loop.
    pub async fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(loop_handle) = &self.polling_loop {
            loop_handle.stop();
        }
    }

    /// Register bot commands with Telegram so they appear in the
    /// command menu when users type `/`. Called once on connect.
    pub async fn register_commands(&self) -> Result<(), String> {
        let client = self.client.lock().await;
        let commands = vec![
            crate::telegram::protocol::BotCommand {
                command: "help".to_string(),
                description: "Show available thClaws commands".to_string(),
            },
            crate::telegram::protocol::BotCommand {
                command: "model".to_string(),
                description: "Change AI model".to_string(),
            },
            crate::telegram::protocol::BotCommand {
                command: "provider".to_string(),
                description: "Change AI provider".to_string(),
            },
            crate::telegram::protocol::BotCommand {
                command: "skill".to_string(),
                description: "Install or list skills".to_string(),
            },
            crate::telegram::protocol::BotCommand {
                command: "mcp".to_string(),
                description: "Manage MCP servers".to_string(),
            },
            crate::telegram::protocol::BotCommand {
                command: "telegram".to_string(),
                description: "Telegram bridge commands (connect/disconnect/status)".to_string(),
            },
            crate::telegram::protocol::BotCommand {
                command: "plan".to_string(),
                description: "Enter plan mode".to_string(),
            },
            crate::telegram::protocol::BotCommand {
                command: "goal".to_string(),
                description: "Start goal-directed loop".to_string(),
            },
            crate::telegram::protocol::BotCommand {
                command: "loop".to_string(),
                description: "Start iteration loop".to_string(),
            },
            crate::telegram::protocol::BotCommand {
                command: "kms".to_string(),
                description: "Knowledge base commands".to_string(),
            },
        ];
        client.set_my_commands(commands).await?;
        Ok(())
    }
}
