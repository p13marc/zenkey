//! `zenctl get` — the fleet discipline on any selector (#114), and `get
//! state`, a zk2 state resource read through its contract (#612, FJ5).
//!
//! The two forms never share a positional: `get <SELECTOR>` is RAW — a wire
//! selector on a session in no namespace, any bus — and
//! `get state <ADDRESS> <IFACE> <STATE>` is RESOLVED, in the deployment's
//! namespace, through the runtime's consumer ([`state`]).
//!
//! A plain fan-in GET: target `All`, consolidation `None`, every reply
//! attributed by its own key (RFC 05 §2.1), error envelopes rendered as
//! errors — through `zenkey_fleet::fleet_get`, never the JSON-lossy admin
//! browse. Each reply is resolved through the same lens `echo` reads a
//! sample through (#612, FJ9): the deployment in `--namespace`, its
//! presence read, the contract each descriptor names. A zk2 key's payload
//! is decoded as its declared type and a JSON Schema value checked against
//! it; where the ladder stops, the rung is named; a foreign key renders
//! structurally. v1's served-schema ladder left with the v1 registry.
//!
//! A reply that is a delete — an owner's `reply_del` within its tombstone
//! window (core §4, S3) — is a deletion in every form (#765): the tombstone
//! words in the table, `--hex`, `--raw` and `--fmt`'s `%v`, and
//! `"delete": true` on its row, never a put with an empty payload.

use anyhow::Result;
use zenkey_model::authoring::Kind;

use super::sample::{
    Line, Payload, SampleLine, TOMBSTONE, attachment_display, attachment_json, format_sample, hex,
};
use crate::bus::Deployment;
use crate::cmd::zk2;
use zenkey_fleet::report::Conformance;
use zenkey_fleet::{Answer, FleetAnswer, Lens};

/// The reply discipline as an exit code: 0 = value replies only, 1 = at
/// least one error envelope, 2 = silence (which is still not a verdict —
/// the code is for scripts, the paragraph is for people).
fn exit_code(answers: &[FleetAnswer]) -> i32 {
    if answers.is_empty() {
        2
    } else if answers
        .iter()
        .any(|a| matches!(a.answer, Answer::Error { .. }))
    {
        1
    } else {
        0
    }
}

