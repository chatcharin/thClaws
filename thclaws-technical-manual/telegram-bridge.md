# Telegram Bridge

**Plan-07 Phase 3** — thClaws Telegram bot integration for remote chat access.

## Architecture

thClaws runs a **Telegram Bot** using the [Telegram Bot API](https://core.telegram.org/bots/api) with **long polling** (no webhooks required). Users interact with their thClaws workspace from anywhere via Telegram.

```
┌─────────────┐         ┌──────────────────────┐         ┌──────────────┐
│   Telegram  │  HTTP   │  thClaws             │  Agent  │  Provider    │
│   Client    │◄───────►│  Telegram Bridge     │◄───────►│  (OpenAI,    │
│   (mobile)  │  poll   │  (long polling loop) │  turn   │  Anthropic,  │
└─────────────┘  reply  └──────────────────────┘         │  etc.)       │
                                                          └──────────────┘
```

### Key Differences from LINE Bridge

| Feature | LINE Bridge | Telegram Bridge |
|---------|-------------|-----------------|
| **Transport** | WebSocket relay server | Direct Bot API (long polling) |
| **Auth** | JWT pairing via relay | Bot token + chat ID allowlist |
| **Server** | Requires `thclaws-line-server` | No server needed |
| **Setup** | QR scan + relay pairing | BotFather token + config |
| **Approval routing** | Quick Reply chips | Not yet implemented (uses GUI) |

## Configuration

### Bot Token

Telegram config is stored at `~/.config/thclaws/telegram.json`:

```json
{
  "bot_token": "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11",
  "allowed_chat_ids": [8665969361],
  "updated_at": "2026-05-16T12:34:56.789Z"
}
```

**Security:**
- File permissions: `0o600` (owner-only read/write)
- Atomic write pattern (write to `.tmp` then rename)
- Bot token never exposed via getters (can't leak to logs)

### Authorization

**`allowed_chat_ids`** is an allowlist of Telegram chat IDs that can interact with the bot:

1. **First user auto-registration**: The first chat to message the bot is automatically added to `allowed_chat_ids`
2. **Subsequent users**: Must be manually added by editing `telegram.json`
3. **Unauthorized users**: Receive "⚠️ You are not authorized to use this bot."

## Getting a Bot Token

1. Open Telegram and search for **@BotFather**
2. Send `/newbot` and follow instructions
3. BotFather returns a token like: `123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11`
4. Connect in thClaws:
   ```
   /telegram connect 123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11
   ```

## Auto-Start

On worker boot, thClaws checks for `~/.config/thclaws/telegram.json`. If it exists, the Telegram bridge automatically starts with saved credentials.

## Slash Commands

### Bot Commands (autocomplete)

When you type `/` in Telegram, you'll see autocomplete for:

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/status` | Show session status |
| `/clear` | Clear conversation |
| `/permissions` | Show permission mode |
| `/sessions` | List sessions |
| `/models` | Show available models |

These are registered via Telegram's `setMyCommands` API on connect.

### /telegram Subcommands

Manage the Telegram bridge itself:

```
/telegram connect <bot_token>  # Connect to a new bot
/telegram disconnect           # Stop polling and delete config
/telegram status               # Show bridge status
```

## Permission Mode Integration

### TelegramGated Mode

When the Telegram bridge connects, thClaws **auto-switches** to `PermissionMode::TelegramGated`:

- **Before connect**: User's current mode (Auto / Ask / Plan) is stashed
- **While connected**: Mode switches to `TelegramGated`
- **On disconnect**: Previous mode is restored

```rust
// shared_session.rs - TelegramConnect handler
if state.telegram_pre_mode.is_none() {
    state.telegram_pre_mode = Some(state.agent.permission_mode);
}
crate::permissions::set_current_mode_and_broadcast(
    crate::permissions::PermissionMode::TelegramGated,
);
state.agent.permission_mode = PermissionMode::TelegramGated;
```

### AskUserQuestion Short-Circuit

When a turn is **Telegram-driven** (triggered by a Telegram message), the `AskUserQuestion` tool short-circuits to prevent hanging:

```rust
// tools/ask.rs
if is_telegram_driven_turn() {
    return Ok(format!(
        "(user is on Telegram — please rephrase \"{question}\" as part of your reply text; \
         their next Telegram message will be the answer)"
    ));
}
```

**Why?** The GUI modal for `AskUserQuestion` wouldn't be visible to a remote Telegram user. Short-circuiting teaches the model to ask in its reply text instead.

### Tool Approvals

**Current behavior (Phase 3):** Tool approvals (Bash, Write, Edit) route to the **local GUI/REPL** approver, not Telegram.

**Future enhancement:** Route approvals to Telegram via inline keyboard buttons:
```
[✅ Approve] [🚫 Deny] [⏸ Deny for Session]
```

This requires implementing a `TelegramApprover` struct (similar to `LineApprover`).

## Message Handling

### Long Message Chunking

Telegram API has a **4096 character limit** per message. thClaws automatically splits long responses:

```rust
const TELEGRAM_MAX_LENGTH: usize = 4000; // Leave buffer for safety

if final_text.len() <= TELEGRAM_MAX_LENGTH {
    // Send once
} else {
    // Split at word boundaries
    // Add numbering: (1/2), (2/2)
    // 100ms delay between chunks
}
```

### Event Collection

Telegram turns collect these `ViewEvent` variants:
- `AssistantTextDelta` — streaming response text
- `AssistantThinkingDelta` — thinking process (fallback if no text)
- `SlashOutput` — slash command output (e.g., `/help`)
- `TurnDone` — signals end of turn

## Code Structure

```
crates/core/src/telegram/
├── mod.rs           # TelegramBridge struct, auto-start
├── config.rs        # Config struct, load/save with permissions
├── client.rs        # HTTP client for Bot API
├── protocol.rs      # Telegram API types (Message, Update, etc.)
├── polling.rs       # Long polling loop, message dispatch
└── handler.rs       # MessageHandler trait for pluggable handlers
```

### Key Types

**`TelegramBridge`** — Main controller:
```rust
pub struct TelegramBridge {
    config_path: PathBuf,
    cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
}
```

**`TelegramClient`** — HTTP wrapper for Bot API:
```rust
pub struct TelegramClient {
    client: reqwest::Client,
    token: Option<String>,
    base_url: String,
}
```

**`MessageHandler`** — Pluggable interface:
```rust
#[async_trait]
pub trait MessageHandler: Send + Sync + 'static {
    async fn on_message(&self, chat_id: i64, text: String, from: Option<User>);
}
```

## Testing

Run Telegram config tests:
```bash
cargo test --features gui telegram::config::tests
```

Tests cover:
- Config serialization/deserialization
- File save/load with allowed_chat_ids
- File deletion
- Timestamp initialization

## Security Considerations

1. **Bot token exposure**: Never log or expose the bot token. Removed `TelegramClient::bot_token()` getter.
2. **File permissions**: Config file is `0o600` (Unix-only, falls back gracefully on Windows)
3. **Atomic writes**: Prevents corruption if process crashes mid-write
4. **Authorization**: `allowed_chat_ids` prevents unauthorized access
5. **Input validation**: Bot token validated against Telegram API before saving

## Limitations (Phase 3)

- ❌ No inline keyboard approval routing (uses GUI approver)
- ❌ No media/file upload support (text only)
- ❌ No group chat support (direct messages only)
- ❌ No technical manual for setup (this document)

## Future Enhancements

- [ ] `TelegramApprover` struct for inline keyboard approvals
- [ ] Media/file upload handling
- [ ] Group chat support with admin authorization
- [ ] Browser chat surface (like LINE plan-10)
- [ ] Markdown/HTML formatting in responses
- [ ] Rate limiting for high-traffic bots

## References

- [Telegram Bot API Documentation](https://core.telegram.org/bots/api)
- [LINE Bridge Documentation](line-bridge.md)
- [Permission System Documentation](permissions.md)
