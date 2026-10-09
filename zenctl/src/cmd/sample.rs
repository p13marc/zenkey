//! Sample rendering shared by the streaming (`echo`) and fan-in
//! (`get`) verbs (#114): one payload through the lens ([`Line`]), the
//! `--fmt` % vocabulary, and the hex form. One vocabulary, wherever a
//! payload is printed.
//!
//! Since #612's FJ9 both verbs resolve a key the same way — the lens
//! (`zenkey_fleet::Lens`): the deployment's namespace, its presence read,
//! the contract each descriptor names — and v1's decode ladder (a served
//! schema per registered subject) is gone with the v1 registry.

use zenkey_fleet::model::render::Member;
use zenkey_fleet::report::{
    Conformance, KeyGroup, KeyIdentity, PayloadRendering, Rendered, SampleRow, Unresolved,
};

/// One payload through the lens, once, for whichever medium prints it —
/// `echo`'s sample and `get`'s reply alike.
pub struct Line {
    pub identity: KeyIdentity,
    pub bytes: Vec<u8>,
    /// `None` on a deletion, which carries nothing to decode (#115).
    pub checked: Option<(PayloadRendering, Conformance)>,
    pub attachment: Option<PayloadRendering>,
}

/// What one payload is, for [`Line::of`].
pub struct Payload<'a> {
    /// The wire key, as it arrived.
    pub key: &'a str,
    /// The sample's `Encoding`, when it carried one.
    pub encoding: Option<&'a str>,
    pub bytes: Vec<u8>,
    pub attachment: Option<&'a [u8]>,
    /// A tombstone: no payload to decode.
    pub delete: bool,
}

impl Line {
    /// `member` is what the payload is of its resource: a sample's `Type`,
    /// or an operation's `Response` when a GET's reply answered one.
    /// `structural_only` (`--no-decode`, `--raw`) asks nothing of the lens
    /// beyond the key's identity, and every payload says so.
    pub fn of(
        p: Payload<'_>,
        member: Member,
        lens: &zenkey_fleet::Lens<'_>,
        structural_only: bool,
    ) -> Line {
        let Payload {
            key,
            encoding,
            bytes,
            attachment,
            delete,
        } = p;
        if structural_only {
            return Line {
                identity: lens.identity(key),
                checked: (!delete).then(|| {
                    (
                        PayloadRendering {
                            key: key.to_owned(),
                            size: bytes.len(),
                            resource: None,
                            rendered: Rendered::Structural {
                                why: Unresolved::DecodeNotAsked,
                                value: zenkey_fleet::structural_value(&bytes),
                                text: zenkey_fleet::structural(&bytes),
                            },
                        },
                        Conformance::NotChecked {
                            reason: Unresolved::DecodeNotAsked.words(),
                        },
                    )
                }),
                attachment: None,
                bytes,
            };
        }
        if delete {
            return Line {
                identity: lens.identity(key),
                checked: None,
                attachment: None,
                bytes,
            };
        }
        let c = lens.check(key, member, encoding, &bytes);
        let attachment = attachment.map(|a| lens.render(key, Member::Attachment, None, a));
        Line {
            identity: c.identity,
            checked: Some((c.rendering, c.conformance)),
            attachment,
            bytes,
        }
    }

    /// The member a payload on `key` is of: an operation's `Response` on an
    /// `@op` key (a GET's reply answered a call), its `Type` otherwise.
    pub fn member_of(identity: &KeyIdentity) -> Member {
        match &identity.group {
            KeyGroup::Resource { token, .. } if token == "@op" => Member::Response,
            _ => Member::Type,
        }
    }

    /// The declared type, when the ladder reached one.
    pub fn declared(&self) -> Option<String> {
        match &self.checked.as_ref()?.0.rendered {
            Rendered::Value { declared, .. } | Rendered::Undecodable { declared, .. } => {
                Some(declared.clone())
            }
            Rendered::Opaque { media_type } => Some(media_type.clone()),
            Rendered::Structural { .. } => None,
        }
    }

    /// The payload as a JSON value: decoded, or the structural document,
    /// or its text.
    pub fn value(&self) -> Option<serde_json::Value> {
        let (r, _) = self.checked.as_ref()?;
        Some(match &r.rendered {
            Rendered::Value { value, .. } => value.clone(),
            Rendered::Structural { value: Some(v), .. } => v.clone(),
            Rendered::Structural { text, .. } => serde_json::Value::String(text.clone()),
            Rendered::Opaque { media_type } => {
                serde_json::Value::String(format!("<{media_type}, {} B>", r.size))
            }
            Rendered::Undecodable { .. } => {
                serde_json::Value::String(zenkey_fleet::structural(&self.bytes))
            }
        })
    }

    /// The value as one line of text, for `--fmt`'s `%v`.
    pub fn value_text(&self) -> String {
        match self.value() {
            Some(serde_json::Value::String(s)) => s,
            Some(v) => v.to_string(),
            None => String::new(),
        }
    }

