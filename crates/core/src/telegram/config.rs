//! On-disk Telegram bot config at `~/.config/thclaws/telegram.json`.
//!
//! Written when the user pairs their bot token via the GUI
//! Telegram Connect modal or the `--telegram <token>` CLI flag.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Telegram bot configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    /// Bot token from @BotFather.
    pub bot_token: String,
    /// When the config was last updated.
    pub updated_at: String,
    /// Chat IDs that are allowed to interact with this bot.
    /// If empty, all users are allowed (not recommended for production).
    #[serde(default)]
    pub allowed_chat_ids: Vec<i64>,
}

impl TelegramConfig {
    /// Create a new config with the given bot token.
    pub fn new(bot_token: String) -> Self {
        Self {
            bot_token,
            updated_at: chrono::Utc::now().to_rfc3339(),
            allowed_chat_ids: Vec::new(),
        }
    }

    /// Load config from disk.
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Err("Config not found. Please pair via GUI or --telegram flag.".to_string());
        }
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config: {}", e))?;
        let config: TelegramConfig = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse config: {}", e))?;
        Ok(config)
    }

    /// Save config to disk (atomic write with secure permissions).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        
        // Atomic write: write to temp file then rename (prevents corruption on crash)
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, &content)
            .map_err(|e| format!("Failed to write config: {}", e))?;
        
        // Set secure permissions (owner read/write only) on Unix systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600)) {
                eprintln!("[Telegram] Warning: failed to set file permissions: {}", e);
            }
        }
        
        fs::rename(&tmp_path, path)
            .map_err(|e| format!("Failed to rename config file: {}", e))?;
        
        Ok(())
    }

    /// Delete config from disk.
    pub fn delete(path: &Path) -> Result<(), String> {
        if path.exists() {
            fs::remove_file(path)
                .map_err(|e| format!("Failed to delete config: {}", e))?;
        }
        Ok(())
    }

    /// Return the default config path (~/.config/thclaws/telegram.json).
    pub fn default_path() -> std::path::PathBuf {
        if let Some(config_dir) = dirs::config_dir() {
            config_dir.join("thclaws").join("telegram.json")
        } else {
            std::env::home_dir()
                .map(|h| h.join(".config/thclaws/telegram.json"))
                .unwrap_or_else(|| std::path::PathBuf::from("telegram.json"))
        }
    }
}
