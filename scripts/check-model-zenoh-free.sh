#!/usr/bin/env bash
# zenkey-model is the session-free half of zk2 (#608): build scripts, CI
# tools and the conformance runners stand on it, so it may depend on
# zenoh-keyexpr (and the zenoh-result that crate pulls in) and on no other
# zenoh crate. A `zenoh` in its normal dependency tree fails here.
set -euo pipefail
cd "$(dirname "$0")/.."
bad=$(cargo tree -p zenkey-model -e normal --prefix none --locked \
    | awk '{print $1}' | grep -E '^zenoh' | grep -vxE 'zenoh-keyexpr|zenoh-result' | sort -u || true)
if [ -n "$bad" ]; then
    echo "zenkey-model depends on zenoh crates beyond zenoh-keyexpr:" >&2
    echo "$bad" >&2
    exit 1
fi
echo "zenkey-model: zenoh-free (zenoh-keyexpr only)"
