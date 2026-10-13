//! Tests for [`SecretScrubber`]: which secrets it learns from a server's
//! credentials and endpoint, and what it leaves alone.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use tinymcp_bus::{
    HttpHeader, McpAuthConfig, McpResourceContents, McpToolContent, McpToolResult, ToolCallOutcome,
};

use super::scrub::SecretScrubber;

#[test]
fn scrubber_redacts_url_encoded_secrets_and_ignores_empty_values() {
    let scrubber = SecretScrubber::new(
        &McpAuthConfig::BearerToken {
            token: "a b/c".into(),
        },
        "https://example.com/mcp",
    );
    assert_eq!(
        scrubber.scrub("x a%20b%2Fc y a b/c"),
        "x [redacted] y [redacted]"
    );

    let empty = SecretScrubber::new(
        &McpAuthConfig::BearerToken { token: "  ".into() },
        "https://example.com/mcp?",
    );
    assert_eq!(empty.secrets.len(), 0);
    assert_eq!(empty.scrub("unchanged"), "unchanged");
}

#[test]
fn scrubber_collects_url_userinfo_credentials() {
    let scrubber = SecretScrubber::new(&McpAuthConfig::None, "https://short:pw@example.com/mcp");
    assert_eq!(
        scrubber.scrub("short pw shortpw"),
        "[redacted] [redacted] [redacted][redacted]"
    );
}

#[test]
fn scrubber_does_not_globally_redact_ordinary_short_query_values() {
    let scrubber = SecretScrubber::new(
        &McpAuthConfig::None,
        "https://example.com/mcp?v=2&format=json&api_token=abcdef1234",
    );
    assert_eq!(
        scrubber.scrub("v=2 and format=json are unrelated"),
        "v=2 and format=json are unrelated"
    );
    assert_eq!(
        scrubber.scrub("leaked abcdef1234 here"),
        "leaked [redacted] here"
    );
}

#[test]
fn scrubber_redacts_credential_query_value() {
    let scrubber = SecretScrubber::new(
        &McpAuthConfig::None,
        "https://example.com/mcp?credential=private12345",
    );
    assert_eq!(
        scrubber.scrub("server echoed private12345"),
        "server echoed [redacted]"
    );
}

#[test]
fn scrubber_redacts_short_query_credentials_even_inside_other_text() {
    let scrubber = SecretScrubber::new(
        &McpAuthConfig::None,
        "https://example.com/mcp?api_key=abc&v=2",
    );
    assert_eq!(
        scrubber.scrub("abc is a credential; prefixabc also contains it"),
        "[redacted] is a credential; prefix[redacted] also contains it"
    );
    assert_eq!(scrubber.scrub("v=2"), "v=2");
}

#[test]
fn scrubber_redacts_credentials_from_bus_tool_outcomes() {
    let scrubber = SecretScrubber::new(
        &McpAuthConfig::BearerToken {
            token: "private-token-value".into(),
        },
        "https://example.test/mcp?api_key=query-secret-value",
    );
    let mut outcome = ToolCallOutcome {
        result: json!({"echo": "private-token-value", "nested": ["query-secret-value"]}),
        is_error: false,
        rendered: McpToolResult {
            content: vec![
                McpToolContent::Text {
                    text: "private-token-value".into(),
                },
                McpToolContent::Json {
                    data: json!({"echo": "query-secret-value"}),
                },
            ],
            markdown_formatted: Some("private-token-value".into()),
            structured_content: Some(json!({"echo": "query-secret-value"})),
            meta: Some(json!({"echo": "private-token-value"})),
            resources: vec![McpResourceContents {
                uri: "private-token-value".into(),
                mime_type: None,
                text: Some("query-secret-value".into()),
                blob: None,
                meta: Some(json!({"echo": "private-token-value"})),
            }],
            ..McpToolResult::default()
        },
    };

    scrubber.scrub_tool_call_outcome(&mut outcome);
    let encoded = serde_json::to_string(&outcome).unwrap();
    assert!(!encoded.contains("private-token-value"), "{encoded}");
    assert!(!encoded.contains("query-secret-value"), "{encoded}");
    assert!(encoded.matches("[redacted]").count() >= 8, "{encoded}");
}

#[test]
fn resource_contents_lose_known_secrets_from_text_and_meta_but_keep_blobs() {
    let scrubber = SecretScrubber::new(
        &McpAuthConfig::BearerToken {
            token: "resource-secret-token".into(),
        },
        "https://example.com/mcp",
    );
    let mut contents = vec![
        McpResourceContents {
            uri: "ui://card".into(),
            mime_type: Some("text/html".into()),
            text: Some("<p>resource-secret-token</p>".into()),
            blob: None,
            meta: Some(json!({ "token": "resource-secret-token" })),
        },
        McpResourceContents {
            uri: "ui://logo".into(),
            mime_type: None,
            text: None,
            blob: Some("cmVzb3VyY2Utc2VjcmV0LXRva2Vu".into()),
            meta: None,
        },
    ];

    scrubber.scrub_resource_contents(&mut contents);

    assert_eq!(contents[0].text.as_deref(), Some("<p>[redacted]</p>"));
    assert_eq!(contents[0].meta, Some(json!({ "token": "[redacted]" })));
    assert_eq!(
        contents[1].blob.as_deref(),
        Some("cmVzb3VyY2Utc2VjcmV0LXRva2Vu")
    );
}