    /// The row-dialect fields this line holds: the key's identity, the
    /// declared type, the value and its conformance, the attachment. What
    /// only a subscription carries (QoS axes, SourceInfo, the tombstone
    /// flag) is the caller's to add.
    pub fn fill(&self, row: &mut SampleRow, attachment: Option<&[u8]>) {
        row.identity = Some(self.identity.clone());
        if let Some(a) = attachment {
            row.attachment = Some(match self.attachment.as_ref().map(|r| &r.rendered) {
                Some(Rendered::Value { value, .. }) => value.clone(),
                _ => attachment_json(a),
            });
            row.attachment_bytes = Some(a.len());
        }
        let Some((rendering, conformance)) = &self.checked else {
            // A tombstone: no value, no byte count — "0 bytes" would read
            // as an empty put, which it is not.
            return;
        };
        row.type_name = self.declared();
        row.typed = Some(matches!(rendering.rendered, Rendered::Value { .. }));
        row.payload_bytes = Some(self.bytes.len());
        row.value = self.value();
        row.verdict = Some(conformance.token());
        match conformance {
            Conformance::Invalid { violations } => row.violations = Some(violations.clone()),
            Conformance::Undecodable { reason, .. } => row.decode_error = Some(reason.clone()),
            Conformance::Valid | Conformance::NotChecked { .. } => {}
        }
    }

    /// The type tag where no rendering is printed (`--hex`): the declared
    /// type, or the rung the ladder stopped at.
    pub fn tag(&self) -> String {
        self.declared()
            .map(|d| format!("<{d}>"))
            .unwrap_or_else(|| format!("<{}>", identity_words(&self.identity)))
    }
}

/// An identity in a few words, for a tag where no type was reached.
pub fn identity_words(id: &KeyIdentity) -> String {
    match &id.unresolved {
        Some(why) => why.words(),
        None => id.group.label(),
    }
}

/// Everything one `--fmt` line can name (#354).
///
/// Twelve positional parameters, seven of them `&str` or `Option<&str>`, is
/// twelve chances to file a sample's encoding as its value — and every one of
/// them compiles. Named, they cannot be transposed.
#[derive(Debug, Clone, Copy)]
pub struct SampleLine<'a> {
    /// The line counter `%n` prints.
    pub n: usize,
    /// The full wire key, namespace included.
    pub wire_key: &'a str,
    /// The key relative to the stated namespace, `%K`; `None` outside it.
    pub relative: Option<&'a str>,
    /// What the lens made of the key: `%A`, `%i` and `%r` expand from it.
    pub identity: &'a KeyIdentity,
    /// The declared payload type, when the ladder reached one (`%t`).
    pub type_name: Option<&'a str>,
    pub encoding: &'a str,
    pub payload_len: usize,
    /// The arrival stamp — a subscribe-path fact; `None` on a reply (#120).
    pub timestamp: Option<&'a str>,
    /// The rendered payload.
    pub value: &'a str,
    /// The already-rendered attachment text, or `None` when there was none:
    /// `%a` expands to an empty field then, so the line shape stays stable
    /// for cut/awk, like `%{path}`.
    pub attachment: Option<&'a str>,
    /// The QoS axes — a subscribe-path fact; `None` on a reply (#120).
    pub qos: Option<&'a str>,
    pub source: Option<&'a str>,
}

