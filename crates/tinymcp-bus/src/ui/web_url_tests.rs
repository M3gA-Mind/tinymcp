//! Differential tests for the hand-written `http(s)` splitter, against the
//! `url` crate's WHATWG parser.
//!
//! The property: whenever link classification calls a URL [`LinkClass::Web`]
//! or [`LinkClass::Image`], `url::Url::parse` accepts it, reports the same
//! host, and reaches the same image verdict. A shape the splitter is unsure
//! of must come back blocked.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::ui::links::image_path;
use crate::ui::{LinkClass, classify_link};

/// Asserts the property for one input. Returns whether the splitter
/// accepted it.
fn agrees_with_url(input: &str) -> bool {
    let class = classify_link(input);
    let split = WebUrl::split(input);
    if !matches!(class, LinkClass::Web | LinkClass::Image) {
        return split.is_some();
    }
    let web = split.unwrap_or_else(|| panic!("{input:?}: classified {class:?} without a split"));
    let parsed = url::Url::parse(input)
        .unwrap_or_else(|error| panic!("{input:?}: accepted, but url refuses it: {error}"));
    assert!(
        matches!(parsed.scheme(), "http" | "https"),
        "{input:?}: accepted a non-web scheme"
    );
    assert_eq!(
        parsed.host_str(),
        Some(web.host.as_str()),
        "{input:?}: the splitter and url disagree on the host"
    );
    assert_eq!(
        class == LinkClass::Image,
        image_path(parsed.path()),
        "{input:?}: the splitter and url disagree on the image verdict"
    );
    true
}

/// Hostile and unusual shapes, each of which the splitter must refuse.
const REFUSED: &[&str] = &[
    // userinfo
    "https://a@b",
    "https://a@b.com/",
    "https://user:pass@b.com/",
    "https://b.com@evil.com/",
    "https://@b.com/",
    // backslashes
    "https:\\\\a.com",
    "https://a.com\\@evil.com",
    "https://a.com/x\\y.png",
    // tabs, newlines and spaces
    "https://a\t.com",
    "https://a.co\nm/",
    "https://a.com/\r\n",
    "https://a .com",
    " https://a.com",
    // IDN and punycode
    "https://bücher.de/",
    "https://xn--bcher-kva.de/",
    "https://XN--BCHER-KVA.DE/",
    "https://a.xn--p1ai/",
    "https://ab--cd.com/",
    "https://аpple.com/",
    // percent-encoded hosts
    "https://%61.com/",
    "https://a%2ecom/",
    "https://a.com%2f@evil.com/",
    // IPv4 in non-canonical forms
    "https://0x7f.0.0.1/",
    "https://0x7f000001/",
    "https://2130706433/",
    "https://127.1/",
    "https://127.0.1/",
    "https://010.0.0.1/",
    "https://127.0.0.01/",
    "https://256.0.0.1/",
    "https://1.2.3.4.5/",
    "https://a.123/",
    "https://a.0x1/",
    // IPv6, in any form
    "https://[::1]/",
    "https://[0:0::1]/",
    "https://[::ffff:127.0.0.1]/",
    "https://[fe80::1%25eth0]/",
    "https://[::1]:8080/",
    // ports
    "https://a.com:65536/",
    "https://a.com:99999/",
    "https://a.com:123456/",
    "https://a.com:/",
    "https://a.com:-1/",
    "https://a.com:+80/",
    "https://a.com:8o/",
    "https://a.com:80:80/",
    // empty and malformed hosts
    "https://",
    "https:///a.com",
    "https:/a.com",
    "https:a.com",
    "https://.com/",
    "https://a..com/",
    "https://a.com./",
    "https://-a.com/",
    "https://a-.com/",
    "https://a_b.com/",
    "https://a!b.com/",
    // dot segments
    "https://a.com/../b.png",
    "https://a.com/x/./b",
    "https://a.com/%2e%2e/b.png",
    "https://a.com/.%2E/b",
    "https://a.com/x/%2E",
    // not web schemes at all
    "javascript:alert(1)",
    "JAVASCRIPT://a.com/%0Aalert(1)",
    "data:text/html,<script>x</script>",
    "data://a.com/x",
    "ftp://a.com/x.png",
    "file:///etc/passwd",
    "httpx://a.com/",
    "nonsense",
];

/// Plain shapes the splitter must accept, so the property is not vacuous.
const ACCEPTED: &[&str] = &[
    "https://a.com",
    "http://a.com/x",
    "HTTPS://A.COM/X",
    "HtTpS://Pay.Example.com/o/1?id=2#top",
    "https://a.com:443/a.png?x=1#y",
    "http://127.0.0.1:8080/p",
    "http://0.0.0.0/",
    "https://255.255.255.255/",
    "http://localhost/",
    "https://1e.com/",
    "https://0x7f.com/",
    "https://a-b.c-d.com/image/upload/x",
    "https://a.com:08/x.JPG",
    "https://a.com/p_(1)",
    "https://a.com/%20x",
    "https://a.com/ünï.png",
];

#[test]
fn refuses_every_hostile_shape() {
    for input in REFUSED {
        assert!(
            WebUrl::split(input).is_none(),
            "{input:?}: the splitter accepted a shape it must refuse"
        );
        assert!(
            !matches!(classify_link(input), LinkClass::Web | LinkClass::Image),
            "{input:?}: classified as a web link"
        );
        agrees_with_url(input);
    }
}

#[test]
fn accepts_plain_web_urls_exactly_as_url_reads_them() {
    for input in ACCEPTED {
        assert!(
            agrees_with_url(input),
            "{input:?}: the splitter refused a plain URL"
        );
    }
}

#[test]
fn splits_host_and_path() {
    let web = WebUrl::split("HTTPS://Pay.Example.COM:8443/O/1.png?x=a.png#b").unwrap();
    assert_eq!(web.host, "pay.example.com");
    assert_eq!(web.path, "/O/1.png");
    assert_eq!(WebUrl::split("https://a.com?x").unwrap().path, "");
    assert_eq!(WebUrl::split("https://a.com#x").unwrap().path, "");
}

#[test]
fn every_combination_agrees_with_url() {
    let schemes = ["http", "https", "HTTPS", "hTtP"];
    let hosts = [
        "a.com",
        "A.Com",
        "x.y.z.example",
        "localhost",
        "127.0.0.1",
        "1.2.3.4",
        "256.0.0.1",
        "1.2.3",
        "0x7f.0.0.1",
        "01.2.3.4",
        "[::1]",
        "xn--nxasmq6b.com",
        "a_b.com",
        "a%2eb.com",
        "a.com.",
        "1e.com",
        "a.0x1",
        "a.1",
        "u@a.com",
        "",
    ];
    let ports = ["", ":80", ":0", ":08", ":65535", ":65536", ":", ":x"];
    let paths = [
        "",
        "/",
        "/a.PNG",
        "/a.png/",
        "/a/../b.png",
        "/a.png/..",
        "/%2E/x.jpg",
        "/image/upload/x",
        "/IMAGE/UPLOAD/x",
        "/x?y.png",
        "/x#a.png",
        "?q",
        "#f",
        "/a.b/c",
    ];
    let mut accepted = 0;
    for scheme in schemes {
        for host in hosts {
            for port in ports {
                for path in paths {
                    if agrees_with_url(&format!("{scheme}://{host}{port}{path}")) {
                        accepted += 1;
                    }
                }
            }
        }
    }
    assert!(accepted > 0, "no combination was accepted");
}