#[test]
fn an_empty_scrubber_leaves_resource_contents_untouched() {
    let scrubber = SecretScrubber::new(&McpAuthConfig::None, "https://example.com/mcp");
    let original = McpResourceContents {
        uri: "ui://card".into(),
        mime_type: None,
        text: Some("<p>hi</p>".into()),
        blob: None,
        meta: None,
    };
    let mut contents = vec![original.clone()];

    scrubber.scrub_resource_contents(&mut contents);

    assert_eq!(contents, vec![original]);
}

#[test]
fn short_query_credentials_do_not_rewrite_json_structure_keys() {
    let scrubber = SecretScrubber::new(&McpAuthConfig::None, "https://example.com/mcp?api_key=abc");
    let mut value = json!({ "prefixabc": "prefixabc", "abc": "abc" });
    scrubber.scrub_value(&mut value);
    assert_eq!(value["prefixabc"], "prefix[redacted]");
    assert_eq!(value["[redacted]"], "[redacted]");
    assert!(value.get("prefix[redacted]").is_none());
}

#[test]
fn short_auth_values_do_not_rewrite_unrelated_words() {
    let scrubber = SecretScrubber::new(
        &McpAuthConfig::Basic {
            username: "abc".into(),
            password: "private12345".into(),
        },
        "https://example.com/mcp",
    );
    assert_eq!(
        scrubber.scrub("abc identifies the user; alphabet is unrelated"),
        "[redacted] identifies the user; alphabet is unrelated"
    );
}

#[test]
fn scrub_value_keeps_both_entries_when_keys_collide_after_redaction() {
    let scrubber = SecretScrubber::new(
        &McpAuthConfig::Headers {
            headers: vec![
                HttpHeader::new("one", "first-secret"),
                HttpHeader::new("two", "second-secret"),
            ],
        },
        "https://example.com/mcp",
    );
    let mut value = json!({
        "first-secret": "a",
        "second-secret": "b",
    });
    scrubber.scrub_value(&mut value);
    let map = value.as_object().expect("object");
    assert_eq!(map.len(), 2, "{value}");
    assert_eq!(map.get("[redacted]"), Some(&json!("a")));
    assert_eq!(map.get("[redacted] (2)"), Some(&json!("b")));
}

#[test]
fn extra_secrets_are_redacted_as_typed_and_url_encoded() {
    let scrubber = SecretScrubber::new(&McpAuthConfig::None, "https://example.com/mcp")
        .with_secrets(["host token/1".to_string(), "   ".to_string()]);
    assert_eq!(
        scrubber.scrub("a host token/1 b host%20token%2F1 c"),
        "a [redacted] b [redacted] c"
    );
    assert_eq!(scrubber.secrets.len(), 2);
}

#[test]
fn extra_secrets_join_the_configured_ones_longest_first() {
    let scrubber = SecretScrubber::new(
        &McpAuthConfig::BearerToken {
            token: "configured-token".into(),
        },
        "https://example.com/mcp",
    )
    .with_secrets(["configured-token-extended".to_string()]);
    assert_eq!(
        scrubber.scrub("configured-token-extended and configured-token"),
        "[redacted] and [redacted]"
    );
}

#[cfg(feature = "tools")]
#[test]
fn scrubber_redacts_secrets_from_the_tinytools_adapter_result() {
    let scrubber = SecretScrubber::new(
        &McpAuthConfig::BearerToken {
            token: "configured-token".into(),
        },
        "https://example.com/mcp",
    )
    .with_secrets(["configured-token-extended".to_string()]);
    let result = scrubber.scrub_result(tinytools::ToolResult::json(
        json!({ "echo": "configured-token-extended" }),
    ));
    assert!(!format!("{:?}", result.content).contains("configured-token"));
}

#[test]
fn a_short_extra_secret_is_redacted_inside_larger_text() {
    let scrubber = SecretScrubber::new(&McpAuthConfig::None, "https://example.com/mcp")
        .with_secrets(["k9".to_string()]);
    assert_eq!(
        scrubber.scrub("token=xk9y and k9"),
        "token=x[redacted]y and [redacted]"
    );
    let mut value = json!({ "k9": "a", "tool_k9x": "b" });
    scrubber.scrub_value(&mut value);
    let map = value.as_object().unwrap();
    assert!(map.contains_key("[redacted]"), "{value}");
    assert!(map.contains_key("tool_k9x"), "{value}");
}

#[test]
fn debug_output_never_contains_a_secret() {
    let scrubber = SecretScrubber::new(
        &McpAuthConfig::BearerToken {
            token: "configured-token".into(),
        },
        "https://example.com/mcp",
    )
    .with_secrets(["host token/1".to_string()]);
    let shown = format!("{scrubber:?} {scrubber:#?}");
    assert!(!shown.contains("configured-token"), "{shown}");
    assert!(!shown.contains("host"), "{shown}");
    assert!(!shown.contains("%2F"), "{shown}");
    assert!(shown.contains("SecretScrubber"), "{shown}");
}
