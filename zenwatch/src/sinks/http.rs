//! The one HTTP client the ntfy and webhook sinks share.
//!
//! One client, one connection pool, one `User-Agent`; each sink applies its
//! own timeout per request. TLS is rustls on the ring provider zenwatch's
//! `main` installs — reqwest is built with `rustls-no-provider` precisely so
//! it cannot bring a second one (see the workspace manifest).

/// `zenwatch/<version>`.
pub const USER_AGENT: &str = concat!("zenwatch/", env!("CARGO_PKG_VERSION"));

/// The shared client. Building it cannot fail on a default configuration;
/// if it ever does, a client with no options is still a client.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .unwrap_or_default()
}

/// The first `max` bytes of a response body, for an error message — a
/// failing webhook that returns its whole HTML error page must not become
/// the log line.
pub fn head(body: &str, max: usize) -> String {
    let cut = body.trim();
    if cut.len() <= max {
        return cut.to_string();
    }
    let mut end = max;
    while end > 0 && !cut.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &cut[..end])
}

/// Read at most `max` bytes of a body as text.
pub async fn body_head(resp: reqwest::Response, max: usize) -> String {
    match resp.text().await {
        Ok(t) => head(&t, max),
        Err(_) => String::new(),
    }
}

/// A header-safe rendering of free text: control characters and non-ASCII
/// replaced, because an ntfy `Title` rides an HTTP header and a title with a
/// newline in it is a request that does not go out.
pub fn header_safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() {
                c
            } else {
                '?'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads_are_bounded_on_char_boundaries() {
        assert_eq!(head("  ok  ", 10), "ok");
        assert_eq!(head("éééé", 3), "é…");
        assert_eq!(header_safe("a\nb é"), "a?b ?");
    }
}
