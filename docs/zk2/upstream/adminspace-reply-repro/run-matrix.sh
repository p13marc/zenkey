#!/usr/bin/env bash
# The measurement matrix (README.md). One trial per line of output, each a
# fresh router process; hung trials keep their stacks under stacks-<ver>/.
#
#   ./run-matrix.sh 1.10.1 >> results-1.10.1.txt
#   ./run-matrix.sh fix    >> results-fix.txt
set -u
cd "$(dirname "$0")"
ver=${1:?usage: run-matrix.sh 1.10.1|fix}
case $ver in
  1.10.1) bin=./target-1.10.1/release/asr ;;
  fix) bin=./target-fix/release/asr ;;
  *) echo "unknown version $ver" >&2; exit 2 ;;
esac
STORAGES='@/*/router/**/storage_manager/storages/**'
NARROW='@/*/router/status/plugins/storage_manager/storages/**'
n=0
cell() { # trials, then the trial's own flags
  local trials=$1; shift
  echo "# cell $*"
  for _ in $(seq "$trials"); do
    n=$((n + 1))
    timeout 60 "$bin" trial --trial "$ver-$n" --for 15 --stall 5 \
      --stacks "scratch/stacks-$ver" "$@" < /dev/null 2>/dev/null \
      || echo "result ver=$ver trial=$ver-$n outcome=error $*"
  done
}
# A: how much the reply loop holds (subscribers, every churn).
for load in 0 1 10 100 500; do cell 10 --load $load; done
# B: every kind of declaration record.
for kind in publisher queryable querier token; do
  cell 5 --selector "@/*/router/$kind/**"
done
# C: every kind of writer.
for churn in sub lsub token connect; do cell 5 --churn $churn; done
# D: controls: no writer, and the keys answered without the tables' lock
# held across a reply (or with nothing to send under it).
cell 5 --churn none
cell 5 --selector '@/*/router'
cell 5 --selector '@/*/*'
cell 5 --selector '@/*/router/linkstate/*'
cell 3 --selector '@/*/router/route/successor/**'
# E: the storages selector zenkey's tools read, beside records ending in
# `**` and beside none, and the narrowed one.
cell 10 --selector "$STORAGES" --shape wild
cell 5 --selector "$STORAGES" --shape concrete
cell 5 --selector "$NARROW" --shape wild
# F: the whole admin space.
cell 5 --selector '@/**'
