# zenkey — plain `cargo` remains the build system. This file only covers the
# things that need more than one command, chiefly running the GUI against
# traffic to look at.
#
#   just gui-demo     # the GUI, against generated demo traffic. Start here.
#   just ci           # everything CI runs, in the same order
#
# ── What to look at once the window is up ────────────────────────────────
#
# `spray` publishes conforming *and* deliberately non-conforming keys, because
# the interesting half of zengui is what it does with traffic this convention
# does not govern.
#
#   * Expand `v1` — chunks are labelled origin / class / producer / subject,
#     and leaves carry a registration badge plus the registry-declared type.
#   * Expand `demo`, `two`, `v2`, `someotherbase` — foreign keys, carrying real
#     counts and rates, deliberately *unlabelled* rather than mislabelled.
#     `someotherbase/…` is another deployment's traffic, visible because an
#     explorer runs un-namespaced (RFC 09 §5).
#   * Switch scope to `deployment` and watch `@catalog` appear. It is invisible
#     under `everything` because `**` never crosses an `@` chunk (RFC 03 §4 D2)
#     — which is also why `**` cannot pull the `@media` frames spray publishes.
#   * Select `…/telemetry/sysinfo/disk/var-log/used` and watch its subtree. The
#     detail pane's `Series` section traces the wandering `value`; the `history`
#     tab (Alt 8) fills, and clicking a row names the field that moved. Before
#     the watch, both say *why* they are empty — an unwatched key records
#     nothing, and that is not the same as a quiet one.
#   * Select `…/state/sysinfo/health` and wait: every 20th sample is a
#     tombstone. It renders as retirement, and the put after it as a new value
#     rather than a change (RFC 04 §1.2).
#   * `…/telemetry/probe/reading` moves too, but offers no chart: a protobuf
#     leaf needs a schema decode, which must not sit on a render path.
#   * Alt 9 opens the blob pane. Probe `01jqz3demo0001`: spray serves it from
#     two origins at two different content roots, so the pane flags the
#     disagreement rather than picking (RFC 07 §2.1). Fetch from the second
#     with the first's root pinned and it aborts naming that origin, leaving
#     no file — verification happens before disk, not after transfer.
#   * Alt 0 opens the admin pane. Sweep it: against this demo's peer-only bus
#     every section should say why it is empty, and the coverage table should
#     read "coverage not judged" rather than "uncovered" — a registry that was
#     never loaded has not told you a family is uncovered (RFC 09 §5.1 O4).
#   * `just gui-demo-bounded` trips *both* bounds immediately: the status strip
#     should read "N keys (+M retired — bound reached)" and, beside it,
#     "facts: N cached (+M projections retired — cache bound reached)" — two
#     bounds over two populations, two sentences (RFC 09 §5.1 O6).
#   * `just gui-demo-no-registry` withholds the registry: every badge should
#     read "—" ("not asked"), never "unregistered" (RFC 09 §5.1 O4).
#
# `just --list` for the rest.

port := "7449"
registry := "fixture-tests/registry"
rundir := ".run"

# zengui's pane tests build a real iced renderer per test, which probes wgpu
# across every backend and every Vulkan ICD on the box. Left unconstrained on
# a headless host that segfaults under concurrency — measured at 12/128 runs
# of 8 concurrent test binaries, against 0/224 with *any* constraint applied
# (`gl`, `vulkan`, a single ICD, or --test-threads=1). The fault is upstream
# in wgpu/mesa adapter enumeration, not here; naming one backend is the
# mitigation, and it is a mitigation rather than a repair (zenkey #229).
export WGPU_BACKEND := "gl"

default:
    @just --list

# The GUI against generated demo traffic — start here. Ctrl-C stops both.
gui-demo port=port: (_demo port registry "50000")

# The demo with a key bound small enough to trip immediately.
gui-demo-bounded port=port: (_demo port registry "6")

# The demo with no registry loaded, so badges read "not asked".
gui-demo-no-registry port=port: (_demo port "" "50000")

_demo port registry_dir max_keys:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p zengui --example spray
    cargo build -p zengui
    mkdir -p {{rundir}}
    ./target/debug/examples/spray -l tcp/127.0.0.1:{{port}} > {{rundir}}/spray.log 2>&1 &
    spray=$!
    trap 'kill $spray 2>/dev/null || true' EXIT
    for _ in $(seq 1 100); do
        grep -q 'Ctrl-C to stop' {{rundir}}/spray.log 2>/dev/null && break
        kill -0 $spray 2>/dev/null || { echo "spray exited early:"; cat {{rundir}}/spray.log; exit 1; }
        sleep 0.1
    done
    echo "spray publishing on tcp/127.0.0.1:{{port}} (log: {{rundir}}/spray.log)"
    ./target/debug/zengui -c tcp/127.0.0.1:{{port}} \
        --max-keys {{max_keys}} \
        {{ if registry_dir == "" { "" } else { "--registry " + registry_dir } }}

# Just the traffic generator: it listens, so anything can connect to it.
spray port=port:
    cargo run -p zengui --example spray -- -l tcp/127.0.0.1:{{port}}

# Just the GUI, against an already-running bus.
gui port=port:
    cargo run -p zengui -- -c tcp/127.0.0.1:{{port}} --registry {{registry}}

# The CLI, for cross-checking the GUI's numbers against the same engine.
ctl *ARGS:
    cargo run -p zenctl -- {{ARGS}}

# The live-bus tests. Needs `just spray` running in another terminal.
test-live port=port:
    ZENGUI_TEST_ENDPOINT=tcp/127.0.0.1:{{port}} \
        cargo test -p zengui --test live_bus -- --ignored --nocapture

# The ordinary test suite.
test:
    cargo test --workspace --locked

# Everything CI runs (.forgejo/workflows/ci.yml), in the same order.
ci:
    cargo fmt --all --check
    # The gutter gate (#195): a dropped `\` in a multi-line string prints the
    # source indentation to the user. Cheap, and it runs before the compiler.
    python3 scripts/check-prose.py zenctl/src zenkey-fleet/src zengui/src zenkey/src
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    cargo build --workspace --all-targets --locked
    cargo test --workspace --locked
    cargo bench --workspace --no-run --locked
    just features
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

# The criterion baselines behind docs/bench-baseline.md. Slow: tree/build_50k
# and skeleton/merge_10k are tens of milliseconds an iteration.
bench:
    cargo bench --workspace

# The #45 soak: a hot bus against the O6 ledger, with the numbers printed.
# The ledger itself is an ordinary test and runs in `just ci`.
soak:
    cargo test --release -p zenkey-fleet --test ledger -- --ignored --nocapture

fmt:
    cargo fmt --all

# Remove the demo's scratch directory.
clean-run:
    rm -rf {{rundir}}

# `--all-features` cannot see a feature that silently depends on another one.
# Each published axis of zenkey-fleet, on its own (#204).
features:
    cargo check -p zenkey-fleet --no-default-features --locked
    for f in blob decode decode-protobuf decode-cdr validate-json; do \
        cargo check -p zenkey-fleet --no-default-features --features "$f" --locked; \
    done
    cargo bench -p zenkey-fleet --no-default-features --no-run --locked
