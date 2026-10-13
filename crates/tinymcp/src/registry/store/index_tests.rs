//! Unit tests for the catalog index tables.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

const SOURCE: &str = "mcp_official";
const BASE: &str = "https://registry.test";

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

fn row(name: &str, label: &str, description: &str) -> IndexRow {
    IndexRow {
        qualified_name: name.to_string(),
        label: label.to_string(),
        description: description.to_string(),
        record_json: format!("{{\"name\":\"{name}\"}}"),
    }
}

fn terms(words: &[&str]) -> Vec<String> {
    words.iter().map(ToString::to_string).collect()
}

fn names(hits: &[IndexHit]) -> Vec<&str> {
    let mut names: Vec<&str> = hits.iter().map(|hit| hit.qualified_name.as_str()).collect();
    names.sort_unstable();
    names
}

/// Moves every timestamp of the index back by a day.
fn age(store: &Store) {
    store.age_collection(
        crate::storage::REGISTRY_INDEX,
        &["synced_at"],
        86_400_000,
    );
    store.age_collection(
        crate::storage::REGISTRY_INDEX_STATE,
        &["synced_at"],
        86_400_000,
    );
}

#[test]
fn a_store_with_no_sync_has_no_index_state() {
    assert_eq!(store().index_state(SOURCE).unwrap(), None);
}

#[test]
fn a_sync_records_each_page_and_its_cursor() {
    let store = store();
    store.begin_index_sync(SOURCE, BASE).unwrap();
    store
        .store_index_page(SOURCE, &[row("a/one", "a/one one", "first")], Some("2"))
        .unwrap();

    let state = store.index_state(SOURCE).unwrap().unwrap();
    assert_eq!(state.base_url, BASE);
    assert_eq!(state.cursor.as_deref(), Some("2"));
    assert_eq!(state.pages, 1);
    assert!(state.in_progress());
    assert_eq!(state.synced_at, None);
}

#[test]
fn finishing_a_sync_records_when_and_clears_the_resume_point() {
    let store = store();
    store.begin_index_sync(SOURCE, BASE).unwrap();
    store
        .store_index_page(SOURCE, &[row("a/one", "a/one", "")], None)
        .unwrap();
    store.finish_index_sync(SOURCE).unwrap();

    let state = store.index_state(SOURCE).unwrap().unwrap();
    assert!(!state.in_progress());
    assert!(state.synced_at.is_some());
    assert_eq!(state.cursor, None);
    assert_eq!(state.pages, 0);
}

#[test]
fn finishing_a_sync_drops_servers_it_did_not_see_again() {
    let store = store();
    store.begin_index_sync(SOURCE, BASE).unwrap();
    store
        .store_index_page(
            SOURCE,
            &[row("a/kept", "kept", ""), row("a/gone", "gone", "")],
            None,
        )
        .unwrap();
    store.finish_index_sync(SOURCE).unwrap();
    age(&store);

    store.begin_index_sync(SOURCE, BASE).unwrap();
    store
        .store_index_page(SOURCE, &[row("a/kept", "kept", "")], None)
        .unwrap();
    assert_eq!(
        names(&store.search_index(SOURCE, &[]).unwrap()),
        ["a/gone", "a/kept"],
        "an unfinished sync keeps the earlier rows searchable"
    );

    store.finish_index_sync(SOURCE).unwrap();
    assert_eq!(names(&store.search_index(SOURCE, &[]).unwrap()), ["a/kept"]);
}

#[test]
fn restarting_on_the_same_catalog_keeps_the_last_finished_sync() {
    let store = store();
    store.begin_index_sync(SOURCE, BASE).unwrap();
    store.finish_index_sync(SOURCE).unwrap();

    store.begin_index_sync(SOURCE, BASE).unwrap();

    assert!(
        store
            .index_state(SOURCE)
            .unwrap()
            .unwrap()
            .synced_at
            .is_some()
    );
}

#[test]
fn a_sync_of_a_different_catalog_drops_the_old_rows() {
    let store = store();
    store.begin_index_sync(SOURCE, BASE).unwrap();
    store
        .store_index_page(SOURCE, &[row("a/one", "one", "")], None)
        .unwrap();
    store.finish_index_sync(SOURCE).unwrap();

    store
        .begin_index_sync(SOURCE, "https://elsewhere.test")
        .unwrap();

    let state = store.index_state(SOURCE).unwrap().unwrap();
    assert_eq!(state.base_url, "https://elsewhere.test");
    assert_eq!(state.synced_at, None);
    assert_eq!(
        names(&store.search_index(SOURCE, &[]).unwrap()),
        Vec::<&str>::new()
    );
}

#[test]
fn a_search_needs_every_term_in_the_label_or_the_description() {
    let store = store();
    store.begin_index_sync(SOURCE, BASE).unwrap();
    store
        .store_index_page(
            SOURCE,
            &[
                row("a/slack", "a/slack slack", "post messages"),
                row("a/chat", "a/chat chat", "a slack bridge"),
                row("a/mail", "a/mail mail", "send email"),
            ],
            None,
        )
        .unwrap();

    let hits = store.search_index(SOURCE, &terms(&["slack"])).unwrap();
    assert_eq!(names(&hits), ["a/chat", "a/slack"]);

    let in_label: Vec<bool> = {
        let mut hits = hits;
        hits.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));
        hits.iter().map(|hit| hit.in_label).collect()
    };
    assert_eq!(in_label, [false, true]);

    let both = store
        .search_index(SOURCE, &terms(&["slack", "messages"]))
        .unwrap();
    assert_eq!(names(&both), ["a/slack"]);
}

#[test]
fn a_search_is_scoped_to_its_source() {
    let store = store();
    store.begin_index_sync("other", BASE).unwrap();
    store
        .store_index_page("other", &[row("a/one", "one", "")], None)
        .unwrap();

    assert_eq!(
        names(&store.search_index(SOURCE, &terms(&["one"])).unwrap()),
        Vec::<&str>::new()
    );
}