pub async fn run(cli: crate::cli::GetArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let crate::cli::GetArgs {
        selector,
        body,
        raw,
        hex: hex_payload,
        fmt,
        no_decode,
        cmd: _,
        contracts: _,
        ns: _,
    } = cli;
    // Clap requires it whenever `state` is not given.
    let selector = selector.ok_or_else(|| crate::exit::unaskable!("a selector is required"))?;
    let namespace = dep.namespace().to_owned();
    // The raw seam (`$*` never reaches the session, RFC 03 §2), and the
    // one-line hint for a base-relative `zk2/…` typed under a namespace.
    let selector = zk2::wire_selector(Some(&selector), &namespace)?;
    let payload = body.as_ref().map(crate::input::Source::read).transpose()?;

    // The lens before the GET: presence and the revisions it names, read in
    // the namespace — unless nothing is to be decoded, which asks nothing
    // of it.
    let store = zk2::store(&dep, &contracts);
    let structural_only = raw || no_decode;
    let catalog = if structural_only {
        None
    } else {
        let ns_session = dep.session().await?;
        zk2::read_lens(&dep, &ns_session, &store).await
    };
    let lens = Lens::new(&namespace, catalog.as_ref(), &store).offline(&contracts);

    // The GET itself, on a session in no namespace: the wire key as typed.
    let session = dep.link().session().await?;
    // Named, because the reply bound's *cost* rides on the options that state
    // it (#339): a GET that read only some of the replies must say so, or the
    // rendering claims a fan-in it did not have (RFC 13 §3 O6).
    let opts = zenkey_fleet::GetOpts::new(dep.timeout()).payload(payload);
    let answers = zenkey_fleet::fleet_get(&session, &selector, &opts).await?;
    let elided = opts.elided();

    let secs = dep.timeout().as_secs_f64();
    // A fan-in GET *looks* like a stream and is not: it waits for the window,
    // then has every answer in hand. So it is a document, and the one place
    // that decides which format to print it in is `emit` (#198).
    if crate::render::Mode::of(dep.format()).machine() {
        let rows = answers
            .iter()
            .map(|a| row(a, &lens, raw, structural_only))
            .collect();
        let report = crate::render::GetReport {
            selector: selector.clone(),
            timeout_s: secs,
            elided,
            answers: rows,
        };
        crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    } else {
        let human = Human {
            lens: &lens,
            namespace: &namespace,
            raw,
            hex: hex_payload,
            fmt: fmt.as_deref(),
            structural_only,
        };
        let mut n = 0usize;
        for a in &answers {
            if matches!(a.answer, Answer::Value(_)) {
                n += 1;
            }
            let printed = human.reply(a, n);
            for line in printed.out {
                println!("{line}");
            }
            for line in printed.err {
                eprintln!("{line}");
            }
        }
        match answers.len() {
            0 => println!(
                "no replies within {secs}s. Silence is not a verdict (RFC 05 §3.1): \
                 nothing may hold the selector, its holders may be down, a router's \
                 access control may have refused it (core §11.2: at once, complete and \
                 empty), or the timeout too short — `zenctl admin graph` says who is \
                 attached."
            ),
            len => eprintln!(
                "{len} repl{} within {secs}s — an observation, not totality \
                 (RFC 05 §2.1)",
                if len == 1 { "y" } else { "ies" }
            ),
        }
        if elided > 0 {
            eprintln!(
                "{elided} further repl(y|ies) arrived and were not read — the \
                 reply bound bit, so the above is a sample of the answers"
            );
        }
    }
    // Exit-code discipline shared with `call`: 1 = an error reply, 2 = zero
    // replies (silence stays a distinct non-verdict — RFC 05 §3.1).
    let code = exit_code(&answers);
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// One reply through the lens: a response on an `@op` key, its resource's
/// type otherwise. A delete carries nothing to decode (#765).
fn line_of(a: &FleetAnswer, bytes: Vec<u8>, lens: &Lens<'_>, structural_only: bool) -> Line {
    let member = Line::member_of(&lens.identity(&a.key));
    let attachment = a.attachment.as_ref().map(|t| t.to_bytes());
    Line::of(
        Payload {
            key: &a.key,
            encoding: a.encoding.as_deref().filter(|e| !e.is_empty()),
            bytes,
            attachment: attachment.as_deref(),
            delete: a.is_delete(),
        },
        member,
        lens,
        structural_only,
    )
}

/// The table form's flags: which shape a reply is printed in, and the lens
/// it is resolved through.
struct Human<'a> {
    lens: &'a Lens<'a>,
    namespace: &'a str,
    /// `--raw`: the key and the bytes in hex, nothing resolved.
    raw: bool,
    /// `--hex`: the key, the type tag and the bytes in hex.
    hex: bool,
    /// `--fmt`: one line per reply, from the `%` vocabulary.
    fmt: Option<&'a str>,
    structural_only: bool,
}

/// One reply as the table form prints it: stdout's lines, then stderr's
/// (what failed against the declared type).
#[derive(Debug, Default)]
struct Printed {
    out: Vec<String>,
    err: Vec<String>,
}

