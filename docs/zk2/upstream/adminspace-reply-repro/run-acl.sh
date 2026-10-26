#!/usr/bin/env bash
# The access-control matrix (README.md, "Under access control"; zenkey
# #894): the router under `default_permission: deny`, the asker a tool
# principal holding zk2's admin read, core 0.15's (`wide`) or 0.50's
# (`narrow`), beside 500 records of each kind and a client declaring flat
# out. One trial per line of output, each a fresh router process; hung
# trials keep their stacks under $STACKS (default scratch/stacks-acl).
#
#   ./run-acl.sh >> results-acl.txt
#   ASR=/path/to/asr ./run-acl.sh >> results-acl.txt
set -u
cd "$(dirname "$0")"
bin=${ASR:-./target-1.10.1/release/asr}
stacks=${STACKS:-scratch/stacks-acl}
STORAGES='@/*/router/**/storage_manager/storages/**'
NARROW='@/*/router/status/plugins/storage_manager/storages/**'
n=0
cell() { # trials, then the trial's own flags
  local trials=$1; shift
  echo "# cell $*"
  for _ in $(seq "$trials"); do
    n=$((n + 1))
    # The cell's own flags first: `asr` reads the first of each.
    timeout 120 "$bin" trial "$@" --trial "acl-$n" --for 15 --stall 5 \
      --stacks "$stacks" < /dev/null 2>/dev/null \
      || echo "result ver=1.10.1 trial=acl-$n outcome=error $*"
  done
}
for acl in wide narrow; do
  # A: the declaration records, every kind, and the wider GETs a script
  # with a tool's credentials could type.
  cell 10 --acl $acl --selector '@/*/router/subscriber/**'
  for kind in publisher queryable querier token; do
    cell 5 --acl $acl --selector "@/*/router/$kind/**"
  done
  cell 5 --acl $acl --selector '@/*/router/linkstate/*'
  cell 5 --acl $acl --selector '@/*/router/**'
  cell 5 --acl $acl --selector '@/**'
  cell 5 --acl $acl --selector "$STORAGES" --shape wild
  # B: the tools' own reads beside the same churn.
  cell 5 --acl $acl --selector '@/*/router'
  cell 5 --acl $acl --selector '@/*/*'
  cell 5 --acl $acl --selector "$NARROW" --shape wild
done
# C (narrow only): every kind of writer, beside a declaration-record GET.
for churn in sub lsub token connect; do
  cell 5 --acl narrow --selector '@/*/router/subscriber/**' --churn $churn
done
