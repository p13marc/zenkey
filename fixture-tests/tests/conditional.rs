//! The fixture registry's `conditional.lock` (RFC 08 §6.1, v1.25), pinned.
//!
//! The build itself already checks the ledger (`build.rs` runs
//! `zenkey_build::Config::generate`, which fails on a line naming no
//! registry subject); this test pins the *consumer-facing* half — the
//! validated set a downstream emitted-surface check reads to know which
//! subjects it must NOT require the build's mappers to cover
//! unconditionally. The generated code is untouched by the ledger: the
//! slice carries no conditional marking (the ledger conditions an entry,
//! it does not replace one).

#[test]
fn conditional_ledger_surfaces_the_fixture_gates() {
    let entries = zenkey_build::Config::new()
        .registry_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/registry"))
        .no_rerun_if_changed()
        .conditional_subjects()
        .unwrap();
    let rows: Vec<(&str, &str, &str)> = entries
        .iter()
        .map(|e| (e.producer.as_str(), e.path.as_str(), e.condition.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("netlink", "sockets/tcp/connlat_us_p50", "feature ebpf"),
            ("netlink", "sockets/tcp/connlat_us_p95", "feature ebpf"),
            (
                "netlink",
                "wireguard/{iface}/peers",
                "host exposes a WireGuard interface"
            ),
        ]
    );
}
