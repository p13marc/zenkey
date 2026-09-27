//! The fixture registry's conditional subjects (RFC 08 §6.1), pinned.
//!
//! Since v1.41 (zenkey #171) the gates are `when` fields on the entries
//! themselves and the ledger `conditional.lock` is empty; this test pins
//! the *consumer-facing* half — the validated set a downstream
//! emitted-surface check reads to know which subjects it must NOT require
//! the build's mappers to cover unconditionally — now carrying the
//! predicates a consumer can read, beside the prose a human can.

#[test]
fn conditional_ledger_surfaces_the_fixture_gates() {
    let entries = zenkey_build::Config::new()
        .registry_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/registry"))
        .no_rerun_if_changed()
        .conditional_subjects()
        .unwrap();
    /// producer, path, the human condition, and the predicates a consumer reads.
    type Row<'a> = (&'a str, &'a str, &'a str, Vec<(&'a str, &'a str)>);
    let rows: Vec<Row<'_>> = entries
        .iter()
        .map(|e| {
            (
                e.producer.as_str(),
                e.path.as_str(),
                e.condition.as_str(),
                e.predicates
                    .iter()
                    .map(|(k, n)| (k.as_str(), n.as_str()))
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                "netlink",
                "sockets/tcp/connlat_us_p50",
                "feature:ebpf",
                vec![("feature", "ebpf")]
            ),
            (
                "netlink",
                "sockets/tcp/connlat_us_p95",
                "feature:ebpf",
                vec![("feature", "ebpf")]
            ),
            (
                "netlink",
                "wireguard/{iface}/peers",
                "host exposes a WireGuard interface",
                vec![("capability", "wireguard")]
            ),
        ]
    );
}
