#!/usr/bin/env bash
# The measurement matrix, one part per command (each part well under ten
# minutes). Usage: ./run-matrix.sh <1.10.1|main|fix> <part>
# Every cell appends its raw result lines and a "# drive" line to
# results-<ver>.txt; `python3 -I summary.py results-*.txt` prints the tables
# (README.md).
set -euo pipefail
cd "$(dirname "$0")"
ver=$1
part=$2
case $ver in
  1.10.1) bin=target-1.10.1/release/lgr ;;
  main) bin=target-main/release/lgr ;;
  # zenoh 1.10.1 with fix.patch (vfix/): the intervention.
  fix) bin=target-fix/release/lgr ;;
  *) echo "version: 1.10.1, main or fix" >&2; exit 2 ;;
esac
out=results-$ver.txt
n=${TRIALS:-720}
cell() { "$bin" drive --jobs 12 --out "$out" "$@"; }

case $part in
  # Tokens held before the reader subscribes; router, holder and reader in
  # one fresh process per trial (zenkey's test-suite shape).
  tf-ab)
    for w in 0 10 100 samples narrow; do cell --trials "$n" --shape A --order tokens-first --wait "$w"; done
    for w in 0 10 100 narrow; do cell --trials "$n" --shape B --order tokens-first --wait "$w"; done ;;
  tf-cd)
    for w in 0 10 100 samples narrow; do cell --trials "$n" --shape C --order tokens-first --wait "$w"; done
    for w in 0 10 100 narrow; do cell --trials "$n" --shape D --order tokens-first --wait "$w"; done ;;
  # The reader subscribes (and round-trips the router) before the tokens
  # appear: the subscribers learn them as live declarations.
  sf)
    for s in A B C; do for w in 0 samples narrow; do cell --trials "$n" --shape "$s" --order subs-first --wait "$w"; done; done
    for w in 0 narrow; do cell --trials "$n" --shape D --order subs-first --wait "$w"; done ;;
  # A router process and a holder process shared by every trial; each trial
  # is a fresh reader process (subs-first: it also holds its own tokens).
  shared)
    for w in 0 10 100 samples; do cell --shared --trials "$n" --shape A --order tokens-first --wait "$w"; done
    for w in 0 10 100; do cell --shared --trials "$n" --shape B --order tokens-first --wait "$w"; done
    for w in 0 10 100 samples; do cell --shared --trials "$n" --shape C --order tokens-first --wait "$w"; done
    for w in 0 10 100; do cell --shared --trials "$n" --shape D --order tokens-first --wait "$w"; done
    for s in A B C; do cell --shared --trials "$n" --shape "$s" --order subs-first --wait samples; done
    for s in A B; do cell --shared --trials "$n" --shape "$s" --order subs-first --wait narrow; done
    cell --shared --trials "$n" --shape D --order subs-first --wait narrow
    for s in A B C D; do cell --shared --trials "$n" --shape "$s" --order subs-first --wait 0; done ;;
  shared0)
    for s in A B C D; do cell --shared --trials "$n" --shape "$s" --order subs-first --wait 0; done ;;
  # Six spinning threads beside the twelve trials.
  burn)
    cell --burn 6 --trials "$n" --shape A --order tokens-first --wait samples
    cell --burn 6 --trials "$n" --shape A --order tokens-first --wait 100
    cell --burn 6 --trials "$n" --shape C --order tokens-first --wait samples
    for s in A B C; do cell --burn 6 --trials "$n" --shape "$s" --order subs-first --wait samples; done
    cell --burn 6 --trials "$n" --shape D --order subs-first --wait narrow
    cell --burn 6 --shared --trials "$((n * 2))" --shape A --order tokens-first --wait samples
    for s in A B; do cell --burn 6 --shared --trials "$((n * 2))" --shape "$s" --order subs-first --wait samples; done ;;
  # The rest of `burn` on main: the 2026-10-24 reboot stopped main's run in
  # its sixth cell (C subs-first samples burn6) after 648 of 720 trials,
  # which stay in results-main.txt; this tops the cell up to 720.
  burn-rest)
    cell --burn 6 --trials "$((n / 10))" --shape C --order subs-first --wait samples
    cell --burn 6 --trials "$n" --shape D --order subs-first --wait narrow
    cell --burn 6 --shared --trials "$((n * 2))" --shape A --order tokens-first --wait samples
    for s in A B; do cell --burn 6 --shared --trials "$((n * 2))" --shape "$s" --order subs-first --wait samples; done ;;
  # What the mechanism (README) predicts: a delay closes the in-flight window
  # but not the overtaken DeclareFinal, so A and B still miss at 100 ms under
  # load; B misses only tokens declared after it subscribed (tokens-first: 0);
  # C and D never miss.
  delay)
    for s in A B; do cell --trials "$n" --shape "$s" --order subs-first --wait 100; done
    for s in A B; do cell --burn 6 --trials "$n" --shape "$s" --order subs-first --wait 100; done
    cell --burn 6 --trials "$n" --shape B --order tokens-first --wait 100
    cell --burn 6 --trials "$n" --shape C --order subs-first --wait 0
    cell --burn 6 --trials "$n" --shape D --order subs-first --wait 0
    cell --burn 6 --trials "$n" --shape D --order tokens-first --wait 100 ;;
  # The cells that miss most on 1.10.1 and main, for `fix`: if the mechanism
  # is right they all go to zero.
  fix-check)
    cell --trials "$n" --shape A --order tokens-first --wait 0
    for s in A B; do cell --trials "$n" --shape "$s" --order subs-first --wait 0; done
    cell --burn 6 --trials "$n" --shape A --order tokens-first --wait samples
    cell --burn 6 --trials "$n" --shape A --order tokens-first --wait 100
    for s in A B; do cell --burn 6 --trials "$n" --shape "$s" --order subs-first --wait samples; done
    for s in A B; do cell --burn 6 --trials "$n" --shape "$s" --order subs-first --wait 100; done ;;
  # Larger counts where the rate is about one in a hundred.
  big)
    cell --trials "$((n * 4))" --shape A --order tokens-first --wait samples
    cell --trials "$((n * 4))" --shape A --order subs-first --wait narrow
    cell --trials "$((n * 4))" --shape B --order subs-first --wait samples ;;
  *) echo "part: tf-ab tf-cd sf shared burn burn-rest delay fix-check big (shared0: the last four of shared)" >&2; exit 2 ;;
esac
