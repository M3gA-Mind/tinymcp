//! Differential tests for the hand-written URL scanner, against the regex it
//! replaced: `(?i)\b([a-z][a-z0-9+.\-]{1,31})://[^\s"'<>\x60{}|\\^\[\]]+`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn regex_spans(haystack: &str) -> Vec<&str> {
    regex::Regex::new(r#"(?i)\b([a-z][a-z0-9+.\-]{1,31})://[^\s"'<>\x60{}|\\^\[\]]+"#)
        .unwrap()
        .find_iter(haystack)
        .map(|found| found.as_str())
        .collect()
}

const HAYSTACKS: &[&str] = &[
    "",
    "no links here",
    "Pay at https://pay.example.com/o/1 or phonepe://pay?id=1 or upi://pay?pa=a@b&am=10.",
    "javascript://x%0Aalert(1) data://text/html,hi file:///etc/passwd tauri://localhost",
    "[pay](https://pay.example.com/x) (see https://a.example.com/p_(1)).",
    "\"https://q.com/a\"'https://r.com/b'<https://s.com/c>`https://t.com/d`",
    "{https://u.com}|https://v.com|^https://w.com^\\https://x.com\\",
    "1http://a.com _http://b.com ahttp://c.com x.https://d.com x-https://e.com x+f://g",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.https://long.example.com",
    "abcdefghijklmnopqrstuvwxyzabcdef://exactly-32 abcdefghijklmnopqrstuvwxyzabcdefg://33",
    "a://one-letter ab://two-letters",
    "https:// empty https://\tx https://\u{a0}nbsp https://\u{3000}ideographic",
    "éhttps://after-accent 日本https://after-cjk ١https://after-arabic-digit",
    "—https://after-dash «https://after-guillemet» (https://in-parens)",
    "HTTPS://UPPER.EXAMPLE.COM/PATH mailto://x@y.com",
    "https://a.com/ü/日本?q=ü#ü https://b.com/emoji/😀",
    "https://a.com:https://b.com https://c.com//https://d.com",
    "x://y://z http:/single http//no-colon",
];

#[test]
fn finds_the_same_spans_as_the_regex() {
    for haystack in HAYSTACKS {
        assert_eq!(
            url_spans(haystack),
            regex_spans(haystack),
            "{haystack:?}: the scanner and the regex disagree"
        );
    }
}

#[test]
fn finds_spans_in_generated_text() {
    let pieces = [
        "https://a.com",
        "x",
        " ",
        ".",
        "_",
        "é",
        "://",
        "ab",
        "\"",
        "1",
        "-",
        "+",
        "\n",
        "upi://pay?pa=a@b",
        "(",
        ")",
        "日",
    ];
    for a in pieces {
        for b in pieces {
            for c in pieces {
                let haystack = format!("{a}{b}{c}");
                assert_eq!(
                    url_spans(&haystack),
                    regex_spans(&haystack),
                    "{haystack:?}: the scanner and the regex disagree"
                );
            }
        }
    }
}
