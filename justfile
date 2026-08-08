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
#   * `just gui-demo-bounded` trips the key bound immediately: the status strip
#     should read "N keys (+M retired — bound reached)" (RFC 09 §5.1 O6).
#   * `just gui-demo-no-registry` withholds the registry: every badge should
#     read "—" ("not asked"), never "unregistered" (RFC 09 §5.1 O4).
#
# `just --list` for the rest.

port := "7449"
registry := "fixture-tests/registry"
rundir := ".run"

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
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    cargo build --workspace --all-targets --locked
    cargo test --workspace --locked
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

fmt:
    cargo fmt --all

# Remove the demo's scratch directory.
clean-run:
    rm -rf {{rundir}}
