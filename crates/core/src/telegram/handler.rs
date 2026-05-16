//! Message handler trait for dispatching Telegram events to the agent.
//!
//! Implement this trait to handle incoming messages, photos, documents,
//! and callback queries.

use crate::telegram::protocol::User;

/// Trait for handling Telegram events.
/// 
/// Implement this to connect Telegram messages to your agent logic.
pub trait MessageHandler: Send + Sync {
    /// Handle an incoming text message.
    fn on_message(&self, chat_id: i64, text: String, from: Option<User>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>;

    /// Handle an edited message.
    fn on_message_edited(&self, _chat_id: i64, _text: String, _from: Option<User>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }

    /// Handle an inline button callback query.
    fn on_callback_query(&self, _chat_id: i64, _data: String, _from: Option<User>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }

    /// Handle an incoming photo (download and process).
    fn on_photo(&self, _chat_id: i64, _photo_urls: Vec<String>, _caption: Option<String>, _from: Option<User>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }

    /// Handle an incoming document (download and process).
    fn on_document(&self, _chat_id: i64, _file_name: String, _mime_type: Option<String>, _from: Option<User>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }
}

/// Format a user for display.
pub fn format_user(user: &User) -> String {
    if !user.username.is_empty() {
        format!("@{}", user.username)
    } else if let Some(ref last_name) = user.last_name {
        format!("{} {}", user.first_name, last_name)
    } else {
        user.first_name.clone()
    }
}
