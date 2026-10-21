# zenkey — plain `cargo` remains the build system. This file only covers the
# things that need more than one command, chiefly running the GUI against
# traffic to look at.
#
#   just gui-demo     # the GUI, against the zk2 demo traffic. Start here.
#   just ci           # everything CI runs, in the same order
#
# ── The demo traffic ─────────────────────────────────────────────────────
#
# `spray` (zengui/examples/spray.rs) is a zk2 deployment in namespace
# `spray`: nav, camera, perception, teleop, thrusters, a planner and a tc
# backend, real services serving their contracts from examples/zk2/, a
# service implementing health.v1 alone, and `lab/meter`, whose metrics are
# telemetry.v1's (zenkey/tests/contracts/meter.v1.toml). Beside it runs the
# traffic no zk2 service would send: an interface token with no instance
# token, a second answerer on an owner's state, garbage on an owned key, an
# unowned key, a key in another namespace, and foreign keys.
#
# `just gui-demo` opens zengui on it with `--namespace spray`: the pump on a
# session in no namespace, presence and the contracts on one in the
# namespace (#614). What to look at:
#
#   * The Locator. The tree starts from the contract skeleton — every
#     resource of every address presence names, a declared row before a
#     byte arrives — each chunk labelled with its zk2 role, and it pivots
#     by system, address, iface and kind. Every key reads its identity
#     rung and its payload's conformance: valid, invalid (the squatter's),
#     undecodable (the garbage), `no provider` (the unowned key), `not in
#     namespace` (the other namespace's), `not zk2` (the v1 key), and
#     the other foreign keys unbadged.
#     Watched, a templated family (fleet/planner's `plans/{vehicle}`)
#     wears a budget badge, mostly "unobservable" and saying why.
#   * The Inspector. A state key's value is the owner's S4 GET, and a
#     stream's that declares history the owner's @adv cache; the declared
#     QoS against the observed axes; why, fields and consumers. A metric
#     of lab/meter (watch it) reads its kind and unit beside its type,
#     plots its value in that unit, and a counter its telemetry.v1 rate,
#     with a reset at each new term of the owner.
#   * The Workbench. Nodes per address from presence (select host-a/sysmon
#     for its health.v1 reading), the doctor (with `deep` on it finds the
#     spray's token-missing and state-stamp-foreign), Send (a foreign
#     publish, or robot1/nav's set_origin call), the admin space, the mesh.
#
# zenctl reads the same deployment resolved:
#
#   just ctl service list --namespace spray -c tcp/127.0.0.1:7449
#   just ctl doctor --namespace spray --deep --clocks-synced -c tcp/127.0.0.1:7449
#   just smoke-spray  # both, headless, against a spray on a free port
#
# ── Two more demos ───────────────────────────────────────────────────────
#
#   * `just gui-demo-bounded` trips *both* bounds immediately: the status strip
#     should read "N keys (+M retired — bound reached)" and, beside it,
#     "identities: N cached (+M retired — cache bound reached)" — two
#     bounds over two populations, two sentences (RFC 09 §5.1 O6).
#   * `just gui-demo-no-contracts` is the zk2 demo with nothing held offline:
#     the contracts cell counts every revision as retrieved from the spray's
#     own holders (spec §8.4), and the decodes are the same.
#
# `just --list` for the rest.

port := "7449"
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

# `--namespace spray` opens a session in the namespace for presence and the
# lens, and every zk2 key resolves to its identity rung and is checked
# through its contract — the four conformance poles on the echo rows and
# the tree's leaves, the contract decode in the Inspector. The contracts
# come offline from examples/zk2/.history.
# The GUI against the zk2 demo traffic — start here. Ctrl-C stops both.
gui-demo port=port: (_demo port "spray" "--namespace spray --contracts examples/zk2/.history" "50000")

# No contracts named: every revision a descriptor names is retrieved from the
# spray's own holders (spec §8.4), and the strip counts them as retrieved.
# The zk2 demo with no offline contracts: retrieval from the bus.
gui-demo-no-contracts port=port: (_demo port "spray" "--namespace spray" "50000")

