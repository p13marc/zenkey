//! Sample rendering shared by the streaming (`topic echo`) and fan-in
//! (`get`) verbs (#114): the `--fmt` % vocabulary, the hex form, and the
//! type tag. One vocabulary, wherever a payload is printed.

/// One decoded/rendered sample line for `--fmt`.
#[allow(clippy::too_many_arguments)]
pub fn format_sample(
    fmt: &str,
    n: usize,
    wire_key: &str,
    base: &str,
    type_name: Option<&str>,
    encoding: &str,
    payload_len: usize,
    timestamp: Option<&str>,
    value: &str,
) -> String {
    let parsed = zenkey::grammar::parse_full(base, wire_key);
    let mut out = String::with_capacity(fmt.len() + value.len());
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
            continue;
        }
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('{') => {
                // %{a.b.c}: a decoded payload field by dot-path. Unresolvable
                // paths render as an empty field, honestly (the line shape
                // stays stable for cut/awk).
                let mut path = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    path.push(c);
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(value) {
                    let mut cur = &v;
                    let mut ok = true;
                    for seg in path.split('.') {
                        match cur.get(seg) {
                            Some(next) => cur = next,
                            None => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        match cur {
                            serde_json::Value::String(s) => out.push_str(s),
                            other => out.push_str(&other.to_string()),
                        }
                    }
                }
            }
            Some('k') => out.push_str(wire_key),
            Some('K') => {
                out.push_str(zenkey::grammar::strip_base(base, wire_key).unwrap_or(wire_key))
            }
            Some('o') => {
                if let Some(p) = &parsed {
                    out.push_str(p.origin.chunk());
                }
            }
            Some('c') => {
                if let Some(p) = &parsed {
                    out.push_str(match &p.class {
                        zenkey::grammar::ClassOrPlane::Class(c) => c.chunk(),
                        zenkey::grammar::ClassOrPlane::Plane(pl) => pl.chunk(),
                    });
                }
            }
            Some('p') => {
                if let Some(name) = parsed.as_ref().and_then(|p| p.producer.as_ref()) {
                    out.push_str(name.name());
                }
            }
            Some('s') => {
                if let Some(p) = &parsed {
                    out.push_str(&p.subject.join("/"));
                }
            }
            Some('t') => out.push_str(type_name.unwrap_or("unregistered")),
            Some('v') => out.push_str(value),
            Some('e') => out.push_str(encoding),
            Some('l') => {
                use std::fmt::Write as _;
                let _ = write!(out, "{payload_len}");
            }
            Some('n') => {
                use std::fmt::Write as _;
                let _ = write!(out, "{n}");
            }
            Some('T') => out.push_str(timestamp.unwrap_or("-")),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// Space-separated lowercase hex — the byte-honest form.
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The type tag: `<T>` schema-decoded, `<T?>` registered but rendered
/// structurally, `<unregistered>` when no slice names the key.
pub fn type_tag(type_name: Option<&str>, typed: bool) -> String {
    match (type_name, typed) {
        (Some(t), true) => format!("<{t}>"),
        (Some(t), false) => format!("<{t}?>"),
        (None, _) => "<unregistered>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_sample_extracts_payload_fields() {
        let line = format_sample(
            "%{iface.name} up=%{iface.up} missing=[%{no.such}]",
            1,
            "k",
            "",
            None,
            "application/json",
            2,
            None,
            r#"{"iface":{"name":"eth0","up":true}}"#,
        );
        assert_eq!(line, "eth0 up=true missing=[]");
    }

    #[test]
    fn format_sample_expands_fields() {
        let line = format_sample(
            "%n %o %c/%p %s <%t> %v (%l B, %e)",
            3,
            "tcgui/v1/h-3fa9c2d41b7e/state/tc/iface/eth0/state",
            "tcgui",
            Some("NetworkInterface"),
            "application/json",
            12,
            None,
            r#"{"up":true}"#,
        );
        assert_eq!(
            line,
            r#"3 h-3fa9c2d41b7e state/tc iface/eth0/state <NetworkInterface> {"up":true} (12 B, application/json)"#
        );
        // Escapes and literals.
        assert_eq!(
            format_sample(
                "%%|%K\\t.",
                1,
                "b/v1/h-3fa9c2d41b7e/state/tc/x",
                "b",
                None,
                "e",
                0,
                None,
                "v"
            ),
            "%|v1/h-3fa9c2d41b7e/state/tc/x\t."
        );
    }

    /// The three-rung ladder every payload print shares.
    #[test]
    fn the_type_tag_ladder_has_three_rungs() {
        assert_eq!(type_tag(Some("T"), true), "<T>");
        assert_eq!(type_tag(Some("T"), false), "<T?>");
        assert_eq!(type_tag(None, true), "<unregistered>");
        assert_eq!(type_tag(None, false), "<unregistered>");
    }
}