impl Human<'_> {
    /// One reply in the table form, pure so a snapshot pins it: the reply's
    /// key, then the rendering with the rung it stopped at, or the
    /// tombstone's words for a delete in every shape (#765). `n` is the
    /// reply's place among the values, `--fmt`'s `%n`.
    fn reply(&self, a: &FleetAnswer, n: usize) -> Printed {
        let mut p = Printed::default();
        let payload = match &a.answer {
            Answer::Error { name, message } => {
                // An error reply has no sample, so no key to name.
                p.out.push(format!("✗ {name} — {message}"));
                return p;
            }
            Answer::Value(payload) => payload,
        };
        let delete = a.is_delete();
        let bytes = payload.to_bytes().into_owned();
        if self.raw {
            p.out.push(a.key.clone());
            p.out.push(if delete {
                format!("  {TOMBSTONE}")
            } else {
                format!("  {}", hex(&bytes))
            });
            if let Some(att) = &a.attachment {
                p.out
                    .push(format!("  attachment: {}", hex(&att.to_bytes())));
            }
            return p;
        }
        let line = line_of(a, bytes, self.lens, self.structural_only);
        if self.hex {
            p.out.push(a.key.clone());
            p.out.push(if delete {
                format!("  {TOMBSTONE}")
            } else {
                format!("  {} {}", line.tag(), hex(&line.bytes))
            });
            if let Some(att) = &a.attachment {
                p.out
                    .push(format!("  attachment: {}", hex(&att.to_bytes())));
            }
            return p;
        }
        let attachment = line
            .attachment
            .as_ref()
            .map(crate::render::payload_text)
            .or_else(|| {
                a.attachment
                    .as_ref()
                    .map(|t| attachment_display(&t.to_bytes()))
            });
        if let Some(fmt) = self.fmt {
            let value = if delete {
                TOMBSTONE.to_owned()
            } else {
                line.value_text()
            };
            let declared = line.declared();
            let stamp = a.timestamp.map(|t| t.to_string());
            p.out.push(format_sample(
                fmt,
                &SampleLine {
                    n,
                    wire_key: &a.key,
                    relative: zenkey_fleet::strip_namespace(self.namespace, &a.key),
                    identity: &line.identity,
                    type_name: declared.as_deref(),
                    encoding: a.encoding.as_deref().unwrap_or(""),
                    payload_len: line.bytes.len(),
                    // A reply is not a subscribe-path sample: no QoS axes
                    // and no SourceInfo, and an empty field is honest
                    // (#120). The HLC is the one thing a reply may carry —
                    // only when the responder stamped it (#215).
                    timestamp: stamp.as_deref(),
                    value: &value,
                    attachment: attachment.as_deref(),
                    qos: None,
                    source: None,
                },
            ));
            return p;
        }
        p.out.push(a.key.clone());
        p.out.push(match &line.checked {
            Some((rendering, _)) => format!("  {}", crate::render::payload_text(rendering)),
            None => format!("  {TOMBSTONE}"),
        });
        if let Some(att) = attachment {
            p.out.push(format!("  attachment: {att}"));
        }
        if let Some((_, Conformance::Invalid { violations })) = &line.checked {
            p.err
                .extend(violations.iter().map(|v| format!("  invalid: {v}")));
        }
        p
    }
}

/// One reply as a JSON row — the ndjson line and the json array element.
/// A delete says so, `"delete": true`, and carries no bytes (#765).
fn row(a: &FleetAnswer, lens: &Lens<'_>, raw: bool, structural_only: bool) -> serde_json::Value {
    match &a.answer {
        Answer::Error { name, message } => serde_json::json!({
            "error": { "name": name, "message": message },
        }),
        Answer::Value(payload) => {
            let bytes = payload.to_bytes().into_owned();
            if raw {
                // `delete` is always written, as on the decoded row: every
                // reply sample is a put or a delete, and that is a fact.
                let mut obj = serde_json::json!({
                    "key": a.key,
                    "encoding": a.encoding,
                    "delete": a.is_delete(),
                });
                if !a.is_delete() {
                    obj["hex"] = serde_json::Value::String(hex(&bytes));
                }
                if let Some(att) = &a.attachment {
                    obj["attachment"] = serde_json::Value::String(hex(&att.to_bytes()));
                    obj["attachment_bytes"] = att.len().into();
                }
                return obj;
            }
            value_row(a, &line_of(a, bytes, lens, structural_only))
        }
    }
}

/// The decoded half of `row`, pure so the verdict terms are testable: the
/// row dialect `echo --format ndjson` writes (so `pub --from ndjson` reads
/// it back), with the reply's HLC when the responder stamped one.
fn value_row(a: &FleetAnswer, line: &Line) -> serde_json::Value {
    let mut row = zenkey_fleet::SampleRow::of_key(&a.key);
    row.encoding = a.encoding.clone().filter(|e| !e.is_empty());
    row.timestamp = a.timestamp.map(|t| t.to_string());
    row.delete = a.is_delete();
    let attachment = a.attachment.as_ref().map(|t| t.to_bytes());
    line.fill(&mut row, attachment.as_deref());
    if let (None, Some(att)) = (&row.attachment, attachment.as_deref()) {
        row.attachment = Some(attachment_json(att));
    }
    serde_json::to_value(&row).expect("a sample row serializes")
}

