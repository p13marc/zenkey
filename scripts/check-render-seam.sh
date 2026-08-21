#!/usr/bin/env bash
# The render seam, gated (#198).
#
# `zenctl` renders one report three ways, and exactly one place decides which:
# `render::Mode::of`. Before the seam existed the decision was re-derived at 40
# sites, six of which collapsed `Json` and `Ndjson` into one arm — which is how
# `zenctl node info --format ndjson` came to emit a pretty multi-line document
# that cannot be read a line at a time.
#
# A second `resolved()` is not necessarily wrong. It is necessarily a decision
# somebody should look at, which is what a gate is for.
set -euo pipefail

found=$(grep -ro 'format\.resolved()' zenctl/src | wc -l)
if [ "$found" -ne 1 ]; then
    echo "render seam: expected exactly 1 call to format.resolved(), found $found:"
    grep -rn 'format\.resolved()' zenctl/src
    echo
    echo "The one place that resolves a Format is render::Mode::of. A verb that"
    echo "needs to know whether a program is reading it asks Mode::machine();"
    echo "a verb with a report in hand hands it to render::emit."
    exit 1
fi
echo "render seam: one format decision (#198)."