# The zk2 demo with a key bound small enough to trip immediately.
gui-demo-bounded port=port: (_demo port "spray" "--namespace spray --contracts examples/zk2/.history" "6")

_demo port example flags max_keys:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p zengui --example {{example}}
    cargo build -p zengui
    mkdir -p {{rundir}}
    ./target/debug/examples/{{example}} -l tcp/127.0.0.1:{{port}} > {{rundir}}/{{example}}.log 2>&1 &
    spray=$!
    trap 'kill $spray 2>/dev/null || true' EXIT
    # Up to 30 s: the zk2 spray brings nine services up before it says so.
    for _ in $(seq 1 300); do
        grep -q 'Ctrl-C to stop' {{rundir}}/{{example}}.log 2>/dev/null && break
        kill -0 $spray 2>/dev/null || { echo "{{example}} exited early:"; cat {{rundir}}/{{example}}.log; exit 1; }
        sleep 0.1
    done
    echo "{{example}} publishing on tcp/127.0.0.1:{{port}} (log: {{rundir}}/{{example}}.log)"
    ./target/debug/zengui -c tcp/127.0.0.1:{{port}} \
        --max-keys {{max_keys}} {{flags}}

# Just the zk2 traffic: it listens as a router, so anything can connect to it.
spray port=port:
    cargo run -p zengui --example spray -- -l tcp/127.0.0.1:{{port}}

# The spray runs on a free port. The doctor exits 1, and that is the point:
# its findings (token-missing, state-stamp-foreign) are the spray's
# deliberate non-conformance.
# The zk2 spray, headless: zenctl's service list and doctor --deep against it.
smoke-spray:
    #!/usr/bin/env bash
    set -uo pipefail
    cargo build -p zengui --example spray
    cargo build -p zenctl
    mkdir -p {{rundir}}
    port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
    ep=tcp/127.0.0.1:$port
    ./target/debug/examples/spray -l "$ep" > {{rundir}}/spray-smoke.log 2>&1 &
    spray=$!
    trap 'kill $spray 2>/dev/null || true' EXIT
    for _ in $(seq 1 300); do
        grep -q 'Ctrl-C to stop' {{rundir}}/spray-smoke.log 2>/dev/null && break
        kill -0 $spray 2>/dev/null || { echo "spray exited early:"; cat {{rundir}}/spray-smoke.log; exit 1; }
        sleep 0.1
    done
    ./target/debug/zenctl service list --namespace spray -c "$ep"
    ./target/debug/zenctl doctor --namespace spray --deep --clocks-synced -c "$ep"
    echo "doctor exit $? (1: the findings above are the spray's own)"

# Just the GUI, against an already-running bus.
gui port=port namespace="spray":
    cargo run -p zengui -- -c tcp/127.0.0.1:{{port}} --namespace {{namespace}}

# The CLI, for cross-checking the GUI's numbers against the same engine.
ctl *ARGS:
    cargo run -p zenctl -- {{ARGS}}

# The live-bus tests, against the zk2 spray (#614, FL6b). Needs `just spray`
# running in another terminal; with no `ZENGUI_TEST_ENDPOINT` the same tests
# bring the spray up in process on an ephemeral port instead
# (`cargo test -p zengui --test live_bus -- --ignored`). The spray's own
# check is not here: `zengui/tests/spray_smoke.rs` runs in the ordinary
# suite, and `just smoke-spray` runs the real binaries.
test-live port=port namespace="spray":
    ZENGUI_TEST_ENDPOINT=tcp/127.0.0.1:{{port}} ZENGUI_TEST_NAMESPACE={{namespace}} \
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
    # cargo-nextest (`cargo install cargo-nextest --locked`), as CI: the
    # suite's waits overlap across binaries (.config/nextest.toml).
    cargo nextest run --workspace --locked --profile ci
    cargo test --workspace --doc --locked
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
    cargo check -p zenkey --no-default-features --locked
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

