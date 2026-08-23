#!/usr/bin/env bash
# The update thread does not touch the disk (#255).
#
# `update` runs on the thread iced renders from, so every synchronous
# filesystem call inside it is a frame the window does not paint — and one of
# the eight #255 found (the `.zrec` parse) had no upper bound at all. The fix
# has one shape: a `services::` free function returning `Task<Message>`, and
# a message the result lands on.
#
# The rule is mechanical, so this is the gate: nothing under `zengui/src/update`
# — nor in `app.rs`, the shell that routes into it — names `std::fs`, opens or
# creates a `File`, reads or writes the shared context store, or persists the
# preferences. `context_store::config_path()` stays legal: it computes a path
# and touches nothing.
set -euo pipefail

hits=$(grep -rnE 'std::fs|File::(open|create)|context_store::(load|save|upsert)|Prefs::save|prefs\.save|save_to\(|ReplayState::load' \
    zengui/src/update zengui/src/app.rs || true)
if [ -n "$hits" ]; then
    echo "fs seam: the update thread reaches for the filesystem (#255):"
    echo "$hits"
    echo
    echo "A handler that needs the disk needs a services:: function returning"
    echo "Task<Message>, and a message for the result to land on —"
    echo "services/context.rs, services/prefs.rs and services/record.rs are"
    echo "the models. The pane states what is in flight meanwhile (O4)."
    exit 1
fi
echo "fs seam: no filesystem calls on the update thread (#255)."
