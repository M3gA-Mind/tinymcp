//! Scrubbing configured credentials before remote MCP data crosses a boundary.
//!
//! A remote server can echo a credential it was given: in an error body, a
//! tool description, or a result. [`SecretScrubber`] knows every secret one
//! configured server was dialled with — bearer tokens, basic-auth pairs,
//! header and query-parameter values, URL userinfo, credential-looking query
//! values — and replaces each occurrence with `[redacted]` in text, JSON, and
//! bus tool outcomes. The module applies it before static and installed-server
//! results reach a host; the optional `TinyTools` adapter reuses the same scrubber.

use base64::Engine as _;
use serde_json::Value;
use tinymcp_bus::McpAuthConfig;
use tinymcp_bus::{McpResourceContents, McpToolContent, ToolCallOutcome};
#[cfg(feature = "tools")]
use tinytools::{ToolContent, ToolResult};

use crate::config_servers::McpServerRegistry;

/// What a scrubbed secret is replaced with.
pub const REDACTED: &str = "[redacted]";
/// Shorter secrets are only replaced as whole words.
const MIN_QUERY_SECRET_LEN: usize = 8;
const CREDENTIAL_QUERY_PARAM_NEEDLES: [&str; 7] = [
    "token",
    "key",
    "secret",
    "password",
    "auth",
    "sig",
    "credential",
];

/// Every secret one configured server was dialled with, ready to scrub.
#[derive(Clone)]
pub struct SecretScrubber {
    pub(super) secrets: Vec<String>,
    strict: Vec<String>,
}

impl std::fmt::Debug for SecretScrubber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretScrubber")
            .field("secrets", &self.secrets.len())
            .field("strict", &self.strict.len())
            .finish()
    }
}

impl SecretScrubber {
    /// The scrubber for the server named `server`, or an empty one (which
    /// scrubs nothing) when `registry` has no such server.
    #[must_use]
    pub fn for_server(registry: &McpServerRegistry, server: &str) -> Self {
        let Some(definition) = registry.get(server) else {
            return Self {
                secrets: Vec::new(),
                strict: Vec::new(),
            };
        };
        Self::new(&definition.auth, &definition.endpoint)
    }

    /// The scrubber for a server reached at `endpoint` with `auth`.
    #[must_use]
    pub fn new(auth: &McpAuthConfig, endpoint: &str) -> Self {
        let mut raw: Vec<String> = Vec::new();
        let mut strict: Vec<String> = Vec::new();
        if let Ok(url) = url::Url::parse(endpoint) {
            if !url.username().is_empty() {
                strict.push(url.username().to_string());
            }
            if let Some(password) = url.password() {
                strict.push(password.to_string());
            }
        }
        match auth {
            McpAuthConfig::BearerToken { token } => raw.push(token.clone()),
            McpAuthConfig::Basic { username, password } => {
                raw.push(username.clone());
                raw.push(password.clone());
                raw.push(
                    base64::engine::general_purpose::STANDARD
                        .encode(format!("{username}:{password}")),
                );
            }
            McpAuthConfig::Header { value, .. } | McpAuthConfig::QueryParam { value, .. } => {
                raw.push(value.clone());
            }
            McpAuthConfig::Headers { headers } => {
                raw.extend(headers.iter().map(|header| header.value.clone()));
            }
            _ => {}
        }
        if endpoint_query(endpoint).is_some()
            && let Ok(url) = url::Url::parse(endpoint)
        {
            strict.extend(url.query_pairs().filter_map(|(name, value)| {
                let name = name.to_ascii_lowercase();
                let credential_like = CREDENTIAL_QUERY_PARAM_NEEDLES
                    .iter()
                    .any(|needle| name.contains(needle));
                let value = value.into_owned();
                credential_like.then_some(value)
            }));
            // Retain the spelling supplied in the endpoint: form decoding turns
            // '+' into a space, which ordinary URL encoding does not recreate.
            if let Some(query) = url.query() {
                for pair in query.split('&') {
                    if let Some((name, value)) = pair.split_once('=') {
                        let decoded_name = url::form_urlencoded::parse(name.as_bytes())
                            .next()
                            .map(|(name, _)| name.to_ascii_lowercase())
                            .unwrap_or_default();
                        if CREDENTIAL_QUERY_PARAM_NEEDLES
                            .iter()
                            .any(|needle| decoded_name.contains(needle))
                        {
                            strict.push(value.to_string());
                        }
                    }
                }
            }
        }

        strict.retain(|value| !value.trim().is_empty());
        let encoded_strict = strict
            .iter()
            .map(|value| urlencoding::encode(value).into_owned())
            .collect::<Vec<_>>();
        strict.extend(encoded_strict);
        raw.extend(strict.iter().cloned());

        let mut secrets: Vec<String> = Vec::new();
        for value in raw {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let encoded = urlencoding::encode(value).into_owned();
            if encoded != value {
                secrets.push(encoded);
            }
            secrets.push(value.to_string());
        }
        sort_longest_first(&mut secrets);
        Self { secrets, strict }
    }