/// One `--fmt` line. An unknown `%x` prints as typed — v1's `%o %c %p %s`
/// among them since #612's FJ9 — and a field the key does not carry is an
/// empty field.
pub fn format_sample(fmt: &str, s: &SampleLine<'_>) -> String {
    let SampleLine {
        n,
        wire_key,
        relative,
        identity,
        type_name,
        encoding,
        payload_len,
        timestamp,
        value,
        attachment,
        qos,
        source,
    } = *s;
    let group = &identity.group;
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
            Some('K') => out.push_str(relative.unwrap_or(wire_key)),
            Some('A') => {
                if let Some(a) = group.address() {
                    out.push_str(a);
                }
            }
            Some('i') => {
                if let KeyGroup::Resource { iface, .. } = group {
                    out.push_str(iface);
                }
            }
            Some('r') => {
                if let KeyGroup::Resource {
                    resource: Some(r), ..
                } = group
                {
                    out.push_str(r);
                }
            }
            Some('t') => out.push_str(type_name.unwrap_or("")),
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
/// tagged with its size: what a key no contract reaches gets (#117).
pub fn attachment_display(bytes: &[u8]) -> String {
    format!(
        "{} ({} bytes)",
        zenkey_fleet::structural(bytes),
        bytes.len()
    )
}

/// The ndjson half: the attachment as a JSON value (parsed when it is JSON,
/// a string otherwise). Callers add the key only when a wire attachment
/// exists — present-only-when-present, never null-when-absent.
pub fn attachment_json(bytes: &[u8]) -> serde_json::Value {
    zenkey_fleet::structural_value(bytes)
        .unwrap_or_else(|| serde_json::Value::String(zenkey_fleet::structural(bytes)))
}

/// The wire's QoS axes as one stable token (#120) — the engine's spelling.
///
/// A thin re-export, kept because `--fmt %q` and the sample renderer both
/// reach for it by this name. The fifteen literals live in
/// [`zenkey_fleet::report::qos_axes_token`], beside the `SampleRow.qos_axes`
/// field they are the round-trip contract for (#353).
pub fn qos_summary(
    priority: zenoh::qos::Priority,
    congestion_control: zenoh::qos::CongestionControl,
    reliability: zenoh::qos::Reliability,
    express: bool,
) -> String {
    zenkey_fleet::report::qos_axes_token(priority, congestion_control, reliability, express)
}

/// The publishing entity, when SourceInfo rode the sample: `zid:eid#sn`.
pub fn source_summary(source: &zenkey_fleet::SampleSource) -> String {
    format!("{}:{}#{}", source.zid, source.eid, source.sn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn foreign() -> KeyIdentity {
        KeyIdentity {
            group: KeyGroup::NotZk2,
            values: Default::default(),
            unresolved: None,
        }
    }

    fn line<'a>(fmt_value: &'a str, identity: &'a KeyIdentity) -> SampleLine<'a> {
        SampleLine {
            n: 1,
            wire_key: "k",
            relative: None,
            identity,
            type_name: None,
            encoding: "e",
            payload_len: 0,
            timestamp: None,
            value: fmt_value,
            attachment: None,
            qos: None,
            source: None,
        }
    }

    #[test]
    fn format_sample_extracts_payload_fields() {
        let id = foreign();
        let l = SampleLine {
            encoding: "application/json",
            payload_len: 2,
            ..line(r#"{"iface":{"name":"eth0","up":true}}"#, &id)
        };
        assert_eq!(
            format_sample("%{iface.name} up=%{iface.up} missing=[%{no.such}]", &l),
            "eth0 up=true missing=[]"
        );
    }

    /// The zk2 positions expand from the key's identity, and v1's
    /// `%o %c %p %s` print as typed since FJ9 (#612).
    #[test]
    fn format_sample_expands_fields() {
        let id = KeyIdentity {
            group: KeyGroup::Resource {
                address: "host-a/tc".into(),
                iface: "tc.netif.v1".into(),
                token: "state".into(),
                resource: Some("state/interfaces/{ns}/{iface}".into()),
            },
            values: Default::default(),
            unresolved: None,
        };
        let key = "prod/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";
        let l = SampleLine {
            n: 3,
            wire_key: key,
            relative: Some("zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0"),
            type_name: Some("json:NetworkInterface"),
            encoding: "application/json",
            payload_len: 12,
            ..line(r#"{"up":true}"#, &id)
        };
        assert_eq!(
            format_sample("%n %A %i %r <%t> %v (%l B, %e)", &l),
            r#"3 host-a/tc tc.netif.v1 state/interfaces/{ns}/{iface} <json:NetworkInterface> {"up":true} (12 B, application/json)"#
        );
        assert_eq!(
            format_sample("%%|%K\\t.", &l),
            "%|zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0\t."
        );
        assert_eq!(format_sample("%o%c%p%s", &l), "%o%c%p%s");
        // A key no contract reached: the positions are empty fields, and
        // `%K` outside the namespace is the wire key.
        let id = foreign();
        assert_eq!(
            format_sample("[%A][%i][%r][%t]|%K", &line("v", &id)),
            "[][][][]|k"
        );
    }

    /// `%a` is the attachment field: the rendered text when one arrived, an
    /// empty field when none did — the line shape stays stable (#117).
    #[test]
    fn the_attachment_field_is_empty_when_absent() {
        let id = foreign();
        let with = |att: Option<&str>| {
            format_sample(
                "%v|%a",
                &SampleLine {
                    attachment: att,
                    ..line("v", &id)
                },
            )
        };
        assert_eq!(with(Some("meta (4 bytes)")), "v|meta (4 bytes)");
        assert_eq!(with(None), "v|");
    }

    /// The ndjson attachment value parses JSON and falls back to the
    /// structural string; the display form is size-tagged.
    #[test]
    fn attachments_render_structurally_and_stop_there() {
        let json = br#"{"who":"me"}"#;
        assert_eq!(attachment_json(json), serde_json::json!({"who": "me"}));
        assert_eq!(attachment_display(json), r#"{"who":"me"} (12 bytes)"#);
        assert_eq!(attachment_json(b"plain"), serde_json::json!("plain"));
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
        let id = foreign();
        let l = SampleLine {
            qos: Some("data/drop/reliable"),
            ..line("v", &id)
        };
        assert_eq!(format_sample("%q|%S", &l), "data/drop/reliable|");
    }

    /// A GET's reply on an `@op` key answered a call: its payload is the
    /// operation's response, every other reply its resource's type.
    #[test]
    fn a_reply_on_an_op_key_is_a_response() {
        let on = |token: &str| KeyIdentity {
            group: KeyGroup::Resource {
                address: "host-a/tc".into(),
                iface: "tc.netif.v1".into(),
                token: token.into(),
                resource: None,
            },
            values: Default::default(),
            unresolved: None,
        };
        assert_eq!(Line::member_of(&on("@op")), Member::Response);
        assert_eq!(Line::member_of(&on("state")), Member::Type);
        assert_eq!(Line::member_of(&foreign()), Member::Type);
    }
}
