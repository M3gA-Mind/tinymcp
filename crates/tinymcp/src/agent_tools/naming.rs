//! The names a model sees: `mcp_<server>_<tool>`.
//!
//! A name has to be three things at once. **Readable**, because a model picks
//! tools by name and a person reads them in a transcript: `mcp_ticktick_read_goals`
//! says what it is. **Provider-safe**: `[a-z0-9_]`, at most
//! [`MAX_TOOL_NAME_LEN`] bytes, which every major provider accepts. And
//! **stable**: the same server and tool produce the same name in every process
//! and on every launch, because a resumed conversation replays the names it
//! recorded.
//!
//! Generated names include a short digest of the server identity and tool
//! name. That keeps a recorded name stable if another server with the same
//! readable slug is added or removed. A name that would run past the limit
//! keeps as much of both readable parts as fits before the digest.

use sha2::{Digest as _, Sha256};

/// The longest name produced. 64 is the tightest limit among the major model
/// providers' function-name rules.
pub const MAX_TOOL_NAME_LEN: usize = 64;

/// The prefix every MCP tool name carries.
pub const TOOL_NAME_PREFIX: &str = "mcp_";

/// The most of a name the server part may take, so a long server name cannot
/// crowd out the tool it is naming.
const MAX_SERVER_SLUG_LEN: usize = 24;

/// Hex digits of digest appended when a name is shortened or disambiguated.
const SUFFIX_HEX_LEN: usize = 6;

/// Words that say nothing about *which* server this is. Dropped from a server
/// slug when something else is left: `ticktick-mcp-server` is `ticktick`.
const NOISE_WORDS: &[&str] = &["mcp", "server"];

/// `input` as lower-case `[a-z0-9_]`, with camel-case boundaries split, runs of
/// separators collapsed, and no leading or trailing underscore.
///
/// `readGoals`, `read-goals` and `Read Goals` all become `read_goals`.
#[must_use]
pub fn slug(input: &str) -> String {
    slug_with(input, true)
}

/// [`slug`], optionally without the camel-case split. Server names are brand
/// names — `GitHub`, `TickTick` — and read worse split.
fn slug_with(input: &str, split_camel: bool) -> String {
    let mut out = String::with_capacity(input.len());
    let mut previous: Option<char> = None;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            let boundary = split_camel
                && ch.is_ascii_uppercase()
                && previous.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit());
            if boundary && !out.ends_with('_') {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
        previous = Some(ch);
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

/// The server part of a name, from a qualified name (`@scope/ticktick-mcp`), a
/// configured name, or a display name.
///
/// The last path segment is used, the scope is dropped, and noise words are
/// removed. Never empty.
#[must_use]
pub fn server_slug(label: &str) -> String {
    let segment = label.rsplit('/').next().unwrap_or(label);
    let base = slug_with(segment.trim_start_matches('@'), false);
    let words: Vec<&str> = base.split('_').filter(|word| !word.is_empty()).collect();
    let meaningful: Vec<&str> = words
        .iter()
        .copied()
        .filter(|word| !NOISE_WORDS.contains(word))
        .collect();
    let chosen = if meaningful.is_empty() {
        words
    } else {
        meaningful
    };
    let joined = truncate(&chosen.join("_"), MAX_SERVER_SLUG_LEN);
    if joined.is_empty() {
        "server".to_string()
    } else {
        joined
    }
}

/// The name for `tool` on the server labelled `server_label`:
/// `mcp_<server>_<tool>`.
///
/// Shortened with a digest suffix only when the plain form would exceed
/// [`MAX_TOOL_NAME_LEN`].
#[must_use]
pub fn tool_name(server_label: &str, tool: &str) -> String {
    let server = server_slug(server_label);
    let tool_part = non_empty(slug(tool), "tool");
    let plain = format!("{TOOL_NAME_PREFIX}{server}_{tool_part}");
    if plain.len() <= MAX_TOOL_NAME_LEN {
        return plain;
    }
    with_suffix(&server, &tool_part, &digest(&[server_label, tool]))
}

/// A name for `tool` that includes a digest of the server's identity.
///
/// Use this for generated tool registrations so adding another server with the
/// same readable slug cannot change an existing tool's name.
#[must_use]
pub fn disambiguated_tool_name(server_id: &str, server_label: &str, tool: &str) -> String {
    let server = server_slug(server_label);
    let tool_part = non_empty(slug(tool), "tool");
    with_suffix(&server, &tool_part, &digest(&[server_id, tool]))
}

/// The name scheme used before readable names: `mcp_<tool slug>_<12 hex>`.
///
/// Kept so a conversation that recorded one of these names still resolves it.
/// Byte-for-byte what earlier `OpenHuman` builds produced.
#[must_use]
pub fn legacy_tool_name(server_id: &str, tool: &str) -> String {
    let raw: String = tool
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(42)
        .collect();
    let trimmed = raw.trim_matches('_');
    let trimmed = if trimmed.is_empty() { "tool" } else { trimmed };
    let full = hex(&Sha256::digest(format!("{server_id}\0{tool}").as_bytes()));
    format!("{TOOL_NAME_PREFIX}{trimmed}_{}", &full[..12])
}

/// `mcp_<server>_<tool>_<suffix>`, with the tool part cut to fit.
fn with_suffix(server: &str, tool: &str, suffix: &str) -> String {
    let fixed = TOOL_NAME_PREFIX.len() + server.len() + 1 + 1 + suffix.len();
    let room = MAX_TOOL_NAME_LEN.saturating_sub(fixed);
    let tool = non_empty(truncate(tool, room), "tool");
    format!("{TOOL_NAME_PREFIX}{server}_{tool}_{suffix}")
}

/// The first [`SUFFIX_HEX_LEN`] hex digits of a digest over `parts`.
fn digest(parts: &[&str]) -> String {
    let full = hex(&Sha256::digest(parts.join("\0").as_bytes()));
    full[..SUFFIX_HEX_LEN].to_string()
}

/// Cuts an ASCII slug to `max` bytes without leaving a trailing underscore.
fn truncate(slug: &str, max: usize) -> String {
    let cut = &slug[..slug.len().min(max)];
    cut.trim_end_matches('_').to_string()
}

fn non_empty(value: String, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
