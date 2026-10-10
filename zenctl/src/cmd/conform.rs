//! `check conform` (#703) — one zk2 service against the contract revision
//! it claims, as a conformance suite.
//!
//! The suite is the engine's ([`zenkey_fleet::run_conform`]): the reads in
//! its bus layer, every case decided from values in its judge, so a GUI or a
//! CI harness asks the same questions. This command is orchestration and
//! rendering: resolve the deployment, open a session **in** its namespace
//! and one in none for the routers' admin space (S1, §4.2, 0.17), run, print, write `--junit`, and exit through the report's own judgement
//! — 1 on a violation, 0 when every case asked passed, 2 when a case was
//! left unobservable or the service could not be judged at all. A verdict
//! verb: every failure before the run is the reserved 2.
//!
//! **JUnit** is somebody else's schema, like `--dot` and `--json5`, so it is
//! a file of its own beside the report rather than a `--format`: a
//! violation is a `<failure>`, an unobservable case an `<error>` (the test
//! could not run), a case not asked `<skipped>` — the two Unestablished
//! kinds kept apart in that medium too (tooling guide §1).

use anyhow::Result;
use zenkey_fleet::report::{ConformCase, ConformReport};
use zenkey_fleet::{ConformSpec, Judgement};

use crate::bus::Deployment;
use crate::cli::CheckConformArgs;
use crate::cmd::zk2;

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("check conform");