# The Python implementation's budget scenarios (spec/scenarios/budget.md
# §1–§7): §1–§3 with routers in processes of their own, §4–§7 against
# zk2py's owner. No Rust build; pass `--owner <exe>` to run §4–§7 against
# the Rust owner example as well.
py-budget *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    protoc="$(python3 impl/python/bootstrap.py target/py-venv | tail -n 1)"
    ZK2PY_PROTOC="$protoc" PYTHONPATH=impl/python \
        target/py-venv/bin/python -m zk2py.budget_scenarios {{ARGS}}

# The Python implementation's state scenarios (spec/scenarios/state.md §10,
# a late joiner's seed, 0.26) against zk2py's owner. No Rust build.
py-state:
    #!/usr/bin/env bash
    set -euo pipefail
    protoc="$(python3 impl/python/bootstrap.py target/py-venv | tail -n 1)"
    ZK2PY_PROTOC="$protoc" PYTHONPATH=impl/python \
        target/py-venv/bin/python -m zk2py.state_scenarios

# The Python implementation's health.v1 scenarios (spec/profiles/health/
# scenarios.md §1–§8), on in-process zenoh-python routers: no Rust build.
# §2, §4 and §8 wait out the 60 s horizon, so it takes about 8 minutes.
py-health:
    #!/usr/bin/env bash
    set -euo pipefail
    protoc="$(python3 impl/python/bootstrap.py target/py-venv | tail -n 1)"
    ZK2PY_PROTOC="$protoc" PYTHONPATH=impl/python \
        target/py-venv/bin/python -m zk2py.health_scenarios

# The Python implementation's config.v1 scenarios (spec/profiles/config/
# scenarios.md §1–§14) against zk2py's own owner, server and client, on
# in-process zenoh-python routers: no Rust build. About 6 minutes.
py-config:
    #!/usr/bin/env bash
    set -euo pipefail
    protoc="$(python3 impl/python/bootstrap.py target/py-venv | tail -n 1)"
    ZK2PY_PROTOC="$protoc" PYTHONPATH=impl/python \
        target/py-venv/bin/python -m zk2py.config_scenarios

# The Python implementation's link.v1 scenarios (spec/profiles/link/
# scenarios.md §1–§6): a face policy compiled into zenoh-python routers'
# blocks, and what crosses each face measured. No Rust build.
py-link:
    #!/usr/bin/env bash
    set -euo pipefail
    protoc="$(python3 impl/python/bootstrap.py target/py-venv | tail -n 1)"
    ZK2PY_PROTOC="$protoc" PYTHONPATH=impl/python \
        target/py-venv/bin/python -m zk2py.link_scenarios

# The Python implementation's alarms.v1 scenarios (spec/profiles/alarms/
# scenarios.md §1–§10) on in-process zenoh-python routers, against zk2py's
# owner, or the Rust owner example with `--owner <path>` (or ZK2PY_OWNER),
# which this recipe never builds. §2, §4, §6 and §10 run on a 4 s copy of the
# contract unless `--full`; `--only N`, `--ttl S` and `--quick` pass through.
py-alarms *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    protoc="$(python3 impl/python/bootstrap.py target/py-venv | tail -n 1)"
    ZK2PY_PROTOC="$protoc" PYTHONPATH=impl/python \
        target/py-venv/bin/python -m zk2py.alarms_scenarios {{ARGS}}

# The Python implementation's telemetry.v1 scenarios (spec/profiles/telemetry/
# scenarios.md §1–§9) on in-process zenoh-python routers, §8 also across a
# face that denies @zk, against zk2py's owner, or the Rust owner example with
# `--owner <path>` (or ZK2PY_OWNER), which this recipe never builds; with
# `--tool <zenctl>` (or ZK2PY_ZENCTL), zenctl telemetry judges beside zk2py's
# reader. About a minute, two with the tool. `--only N` passes through.
py-telemetry *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    protoc="$(python3 impl/python/bootstrap.py target/py-venv | tail -n 1)"
    ZK2PY_PROTOC="$protoc" PYTHONPATH=impl/python \
        target/py-venv/bin/python -m zk2py.telemetry_scenarios {{ARGS}}
