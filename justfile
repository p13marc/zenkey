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
#     Inspector's `Series` section traces the wandering `value`; its `History`
#     section fills, and clicking a row names the field that moved. Before
#     the watch, both say *why* they are empty — an unwatched key records
#     nothing, and that is not the same as a quiet one.
#   * Select `…/state/sysinfo/health` and wait: every 20th sample is a
#     tombstone. It renders as retirement, and the put after it as a new value
#     rather than a change (RFC 04 §1.2).
#   * `…/telemetry/probe/reading` moves too, but offers no chart: a protobuf
#     leaf needs a schema decode, which must not sit on a render path.
#   * Select a key on spray's `@blob` plane and the Inspector grows a Blobs
#     section. Probe `01jqz3demo0001`: spray serves it from
#     two origins at two different content roots, so the pane flags the
#     disagreement rather than picking (RFC 07 §2.1). Fetch from the second
#     with the first's root pinned and it aborts naming that origin, leaving
#     no file — verification happens before disk, not after transfer.
#   * Open the admin tool (the Workbench dock, or the palette's "go to admin
#     pane"). Sweep it: against this demo's peer-only bus
#     every section should say why it is empty, and the coverage table should
#     read "coverage not judged" rather than "uncovered" — a registry that was
#     never loaded has not told you a family is uncovered (RFC 09 §5.1 O4).
#   * Open the config tool — the Workbench's "config", or "configure" on
#     `probe` in the nodes pane. Read `wlan0`: spray serves the RFC 05 §5.1
#     double, a group of each class. Change `queue` (hot) and it applies at
#     once; change `link` (reach) and it leaves only with a window and a yes,
#     then waits armed — confirm it, or let the window run out and watch the
#     read-back roll it back. `transport` (contract) names the restart.
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
    python3 scripts/check-prose.py zenctl/src zenkey-fleet/src zengui/src zenkey-explorer-config/src zenwatch/src
    # One report, three renderings, and exactly one place that decides which
    # (#198).
    ./scripts/check-render-seam.sh
    # The dispatch dispatches (#209). #210's one door out of a missing
    # registry left with the v1 registry (#612, FJ9).
    ./scripts/check-dispatch.sh
    # The update thread does not touch the disk (#255): a handler that needs
    # the filesystem needs a services:: function and a landing message.
    ./scripts/check-fs-seam.sh
    # RFC chapter headers agree with rfcs/CHANGELOG.md's Amends ledger
    # (v1.25 S6), and the version stated in 00-index.md, CLAUDE.md and
    # README.md is the newest entry's. CI runs it on both lanes: ci.yml for
    # code pushes, docs.yml for doc-only ones (#420).
    ./scripts/check-rfc-status.sh
    # The type scale by role (#191), the interactive seam (#193), the
    # spacing grid (#192) and the colour gate (#534) — all run as CI's
    # type-scale job.
    ./scripts/check-type-scale.sh
    ./scripts/check-interactive.sh
    ./scripts/check-spacing.sh
    # Colour comes from the theme (#534).
    ./scripts/check-color.sh
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    cargo build --workspace --all-targets --locked
    cargo test --workspace --locked
    cargo bench --workspace --no-run --locked
    just features
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

# The criterion baselines behind docs/bench-baseline.md. Slow: tree/build_50k
# is tens of milliseconds an iteration.
bench:
    cargo bench --workspace

# The #45 soak: a hot bus against the O6 ledger, with the numbers printed.
# The ledger itself is an ordinary test and runs in `just ci`.
soak:
    cargo test --release -p zenkey-fleet --test ledger -- --ignored --nocapture

# A picture of every zengui surface, both themes (#532): the scene list in
# zengui/tests/common/scenes.rs, drawn by the app's own renderer — wgpu on
# Vulkan (lavapipe where there is no GPU). Not tiny-skia, which drops canvases
# (#533), and not GL, whose mesa path draws only the last canvas in a frame
# (#540). PNGs land in `dir`, twice the scenes' logical size.
shots dir="target/shots/current":
    SHOTS_DIR={{justfile_directory()}}/{{dir}} ICED_TEST_BACKEND=wgpu WGPU_BACKEND=vulkan \
        cargo test -p zengui --test shots --locked -- --ignored --nocapture

