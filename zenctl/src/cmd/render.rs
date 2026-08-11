//! Sample rendering shared by the streaming (`topic echo`) and fan-in
//! (`get`) verbs (#114): the `--fmt` % vocabulary, the hex form, and the
//! type tag. One vocabulary, wherever a payload is printed.

/// One decoded/rendered sample line for `--fmt`.
///
/// `attachment` is the already-rendered attachment text; `%a` expands to it,
/// or to an empty field when the sample carried none (the line shape stays
/// stable for cut/awk, like `%{path}`).
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
    attachment: Option<&str>,
    qos: Option<&str>,
    source: Option<&str>,
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
            Some('a') => out.push_str(attachment.unwrap_or("")),
            Some('q') => out.push_str(qos.unwrap_or("")),
            Some('S') => out.push_str(source.unwrap_or("")),
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

/// An attachment, rendered structurally (JSON → CBOR → text → hex) and
/// tagged with its size. Deliberately **no schema decode**: the registry's
/// vocabulary ends at the payload, so an attachment renders structurally or
/// as hex, labeled, and that is where it stops (#117).
pub fn attachment_display(att: &zenoh::bytes::ZBytes) -> String {
    let bytes = att.to_bytes();
    format!(
        "{} ({} bytes)",
        zenkey_fleet::decode::structural(&bytes),
        bytes.len()
    )
}

/// The ndjson half: the attachment as a JSON value (parsed when it is JSON,
/// a string otherwise). Callers add the key only when a wire attachment
/// exists — present-only-when-present, never null-when-absent.
pub fn attachment_json(att: &zenoh::bytes::ZBytes) -> serde_json::Value {
    let bytes = att.to_bytes();
    zenkey_fleet::decode::structural_value(&bytes)
        .unwrap_or_else(|| serde_json::Value::String(zenkey_fleet::decode::structural(&bytes)))
}

/// The wire's QoS axes as one stable token (#120):
/// `priority/congestion/reliability`, `+express` when set — lowercase,
/// cut/awk-friendly, never Debug formatting.
pub fn qos_summary(
    priority: zenoh::qos::Priority,
    congestion_control: zenoh::qos::CongestionControl,
    reliability: zenoh::qos::Reliability,
    express: bool,
) -> String {
    use zenoh::qos::{CongestionControl as Cc, Priority as P, Reliability as R};
    let p = match priority {
        P::RealTime => "real_time",
        P::InteractiveHigh => "interactive_high",
        P::InteractiveLow => "interactive_low",
        P::DataHigh => "data_high",
        P::Data => "data",
        P::DataLow => "data_low",
        P::Background => "background",
    };
    let c = match congestion_control {
        Cc::Drop => "drop",
        Cc::Block => "block",
        _ => "other",
    };
    let r = match reliability {
        R::BestEffort => "best_effort",
        R::Reliable => "reliable",
    };
    format!("{p}/{c}/{r}{}", if express { "+express" } else { "" })
}

/// The publishing entity, when SourceInfo rode the sample: `zid:eid#sn`.
pub fn source_summary(source: &zenkey_fleet::SampleSource) -> String {
    format!("{}:{}#{}", source.zid, source.eid, source.sn)
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
            None,
            None,
            None,
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
            None,
            None,
            None,
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
                "v",
                None,
                None,
                None,
            ),
            "%|v1/h-3fa9c2d41b7e/state/tc/x\t."
        );
    }

    /// `%a` is the attachment field: the rendered text when one arrived, an
    /// empty field when none did — the line shape stays stable (#117).
    #[test]
    fn the_attachment_field_is_empty_when_absent() {
        let line = |att: Option<&str>| {
            format_sample(
                "%v|%a", 1, "k", "", None, "e", 0, None, "v", att, None, None,
            )
        };
        assert_eq!(line(Some("meta (4 bytes)")), "v|meta (4 bytes)");
        assert_eq!(line(None), "v|");
    }

    /// The ndjson attachment value parses JSON and falls back to the
    /// structural string; the display form is size-tagged.
    #[test]
    fn attachments_render_structurally_and_stop_there() {
        let json = zenoh::bytes::ZBytes::from(r#"{"who":"me"}"#);
        assert_eq!(attachment_json(&json), serde_json::json!({"who": "me"}));
        assert_eq!(attachment_display(&json), r#"{"who":"me"} (12 bytes)"#);
        let text = zenoh::bytes::ZBytes::from("plain");
        assert_eq!(attachment_json(&text), serde_json::json!("plain"));
    }

    /// #120: the QoS token is stable and lowercase, never Debug output —
    /// and %q/%S render empty when the fact was not carried.
    #[test]
    fn the_qos_token_is_stable_and_the_fields_degrade_empty() {
        use zenoh::qos::{CongestionControl, Priority, Reliability};
        assert_eq!(
            qos_summary(
                Priority::DataHigh,
                CongestionControl::Block,
                Reliability::Reliable,
                true
            ),
            "data_high/block/reliable+express"
        );
        assert_eq!(
            qos_summary(
                Priority::Data,
                CongestionControl::Drop,
                Reliability::BestEffort,
                false
            ),
            "data/drop/best_effort"
        );
        let line = format_sample(
            "%q|%S",
            1,
            "k",
            "",
            None,
            "e",
            0,
            None,
            "v",
            None,
            Some("data/drop/reliable"),
            None,
        );
        assert_eq!(line, "data/drop/reliable|");
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
