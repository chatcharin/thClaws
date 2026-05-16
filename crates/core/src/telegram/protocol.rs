//! Telegram Bot API wire protocol types.
//!
//! This module defines the JSON structures exchanged with the
//! Telegram Bot API (`https://api.telegram.org/bot<TOKEN>/...`).
//! Only the types actually used by the bridge are included.

use serde::{Deserialize, Serialize};

/// Response from `getMe` — used to validate the bot token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_code: Option<String>,
}
/// A single update from `getUpdates`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Update {
    pub update_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited_message: Option<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_post: Option<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_query: Option<CallbackQuery>,
}

/// A message from a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub from: Option<User>,
    pub chat: Chat,
    pub date: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities: Option<Vec<MessageEntity>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo: Option<Vec<PhotoSize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<Document>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_markup: Option<InlineKeyboardMarkup>,
}

/// Chat information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chat {
    pub id: i64,
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// Message entity (link, mention, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEntity {
    pub r#type: String,
    pub offset: i32,
    pub length: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<User>,
}

/// Photo size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhotoSize {
    pub file_id: String,
    pub width: i32,
    pub height: i32,
}

/// Document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub file_id: String,
    pub file_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
}

/// Inline keyboard markup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineKeyboardMarkup {
    pub inline_keyboard: Vec<Vec<InlineKeyboardButton>>,
}

/// A button in an inline keyboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineKeyboardButton {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Callback query from inline button press.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub from: User,
    pub message: Option<Message>,
    pub inline_message_id: Option<String>,
    pub data: String,
}

/// Request payload for `sendMessage`.
#[derive(Debug, Clone, Serialize)]
pub struct SendMessageRequest {
    pub chat_id: i64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_markup: Option<serde_json::Value>,
}

/// Bot command definition for setMyCommands.
#[derive(Debug, Clone, Serialize)]
pub struct BotCommand {
    pub command: String,
    pub description: String,
}

/// Request payload for setMyCommands.
#[derive(Debug, Clone, Serialize)]
pub struct SetMyCommandsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<BotCommand>>,
}

/// Request payload for sending a document.
#[derive(Debug, Clone, Serialize)]
pub struct SendDocumentRequest {
    pub chat_id: i64,
    #[serde(skip_serializing)]
    pub document: String, // file_id or path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

/// Request payload for `answerCallbackQuery`.
#[derive(Debug, Clone, Serialize)]
pub struct AnswerCallbackQueryRequest {
    pub callback_query_id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_alert: Option<bool>,
}