pub async fn run(cli: CheckConformArgs) -> Result<()> {
    let CheckConformArgs {
        address,
        target,
        for_secs,
        i_know,
        junit,
        seed,
        trust_admin_space,
        calls_granted,
        clocks_synced,
        skip,
        contracts,
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
    let window = ASKING.ask(super::positive_secs("--for", for_secs));
    let offline = ASKING.ask(zk2::load_contracts(&contracts));
    let spec = ConformSpec {
        timeout: dep.timeout(),
        window,
        call_all: i_know,
        seed,
        trust_admin: trust_admin_space,
        calls_granted,
        clocks_synced,
    };
    // The service is read in the deployment's namespace, and the admin
    // space in none, as the doctor reads them (S1, §4.2, 0.17).
    let bus = zenkey_fleet::DoctorBus {
        session: ASKING.ask(dep.session().await),
        raw: ASKING.ask(dep.link().session().await),
        namespace: dep.namespace().to_owned(),
    };
    let mut report = zenkey_fleet::run_conform(
        &bus,
        &offline,
        address,
        target.iface,
        target.fingerprint,
        spec,
    )
    .await;
    // `--skip`: the operator's choice, never the engine's (#735).
    report.skip(&skip);
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    if let Some(path) = junit {
        // The artifact CI asked for: without it, no verdict either.
        ASKING.ask(
            std::fs::write(&path, to_junit(&report))
                .map_err(|e| anyhow::anyhow!("--junit {}: {e}", path.display())),
        );
    }
    let judgement = report.judgement();
    if let Judgement::Unobservable { reason } = &judgement {
        eprintln!("check conform: {reason} — exit 2, the reserved non-verdict");
    }
    crate::exit::verdict(&judgement)
}

/// XML text, escaped for an attribute or an element.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\t' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// One case as a `<testcase>`: its class the interface and the case, its
/// name the subject.
fn testcase(iface: &str, c: &ConformCase) -> String {
    let head = format!(
        "    <testcase classname=\"{}\" name=\"{}\">",
        esc(&format!("{iface}.{}", c.case)),
        esc(&c.subject)
    );
    let body = match &c.verdict {
        Judgement::Established => {
            let detail = c.detail.as_deref().unwrap_or("a violation");
            format!(
                "\n      <failure type=\"violation\" message=\"{}\">{} ({})</failure>\n    ",
                esc(detail),
                esc(detail),
                esc(c.section)
            )
        }
        Judgement::Unobservable { reason } => format!(
            "\n      <error type=\"unobservable\" message=\"{}\"/>\n    ",
            esc(reason)
        ),
        Judgement::NotAsked => format!(
            "\n      <skipped message=\"{}\"/>\n    ",
            esc(&format!(
                "not asked: {}",
                c.detail.as_deref().unwrap_or("not asked")
            ))
        ),
        Judgement::NotEstablished { reason } => {
            format!("\n      <system-out>{}</system-out>\n    ", esc(reason))
        }
    };
    format!("{head}{body}</testcase>")
}

/// The suite as JUnit XML: one `<testsuite>` for the service and its
/// interface, one `<testcase>` per case.
pub fn to_junit(r: &ConformReport) -> String {
    let count = |p: fn(&Judgement) -> bool| r.cases.iter().filter(|c| p(&c.verdict)).count();
    let failures = count(|j| *j == Judgement::Established);
    let errors = count(Judgement::is_unobservable);
    let skipped = count(Judgement::is_not_asked);
    let name = format!("{} {}", r.address, r.iface);
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuites name=\"zenctl check conform\" tests=\"{}\" failures=\"{failures}\" \
         errors=\"{errors}\" skipped=\"{skipped}\">\n",
        r.cases.len()
    ));
    out.push_str(&format!(
        "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{failures}\" errors=\"{errors}\" \
         skipped=\"{skipped}\">\n",
        esc(&name),
        r.cases.len()
    ));
    out.push_str("    <properties>\n");
    let mut props = vec![
        ("namespace", r.namespace.clone()),
        ("window_s", r.window_s.to_string()),
    ];
    if let Some(fp) = &r.fingerprint {
        props.push(("fingerprint", fp.clone()));
    }
    if let Some(u) = &r.unobservable {
        props.push(("unobservable", u.clone()));
    }
    for (k, v) in props {
        out.push_str(&format!(
            "      <property name=\"{k}\" value=\"{}\"/>\n",
            esc(&v)
        ));
    }
    out.push_str("    </properties>\n");
    for c in &r.cases {
        out.push_str(&testcase(&r.iface, c));
        out.push('\n');
    }
    out.push_str("  </testsuite>\n</testsuites>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_fleet::report::CaseId;

    fn report() -> ConformReport {
        ConformReport {
            address: "lab/m".into(),
            iface: "m.v1".into(),
            fingerprint: Some("sha256:ab".into()),
            namespace: "acme".into(),
            window_s: 5.0,
            asked: vec![],
            cases: vec![
                ConformCase::passed(CaseId::Qos, "stream/x", "3 sample(s)"),
                ConformCase::failed(CaseId::PayloadType, "stream/x", "/rx: <not> an \"integer\""),
                ConformCase::unobservable(CaseId::ResourceServed, "state/y", "silent"),
                ConformCase::not_asked(CaseId::Budget, "state/y/{id}", "no ceiling"),
            ],
            unobservable: None,
        }
    }

    /// Each pole its own JUnit element, the counts on both suites, and
    /// every attribute escaped.
    #[test]
    fn the_suite_is_junit_with_each_pole_apart() {
        let xml = to_junit(&report());
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
        assert!(xml.contains(
            "<testsuites name=\"zenctl check conform\" tests=\"4\" failures=\"1\" errors=\"1\" \
             skipped=\"1\">"
        ));
        assert!(xml.contains("<testsuite name=\"lab/m m.v1\" tests=\"4\""));
        assert!(xml.contains("<property name=\"fingerprint\" value=\"sha256:ab\"/>"));
        assert!(xml.contains(
            "<testcase classname=\"m.v1.payload-type\" name=\"stream/x\">\n      <failure \
             type=\"violation\" message=\"/rx: &lt;not&gt; an &quot;integer&quot;\">"
        ));
        assert!(xml.contains("<error type=\"unobservable\" message=\"silent\"/>"));
        assert!(xml.contains("<skipped message=\"not asked: no ceiling\"/>"));
        assert!(xml.contains("<system-out>3 sample(s)</system-out>"));
        assert_eq!(xml.matches("<testcase ").count(), 4);
        assert!(xml.ends_with("</testsuite>\n</testsuites>\n"));
    }
}