    /// This scrubber, also replacing each of `secrets`.
    ///
    /// For credentials the server's own configuration does not carry: a token
    /// a host keeps in its own secret store, or a value it injected some other
    /// way. Each is matched as typed and URL-encoded, anywhere in the text even
    /// when short; blank values are skipped.
    #[must_use]
    pub fn with_secrets(mut self, secrets: impl IntoIterator<Item = String>) -> Self {
        for secret in secrets {
            let secret = secret.trim();
            if secret.is_empty() {
                continue;
            }
            let encoded = urlencoding::encode(secret).into_owned();
            if encoded != secret {
                self.strict.push(encoded.clone());
                self.secrets.push(encoded);
            }
            self.strict.push(secret.to_string());
            self.secrets.push(secret.to_string());
        }
        sort_longest_first(&mut self.secrets);
        self
    }

    /// `text` with every known secret replaced.
    #[must_use]
    pub fn scrub(&self, text: &str) -> String {
        self.scrub_text(text, true)
    }

    /// An error's message, scrubbed.
    #[must_use]
    #[cfg(feature = "tools")]
    pub fn scrub_error(&self, error: &anyhow::Error) -> String {
        self.scrub(&error.to_string())
    }

    fn scrub_key(&self, text: &str) -> String {
        // Object keys carry structure (tool names and JSON schema fields). Keep
        // short query credentials from matching inside those names while still
        // removing them when they are the complete key.
        self.scrub_text(text, false)
    }

