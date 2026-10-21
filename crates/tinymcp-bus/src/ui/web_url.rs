//! The host and path of an `http(s)` link, split by hand so the contract needs
//! no URL parser.
//!
//! Fail-closed: [`WebUrl::split`] accepts only plain shapes it splits exactly as
//! a WHATWG parser (the `url` crate) would, and refuses everything else, which
//! link classification treats as blocked. Refused even though a browser would
//! open them:
//!
//! - userinfo and backslashes;
//! - whitespace or control characters anywhere;
//! - hosts that are percent-encoded or non-ASCII (IDN), and punycode labels;
//! - bracketed IPv6, and IPv4 in any form other than canonical dotted decimal;
//! - an empty, non-numeric or out-of-range port;
//! - dot segments in the path, including percent-encoded ones.
//!
//! `web_url_tests.rs` checks every acceptance against `url`.

/// The longest host accepted, in bytes.
const MAX_HOST_LEN: usize = 253;
/// The longest host label accepted, in bytes.
const MAX_LABEL_LEN: usize = 63;

/// An `http` or `https` URL's host and path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WebUrl<'a> {
    /// The host, lowercased: what `url::Url::host_str` reports for it.
    pub(super) host: String,
    /// The path, without query or fragment; empty when the URL has none.
    pub(super) path: &'a str,
}

impl<'a> WebUrl<'a> {
    /// `url` split into host and path, or `None` when it is not an `http(s)`
    /// URL of a shape this splitter is certain of.
    pub(super) fn split(url: &'a str) -> Option<Self> {
        let (scheme, rest) = url.split_once("://")?;
        if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
            || url
                .chars()
                .any(|c| c == '\\' || c.is_control() || c.is_whitespace())
        {
            return None;
        }
        let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
        let path = &tail[..tail.find(['?', '#']).unwrap_or(tail.len())];
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        let host = host.to_ascii_lowercase();
        if port.is_some_and(|port| !valid_port(port))
            || !(valid_ipv4(&host) || valid_domain(&host))
            || has_dot_segment(path)
        {
            return None;
        }
        Some(Self { host, path })
    }
}

/// One to five digits naming a port no larger than 65535.
fn valid_port(port: &str) -> bool {
    (1..=5).contains(&port.len())
        && port.bytes().all(|b| b.is_ascii_digit())
        && port.parse::<u16>().is_ok()
}

/// Canonical dotted decimal: four labels of 0 to 255, no leading zeros.
fn valid_ipv4(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() == 4
        && labels.iter().all(|label| {
            (1..=3).contains(&label.len())
                && label.bytes().all(|b| b.is_ascii_digit())
                && (label.len() == 1 || !label.starts_with('0'))
                && label.parse::<u8>().is_ok()
        })
}

/// Lowercase ASCII labels of letters, digits and inner single hyphens.
///
/// A last label a WHATWG parser would read as a number (all digits, or `0x`)
/// is refused: that host is an IPv4 address, and only [`valid_ipv4`]'s form
/// is accepted. A `--` anywhere is refused, which covers `xn--` punycode.
fn valid_domain(host: &str) -> bool {
    let last = host.rsplit('.').next().unwrap_or_default();
    !host.is_empty()
        && host.len() <= MAX_HOST_LEN
        && !last.bytes().all(|b| b.is_ascii_digit())
        && !last.starts_with("0x")
        && host.split('.').all(|label| {
            (1..=MAX_LABEL_LEN).contains(&label.len())
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
                && !label.contains("--")
        })
}

/// Whether a path segment is `.` or `..`, literally or percent-encoded. A
/// WHATWG parser removes those, so the path it reports would differ.
fn has_dot_segment(path: &str) -> bool {
    path.split('/').any(|segment| {
        let decoded = segment.to_ascii_lowercase().replace("%2e", ".");
        decoded == "." || decoded == ".."
    })
}

#[cfg(test)]
#[path = "web_url_tests.rs"]
mod tests;
