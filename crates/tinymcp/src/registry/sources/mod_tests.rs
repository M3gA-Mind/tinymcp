//! Unit tests for the registry dispatcher and the Smithery adapter.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;

use super::encode::encode_path_segment;
use super::shared::{MAX_ERROR_BODY_BYTES, truncate};
use super::smithery::tag_source;
use super::types::{
    Registries, RegistryOperation, RegistrySource, RegistryTimeouts, SOURCE_MCP_OFFICIAL,
    SOURCE_SMITHERY,
};
use tinymcp_bus::{McpRegistryAuthConfig, RegistryServerSummary};

/// A dispatcher with the given registry credentials.
fn registries(auth: McpRegistryAuthConfig) -> Registries {
    Registries::new(auth).expect("the dispatcher builds")
}

/// Registry credentials carrying a Smithery key.
fn with_smithery_key() -> McpRegistryAuthConfig {
    McpRegistryAuthConfig {
        smithery_api_key: Some("key-1".into()),
        ..McpRegistryAuthConfig::default()
    }
}

// ---------------------------------------------------------------------------
// Operations and their budgets
// ---------------------------------------------------------------------------

#[test]
fn a_listing_with_a_query_is_a_search_and_without_one_a_browse() {
    assert_eq!(RegistryOperation::for_query(""), RegistryOperation::Browse);
    assert_eq!(
        RegistryOperation::for_query("github"),
        RegistryOperation::Search
    );
}

#[test]
fn every_operation_renders_its_lowercase_name() {
    for (operation, name) in [
        (RegistryOperation::Browse, "browse"),
        (RegistryOperation::Search, "search"),
        (RegistryOperation::Detail, "detail"),
    ] {
        assert_eq!(operation.as_str(), name);
        assert_eq!(operation.to_string(), name);
    }
}

#[test]
fn the_default_budgets_are_pinned() {
    use std::time::Duration;

    let timeouts = RegistryTimeouts::default();

    assert_eq!(timeouts.connect, Duration::from_secs(5));
    assert_eq!(
        timeouts.budget(RegistryOperation::Browse),
        Duration::from_secs(15)
    );
    assert_eq!(
        timeouts.budget(RegistryOperation::Search),
        Duration::from_secs(8)
    );
    assert_eq!(
        timeouts.budget(RegistryOperation::Detail),
        Duration::from_secs(12)
    );
}

// ---------------------------------------------------------------------------
// Source identifiers
// ---------------------------------------------------------------------------

#[test]
fn every_source_round_trips_through_its_identifier() {
    for source in [RegistrySource::McpOfficial, RegistrySource::Smithery] {
        assert_eq!(RegistrySource::parse(source.as_str()), Some(source));
    }
}

#[test]
fn the_source_identifiers_are_pinned() {
    // A caller attributes a row by this string, and an install routes its
    // detail lookup by it.
    assert_eq!(RegistrySource::McpOfficial.as_str(), SOURCE_MCP_OFFICIAL);
    assert_eq!(RegistrySource::Smithery.as_str(), SOURCE_SMITHERY);
    assert_eq!(SOURCE_MCP_OFFICIAL, "mcp_official");
    assert_eq!(SOURCE_SMITHERY, "smithery");
}

#[test]
fn an_unknown_identifier_names_no_source() {
    assert_eq!(RegistrySource::parse("nope"), None);
    assert_eq!(RegistrySource::parse(""), None);
}

// ---------------------------------------------------------------------------
// Which sources take part in a search
// ---------------------------------------------------------------------------

#[test]
fn the_official_registry_always_searches_and_leads() {
    // Its rows lead a merged result.
    let searchable = registries(McpRegistryAuthConfig::default()).searchable();

    assert_eq!(searchable.first(), Some(&RegistrySource::McpOfficial));
}

#[test]
fn smithery_does_not_search_without_a_key() {
    // Its servers cannot be connected without one, so listing them would fill
    // the catalog with rows that look installable and are not.
    let searchable = registries(McpRegistryAuthConfig::default()).searchable();

    assert!(!searchable.contains(&RegistrySource::Smithery));
    assert_eq!(searchable.len(), 1);
}

#[test]
fn smithery_searches_once_a_key_is_configured() {
    let searchable = registries(with_smithery_key()).searchable();

    assert!(searchable.contains(&RegistrySource::Smithery));
    assert_eq!(searchable.len(), 2);
}