    fn scrub_text(&self, text: &str, redact_short_substrings: bool) -> String {
        let mut out = text.to_string();
        for secret in &self.secrets {
            if secret.len() < MIN_QUERY_SECRET_LEN
                && (!self.strict.contains(secret) || !redact_short_substrings)
            {
                // Short credentials are common words or field-name fragments;
                // only replace a complete token so unrelated text stays usable.
                let mut next = String::with_capacity(out.len());
                let mut cursor = 0;
                for (start, _) in out.match_indices(secret.as_str()) {
                    if start < cursor {
                        continue;
                    }
                    let end = start + secret.len();
                    let word_char = |ch: char| ch.is_alphanumeric() || ch == '_';
                    let before = out[..start].chars().next_back().is_some_and(word_char);
                    let after = out[end..].chars().next().is_some_and(word_char);
                    if !before && !after {
                        next.push_str(&out[cursor..start]);
                        next.push_str(REDACTED);
                        cursor = end;
                    }
                }
                next.push_str(&out[cursor..]);
                out = next;
            } else if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), REDACTED);
            }
        }
        out
    }

    pub(crate) fn scrub_value(&self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.scrub(text),
            Value::Array(items) => items.iter_mut().for_each(|item| self.scrub_value(item)),
            Value::Object(map) => {
                let entries = std::mem::take(map);
                for (key, mut item) in entries {
                    self.scrub_value(&mut item);
                    let base_key = self.scrub_key(&key);
                    let mut unique_key = base_key.clone();
                    let mut suffix = 2;
                    while map.contains_key(&unique_key) {
                        unique_key = format!("{base_key} ({suffix})");
                        suffix += 1;
                    }
                    map.insert(unique_key, item);
                }
            }
            _ => {}
        }
    }

    /// Resource contents with known secrets replaced in their text and `_meta`.
    ///
    /// A base64 blob is left as the server sent it.
    pub fn scrub_resource_contents(&self, contents: &mut [McpResourceContents]) {
        if self.secrets.is_empty() {
            return;
        }
        for part in contents {
            if let Some(text) = part.text.as_mut() {
                *text = self.scrub(text);
            }
            if let Some(meta) = part.meta.as_mut() {
                self.scrub_value(meta);
            }
        }
    }

    /// A bus tool outcome with known secrets replaced in all text and JSON fields.
    pub fn scrub_tool_call_outcome(&self, outcome: &mut ToolCallOutcome) {
        if self.secrets.is_empty() {
            return;
        }
        self.scrub_value(&mut outcome.result);
        for block in &mut outcome.rendered.content {
            match block {
                McpToolContent::Text { text } => *text = self.scrub(text),
                McpToolContent::Json { data } => self.scrub_value(data),
                _ => {}
            }
        }
        if let Some(markdown) = outcome.rendered.markdown_formatted.as_mut() {
            *markdown = self.scrub(markdown);
        }
        if let Some(structured) = outcome.rendered.structured_content.as_mut() {
            self.scrub_value(structured);
        }
        if let Some(meta) = outcome.rendered.meta.as_mut() {
            self.scrub_value(meta);
        }
        for resource in &mut outcome.rendered.resources {
            resource.uri = self.scrub(&resource.uri);
            if let Some(text) = resource.text.as_mut() {
                *text = self.scrub(text);
            }
            if let Some(blob) = resource.blob.as_mut() {
                *blob = self.scrub(blob);
            }
            if let Some(meta) = resource.meta.as_mut() {
                self.scrub_value(meta);
            }
        }
    }

    /// A [`tinytools::ToolResult`] with every known secret replaced in its text, JSON and
    /// formatted Markdown blocks.
    #[must_use]
    #[cfg(feature = "tools")]
    pub fn scrub_result(&self, mut result: ToolResult) -> ToolResult {
        if self.secrets.is_empty() {
            return result;
        }
        let mut redactions = 0usize;
        for block in &mut result.content {
            match block {
                ToolContent::Text { text } => {
                    let scrubbed = self.scrub(text);
                    if scrubbed != *text {
                        redactions += 1;
                        *text = scrubbed;
                    }
                }
                ToolContent::Json { data } => {
                    let before = data.clone();
                    self.scrub_value(data);
                    if *data != before {
                        redactions += 1;
                    }
                }
                _ => {}
            }
        }
        if let Some(markdown) = result.markdown_formatted.as_mut() {
            let scrubbed = self.scrub(markdown);
            if scrubbed != *markdown {
                redactions += 1;
                *markdown = scrubbed;
            }
        }
        if redactions > 0 {
            tracing::debug!(
                redactions,
                "[mcp] redacted configured secrets from tool output"
            );
        }
        result
    }
}

/// Longest first, so a secret that contains another is replaced whole.
fn sort_longest_first(secrets: &mut Vec<String>) {
    secrets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    secrets.dedup();
}

fn endpoint_query(endpoint: &str) -> Option<&str> {
    let (_, rest) = endpoint.split_once('?')?;
    let query = rest.split('#').next().unwrap_or_default();
    (!query.is_empty()).then_some(query)
}