/// `get state <address> <iface>[@fp] <state> [--last-known <archive>]`:
/// the owner's current state (S4), or an archive's last-known state (S5),
/// through the contract. Exit 2 on silence: never "no value" (S6).
pub async fn state(cli: crate::cli::StateGetArgs) -> Result<()> {
    use crate::cmd::zk2;
    let dep = crate::bus::Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let target = zenkey_fleet::ResolvedTarget::parse(&cli.address.to_string())?;
    let values = zk2::bindings(&cli.params);
    let mut session = None;
    let revision =
        zk2::revision_at(&dep, &contracts, &cli.target, Some(&target), &mut session).await?;
    let r = zenkey_fleet::resolve_resource(&revision, &cli.resource, &[Kind::State])?;
    zenkey_fleet::check_values(r, &values)?;
    let session = match session {
        Some(s) => s,
        None => dep.session().await?,
    };
    let read = zenkey_fleet::StateRead {
        revision: &revision,
        owner: &cli.address,
        resource: r,
        values: &values,
        timeout: dep.timeout(),
    };
    let report = match &cli.last_known {
        None => zenkey_fleet::get_state(&session, read).await?,
        Some(archive) => zenkey_fleet::last_known_state(&session, read, archive).await?,
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    if report.rows.is_empty() {
        std::process::exit(crate::exit::NO_VERDICT);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use snapbox::{assert_data_eq, str};
    use zenoh::sample::SampleKind;

    fn value(key: &str) -> FleetAnswer {
        FleetAnswer {
            key: key.into(),
            encoding: None,
            attachment: None,
            timestamp: None,
            kind: Some(SampleKind::Put),
            answer: Answer::Value(zenoh::bytes::ZBytes::from("1")),
        }
    }

    /// An owner's `reply_del` as `fleet_get` keeps it (#765): a sample,
    /// stamped, its payload the empty one a delete carries.
    fn tombstone(key: &str) -> FleetAnswer {
        FleetAnswer {
            key: key.into(),
            encoding: Some("zenoh/bytes".into()),
            attachment: None,
            timestamp: Some(zenoh::time::Timestamp::new(
                zenoh::time::NTP64(1 << 32),
                "a1b2".parse().expect("an id"),
            )),
            kind: Some(SampleKind::Delete),
            answer: Answer::Value(zenoh::bytes::ZBytes::new()),
        }
    }

    /// A put whose payload is empty: what a tombstone read as a put looked
    /// like before #765, and must never look like again.
    fn empty_put(key: &str) -> FleetAnswer {
        FleetAnswer {
            answer: Answer::Value(zenoh::bytes::ZBytes::new()),
            ..value(key)
        }
    }

    fn error() -> FleetAnswer {
        FleetAnswer {
            key: String::new(),
            encoding: None,
            attachment: None,
            timestamp: None,
            kind: None,
            answer: Answer::Error {
                name: "error/unavailable".into(),
                message: "busy".into(),
            },
        }
    }

    /// 0 = values only, 1 = any error envelope, 2 = silence — the same
    /// discipline as `call`'s, on raw answers.
    #[test]
    fn get_exit_codes_follow_the_reply_discipline() {
        assert_eq!(exit_code(&[]), 2);
        assert_eq!(exit_code(&[value("a"), value("b")]), 0);
        assert_eq!(exit_code(&[value("a"), error()]), 1);
    }

    /// A reply row is the row dialect: its key, the identity the lens made
    /// of it, and the conformance verdict — a key no contract reaches
    /// renders structurally and says why, never refused, and `--no-decode`
    /// says it did not ask (#246's terms, kept: the verdict is present
    /// whenever a row carries a value).
    #[test]
    fn a_get_row_carries_the_identity_and_the_verdict() {
        let contracts = zenkey_fleet::ContractSet::new();
        let lens = Lens::new("prod", None, &contracts);

        let foreign = value("plant/line-1/temp");
        let r = row(&foreign, &lens, false, false);
        assert_eq!(r["key"], "plant/line-1/temp");
        assert_eq!(r["identity"]["is"], "not_in_namespace");
        assert_eq!(r["value"], 1);
        assert!(
            r["verdict"].as_str().unwrap().starts_with("not-checked"),
            "{r}"
        );
        assert!(r.get("origin").is_none(), "v1's origin left at FJ9");

        let zk2 = value("prod/zk2/host-a/tc/tc.netif.v1/state/namespaces");
        let r = row(&zk2, &lens, false, false);
        assert_eq!(r["identity"]["is"], "resource");
        assert_eq!(r["identity"]["address"], "host-a/tc");

        let skipped = row(&zk2, &lens, false, true);
        assert!(
            skipped["verdict"]
                .as_str()
                .unwrap()
                .contains("decode not asked")
                || skipped["verdict"]
                    .as_str()
                    .unwrap()
                    .starts_with("not-checked"),
            "{skipped}"
        );

        let raw = row(&zk2, &lens, true, true);
        assert_eq!(raw["hex"], "31");
        assert!(raw.get("identity").is_none(), "--raw resolves nothing");

        let err = row(&error(), &lens, false, false);
        assert_eq!(err["error"]["name"], "error/unavailable");
    }

    const ETH0: &str = "prod/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";

    /// One reply's stdout in each shape of the table form: the default
    /// rendering, `--hex`, `--raw` (which resolves nothing) and `--fmt`.
    fn shown(a: &FleetAnswer, lens: &Lens<'_>) -> String {
        let shape = |raw, hex, fmt| Human {
            lens,
            namespace: "prod",
            raw,
            hex,
            fmt,
            structural_only: raw,
        };
        let mut out = String::new();
        for (name, h) in [
            ("table", shape(false, false, None)),
            ("--hex", shape(false, true, None)),
            ("--raw", shape(true, false, None)),
            (
                "--fmt",
                shape(false, false, Some("%n %k <%t> %v (%l B) %T")),
            ),
        ] {
            out.push_str(&format!("# {name}\n"));
            for line in h.reply(a, 1).out {
                out.push_str(&line);
                out.push('\n');
            }
        }
        out
    }

    /// #765: an owner's `reply_del` reads as a deletion in every shape of
    /// the table form, in the words `echo` uses for one — never as a put
    /// whose payload is empty, which is drawn below it for contrast.
    #[test]
    fn a_deletion_renders_as_one_in_every_human_form() {
        let contracts = zenkey_fleet::ContractSet::new();
        let lens = Lens::new("prod", None, &contracts);
        assert_data_eq!(
            shown(&tombstone(ETH0), &lens),
            str![[r#"
# table
prod/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0
  <tombstone — a deletion, not an empty value>
# --hex
prod/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0
  <tombstone — a deletion, not an empty value>
# --raw
prod/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0
  <tombstone — a deletion, not an empty value>
# --fmt
1 prod/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0 <> <tombstone — a deletion, not an empty value> (0 B) 4294967296/a1b2

"#]]
        );
        let put = shown(&empty_put(ETH0), &lens);
        assert!(put.contains("\n  <0 bytes>  (structural: "), "{put}");
        assert!(!put.contains(TOMBSTONE), "an empty put is a value: {put}");
        assert_eq!(exit_code(&[tombstone(ETH0)]), 0, "a deletion is an answer");
    }

    /// #765: a deletion's row says `"delete": true` and carries no value,
    /// no byte count and no verdict — "0 bytes" would read as an empty
    /// put — and `--raw`'s row says it too, with no hex; a put's rows say
    /// `"delete": false`, always written.
    #[test]
    fn a_deletion_row_says_delete_and_carries_no_bytes() {
        let contracts = zenkey_fleet::ContractSet::new();
        let lens = Lens::new("prod", None, &contracts);
        let r = row(&tombstone(ETH0), &lens, false, false);
        assert_data_eq!(
            serde_json::to_string_pretty(&r).expect("a row serializes"),
            str![[r#"
{
  "delete": true,
  "encoding": "zenoh/bytes",
  "identity": {
    "address": "host-a/tc",
    "iface": "tc.netif.v1",
    "is": "resource",
    "token": "state",
    "unresolved": {
      "reason": "presence_not_read"
    }
  },
  "key": "prod/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0",
  "timestamp": "4294967296/a1b2"
}
"#]]
        );
        let put = row(&empty_put(ETH0), &lens, false, false);
        assert_eq!(put["delete"], false);
        assert_eq!(put["payload_bytes"], 0, "an empty put is a value");

        let raw = row(&tombstone(ETH0), &lens, true, true);
        assert_eq!(
            raw,
            serde_json::json!({"key": ETH0, "encoding": "zenoh/bytes", "delete": true})
        );
        let raw_put = row(&empty_put(ETH0), &lens, true, true);
        assert_eq!(raw_put["delete"], false);
        assert_eq!(raw_put["hex"], "");
    }
}