#[test]
fn a_blank_key_counts_as_no_key() {
    // Otherwise every request carries a bare `Bearer ` and fails.
    for blank in ["", "   ", "\t\n"] {
        let dispatcher = registries(McpRegistryAuthConfig {
            smithery_api_key: Some(blank.into()),
            ..McpRegistryAuthConfig::default()
        });

        assert_eq!(dispatcher.smithery_key(), None, "{blank:?}");
        assert!(!dispatcher.searchable().contains(&RegistrySource::Smithery));
    }
}

#[test]
fn a_configured_key_is_reported_trimmed_of_nothing_but_read_as_set() {
    assert_eq!(
        registries(with_smithery_key()).smithery_key().as_deref(),
        Some("key-1")
    );
}

// ---------------------------------------------------------------------------
// Detail routing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_detail_lookup_for_an_unknown_source_is_refused() {
    let store = crate::registry::Store::open_in_memory().unwrap();
    let dispatcher = registries(McpRegistryAuthConfig::default());

    let error = dispatcher
        .get(&store, "nope", "some/server")
        .await
        .expect_err("an unknown source");

    assert!(
        matches!(error, crate::Error::UnknownServer { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_smithery_detail_lookup_routes_to_smithery() {
    let store = crate::registry::Store::open_in_memory().unwrap();
    store
        .cache(
            "smithery:detail:@acme/weather",
            &json!({ "qualifiedName": "@acme/weather", "displayName": "Weather" }).to_string(),
        )
        .unwrap();

    let detail = registries(McpRegistryAuthConfig::default())
        .get(&store, SOURCE_SMITHERY, "@acme/weather")
        .await
        .expect("served from the cache");

    assert_eq!(detail.source, SOURCE_SMITHERY);
}

// ---------------------------------------------------------------------------
// Search routing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_smithery_search_is_live_and_a_zero_page_size_means_the_default() {
    let store = crate::registry::Store::open_in_memory().unwrap();
    store
        .cache(
            "smithery:search:weather:1:20",
            &json!({
                "servers": [{ "qualifiedName": "@acme/weather", "displayName": "Weather" }],
                "pagination": { "totalPages": 3 },
            })
            .to_string(),
        )
        .unwrap();

    let page = registries(with_smithery_key())
        .search(&store, RegistrySource::Smithery, Some("  weather "), 0, 0)
        .await
        .expect("served from the cache");

    assert_eq!(page.servers[0].qualified_name, "@acme/weather");
    assert_eq!(page.total_pages, 3);
    assert_eq!(page.freshness, tinymcp_bus::RegistryFreshness::Live);
}

// ---------------------------------------------------------------------------
// Smithery trust-signal scrubbing
// ---------------------------------------------------------------------------

/// A Smithery row claiming both trust signals, on the field and in the
/// passthrough bucket.
fn spoofed_row() -> RegistryServerSummary {
    let mut row: RegistryServerSummary = serde_json::from_value(json!({
        "qualified_name": "smi/evil",
        "display_name": "Evil",
    }))
    .unwrap();

    row.website_url = Some("https://spoofed.test".into());
    row.auth_kind = Some("api_key".into());
    row.extra
        .insert("website_url".to_string(), json!("https://spoofed.test"));
    row.extra.insert("auth_kind".to_string(), json!("api_key"));
    row
}

#[test]
fn a_smithery_row_is_stamped_with_its_source() {
    let tagged = tag_source(vec![spoofed_row()]);
    assert_eq!(tagged[0].source, SOURCE_SMITHERY);
}

#[test]
fn a_smithery_row_that_already_names_a_source_keeps_it() {
    let mut row = spoofed_row();
    row.source = "something-else".into();

    let tagged = tag_source(vec![row]);
    assert_eq!(tagged[0].source, "something-else");
}

#[test]
fn a_smithery_row_cannot_carry_trust_signals_on_its_fields() {
    // They decide whether a row passes the strict catalog filter, and they are
    // derived by the official adapter from metadata it checked.
    let tagged = tag_source(vec![spoofed_row()]);

    assert_eq!(tagged[0].website_url, None);
    assert_eq!(tagged[0].auth_kind, None);
}

#[test]
fn a_smithery_row_cannot_smuggle_trust_signals_through_the_passthrough_bucket() {
    // A value left there would serialize straight back out and read as though
    // the adapter had set it.
    let tagged = tag_source(vec![spoofed_row()]);

    assert!(!tagged[0].extra.contains_key("website_url"));
    assert!(!tagged[0].extra.contains_key("auth_kind"));

    let encoded = serde_json::to_value(&tagged[0]).unwrap();
    assert!(!encoded.to_string().contains("spoofed.test"), "{encoded}");
}

#[test]
fn scrubbing_leaves_everything_else_alone() {
    let mut row = spoofed_row();
    row.extra.insert("useCount".to_string(), json!(42));

    let tagged = tag_source(vec![row]);

    assert_eq!(tagged[0].display_name, "Evil");
    assert_eq!(tagged[0].extra["useCount"], json!(42));
}

// ---------------------------------------------------------------------------
// Path encoding
// ---------------------------------------------------------------------------

#[test]
fn a_plain_name_is_left_alone() {
    assert_eq!(encode_path_segment("simple-name"), "simple-name");
    assert_eq!(encode_path_segment("a.b_c~d"), "a.b_c~d");
}

#[test]
fn a_scope_marker_survives_but_its_separator_does_not() {
    // The whole qualified name is one path segment; an unencoded slash would
    // address a different resource.
    let encoded = encode_path_segment("@modelcontextprotocol/server-filesystem");

    assert!(encoded.starts_with('@'), "{encoded}");
    assert!(!encoded.contains('/'), "{encoded}");
    assert!(encoded.contains("%2F"), "{encoded}");
}

#[test]
fn a_space_is_encoded() {
    assert_eq!(encode_path_segment("hello world"), "hello%20world");
}

#[test]
fn encoding_uses_uppercase_hexadecimal() {
    // The form RFC 3986 prefers, and what makes two encoders agree.
    assert_eq!(encode_path_segment("/"), "%2F");
    assert_eq!(encode_path_segment("?"), "%3F");
}

#[test]
fn a_multibyte_character_is_encoded_byte_by_byte() {
    // `é` is two bytes in UTF-8.
    assert_eq!(encode_path_segment("é"), "%C3%A9");
}

#[test]
fn encoding_nothing_yields_nothing() {
    assert_eq!(encode_path_segment(""), "");
}

// ---------------------------------------------------------------------------
// Error-body truncation
// ---------------------------------------------------------------------------

#[test]
fn a_short_body_is_left_alone() {
    assert_eq!(truncate("short", MAX_ERROR_BODY_BYTES), "short");
}

#[test]
fn a_long_body_is_bounded() {
    // These reach a log line and an error message; an upstream answering with a
    // whole error page would otherwise put all of it there.
    let bounded = truncate(&"x".repeat(10_000), MAX_ERROR_BODY_BYTES);
    assert_eq!(bounded.len(), MAX_ERROR_BODY_BYTES);
}

#[test]
fn truncation_does_not_split_a_character() {
    let bounded = truncate(&"é".repeat(500), MAX_ERROR_BODY_BYTES);

    assert!(bounded.len() <= MAX_ERROR_BODY_BYTES);
    assert!(bounded.chars().all(|character| character == 'é'));
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

#[test]
fn a_failure_body_is_truncated_on_a_character_boundary() {
    // Upstream bodies are arbitrary bytes and end up in a message. Splitting
    // one mid-sequence would panic.
    let truncated = super::shared::truncate(&"é".repeat(300), 201);

    assert!(truncated.len() <= 201);
    assert!(truncated.chars().all(|character| character == 'é'));
}

#[test]
fn a_body_within_the_bound_is_left_alone() {
    assert_eq!(super::shared::truncate("short", 200), "short");
}

#[test]
fn a_cache_that_cannot_be_written_costs_a_round_trip_rather_than_the_search() {
    // Failing a user's search because a cache write failed would cost them the
    // search; the only price for skipping it is one more request next time.
    let store =
        crate::registry::Store::open(crate::storage::StoragePorts::with_failing_documents())
            .unwrap();
    assert!(store.cache("warmup", "{}").is_err());

    // Returns nothing and does not panic.
    super::shared::cache(&store, "key", "{}");
}