# The same scenes drawn from another revision, into target/shots/base, so a
# visual change is reviewed as a before/after pair. The revision's own harness
# draws them — its scenes speak its API; only a revision older than #532,
# which has none, borrows this checkout's.
shots-base rev="main":
    #!/usr/bin/env bash
    set -euo pipefail
    root={{justfile_directory()}}
    src=$root/target/shots-src
    git -C "$root" worktree remove --force "$src" 2>/dev/null || rm -rf "$src"
    git -C "$root" worktree add --detach "$src" {{rev}}
    trap 'git -C "$root" worktree remove --force "$src"' EXIT
    if [ ! -f "$src/zengui/tests/shots.rs" ]; then
        mkdir -p "$src/zengui/tests/common"
        cp "$root/zengui/tests/shots.rs" "$src/zengui/tests/"
        cp "$root"/zengui/tests/common/*.rs "$src/zengui/tests/common/"
    fi
    cd "$src"
    SHOTS_DIR=$root/target/shots/base ICED_TEST_BACKEND=wgpu WGPU_BACKEND=vulkan CARGO_TARGET_DIR=$root/target \
        cargo test -p zengui --test shots --locked -- --ignored --nocapture

# Re-capture every pinned CLI transcript and render snapshot after an
# intentional change (#201). Review the diff: that review is the point of the
# corpus rather than its cost.
snapshots:
    TRYCMD=overwrite SNAPSHOTS=overwrite cargo test -p zenctl

fmt:
    cargo fmt --all

# Remove the demo's scratch directory.
clean-run:
    rm -rf {{rundir}}

# `--all-features` cannot see a build without one (#204). zenkey-fleet has had
# no feature axes since FJ9 (#612); the axis left is zenkey's `zenoh`, without
# which the runtime is the session-free half a contract crate builds on.
features:
    cargo check -p zenkey@0.20.0 --no-default-features --locked
    ./scripts/check-model-zenoh-free.sh

# Builds target/py-venv with the standard library only (no ensurepip
# needed), then a pinned, hash-checked pip, the pinned requirements, and
# protoc 3.21.12 when the one on PATH is not that version
# (impl/python/README.md).
# The Python zk2 implementation (#609) against spec/conformance and the examples.
py-conformance:
    #!/usr/bin/env bash
    set -euo pipefail
    protoc="$(python3 impl/python/bootstrap.py target/py-venv | tail -n 1)"
    ZK2PY_PROTOC="$protoc" PYTHONPATH=impl/python \
        target/py-venv/bin/python -m zk2py.conformance spec/conformance

# The owner and consume examples, then the live runner over them. Reuses target/py-venv
# (eclipse-zenoh 1.10.1 is pinned in impl/python/requirements.txt).
# The Python live interop runner (#609, #610), with the Rust owner and consume examples.
py-live:
    #!/usr/bin/env bash
    set -euo pipefail
    protoc="$(python3 impl/python/bootstrap.py target/py-venv | tail -n 1)"
    cargo build -q -p zenkey --example owner --example consume
    ZK2PY_PROTOC="$protoc" PYTHONPATH=impl/python \
        target/py-venv/bin/python -m zk2py.live_interop

# The Python implementation's hostid.v1 scenarios (spec/profiles/hostid/
# scenarios.md), in temporary roots, with an in-process zenoh-python router:
# no Rust build.
py-hostid:
    #!/usr/bin/env bash
    set -euo pipefail
    python3 impl/python/bootstrap.py target/py-venv > /dev/null
    PYTHONPATH=impl/python target/py-venv/bin/python -m zk2py.hostid_scenarios

# The Python implementation's freshness.v1 scenarios (spec/profiles/freshness/
# scenarios.md §1–§6), on an in-process zenoh-python router: no Rust build.
py-freshness:
    #!/usr/bin/env bash
    set -euo pipefail
    python3 impl/python/bootstrap.py target/py-venv > /dev/null
    PYTHONPATH=impl/python target/py-venv/bin/python -m zk2py.freshness_scenarios

# The Python implementation's health.v1 scenarios (spec/profiles/health/
# scenarios.md §1–§8), on in-process zenoh-python routers: no Rust build.
# §2, §4 and §8 wait out the 60 s horizon, so it takes about 8 minutes.
py-health:
    #!/usr/bin/env bash
    set -euo pipefail
    protoc="$(python3 impl/python/bootstrap.py target/py-venv | tail -n 1)"
    ZK2PY_PROTOC="$protoc" PYTHONPATH=impl/python \
        target/py-venv/bin/python -m zk2py.health_scenarios
