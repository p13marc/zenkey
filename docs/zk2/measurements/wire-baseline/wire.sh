#!/usr/bin/env bash
# #989's wire baseline: bring the RF path up or down, with no root.
#
#   zk2 owner (client) -> zenohd A -> driver A ~udp~ relay ~udp~ driver B -> zenohd B -> reader (client)
#
# Usage: wire.sh up <bare|lean> | down | status | start <name> <cmd...>
#   bare: no layers. lean: compress (1 KB dictionary) + aead aes-128-ccm with
#   an 8-byte tag + a 2-byte overhead layer standing in for address.
#
# WORK (default /srv/dev/targets/zenkey-989) holds the binaries (WORK/bin:
# modem-driver, zenctl; WORK/zenohd/bin/zenohd, the fork), the rendered
# configs, the sockets, the aead state and the logs. Every process runs on
# cores 0-1 under `timeout`, and its PID (timeout's) goes to WORK/run/pids,
# which `down` signals and nothing else.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
WORK=${WORK:-/srv/dev/targets/zenkey-989}
BIN=${BIN:-$WORK/bin}
ZENOHD=${ZENOHD:-$WORK/zenohd/bin/zenohd}
CFG=$WORK/cfg
RUN=$WORK/run
LOGS=${LOGS:-$WORK/logs}
PIDS=$RUN/pids
LIFE=${LIFE:-2h}
LOSS=${LOSS:-0.05}

start() { # name, command...
  local name=$1; shift
  timeout "$LIFE" taskset -c 0-1 "$@" </dev/null >"$LOGS/$name.log" 2>&1 &
  echo "$! $name" >>"$PIDS"
  echo "started $name pid $!"
}

wait_for() { # path, seconds
  local i
  for ((i = 0; i < $2 * 10; i++)); do
    [[ -e $1 ]] && return 0
    sleep 0.1
  done
  echo "timed out waiting for $1" >&2
  return 1
}

render() { # stack
  mkdir -p "$CFG" "$RUN/a" "$RUN/b" "$LOGS"
  local f
  for f in "$HERE"/configs/*-"$1".*; do
    sed "s|@WORK@|$WORK|g" "$f" >"$CFG/$(basename "$f")"
  done
  [[ $1 == lean ]] || return 0
  cp "$HERE/rf.zdict" "$CFG/rf.zdict"
  if [[ ! -e $CFG/rf.keys ]]; then
    # A test key, made once per WORK and never committed: 0600 or aead refuses it.
    (umask 077 && python3 -I -c 'import secrets; print("0", secrets.token_hex(32))' >"$CFG/rf.keys")
  fi
  local x
  for x in a b; do
    mkdir -p "$WORK/state/$x" && chmod 700 "$WORK/state/$x"
    # Creates the nonce counters once; later runs keep them, so no nonce repeats.
    taskset -c 0-1 timeout 20 "$BIN/modem-driver" --config "$CFG/driver-$x-lean.toml" --init-layer-state >/dev/null
  done
}

up() {
  local stack=$1
  [[ -s $PIDS ]] && { echo "already up: $PIDS" >&2; exit 2; }
  render "$stack"
  rm -f "$RUN"/a/mgmt.sock "$RUN"/b/mgmt.sock
  : >"$PIDS"
  start relay python3 -I "$HERE/relay.py" \
    --a-side 127.0.0.1:17412 --b-side 127.0.0.1:17413 \
    --driver-a 127.0.0.1:17410 --driver-b 127.0.0.1:17411 \
    --loss "$LOSS" --latency-ms 150 --jitter-ms 30 --seed 0 \
    --capture "$LOGS/air.jsonl" --summary "$LOGS/relay-summary.json"
  sleep 0.5
  start driver-a env RUST_LOG="${DRIVER_LOG:-info}" "$BIN/modem-driver" --config "$CFG/driver-a-$stack.toml"
  start driver-b env RUST_LOG="${DRIVER_LOG:-info}" "$BIN/modem-driver" --config "$CFG/driver-b-$stack.toml"
  wait_for "$RUN/a/rfa/b/air" 10
  start zenohd-b "$ZENOHD" -c "$CFG/zenohd-b-$stack.json5"
  sleep 1
  start zenohd-a "$ZENOHD" -c "$CFG/zenohd-a-$stack.json5"
}

down() {
  [[ -f $PIDS ]] || { echo "nothing recorded"; return 0; }
  # The reverse of start order: readers and owners, routers, drivers, relay.
  tac "$PIDS" | while read -r pid name; do
    if kill -0 "$pid" 2>/dev/null; then
      kill -TERM "$pid" && echo "stopped $name pid $pid"
      sleep 0.3
    fi
  done
  sleep 1
  tac "$PIDS" | while read -r pid name; do
    if kill -0 "$pid" 2>/dev/null; then
      echo "still alive: $name pid $pid" >&2
    fi
  done
  mv "$PIDS" "$PIDS.done.$(date +%s)"
}

status() {
  [[ -f $PIDS ]] || { echo "nothing recorded"; return 0; }
  while read -r pid name; do
    if kill -0 "$pid" 2>/dev/null; then echo "up   $name $pid"; else echo "gone $name $pid"; fi
  done <"$PIDS"
}

case ${1:-} in
  up) up "${2:?stack: bare or lean}" ;;
  down) down ;;
  status) status ;;
  start) shift; start "$@" ;;
  *) echo "usage: $0 up <bare|lean> | down | status | start <name> <cmd...>" >&2; exit 2 ;;
esac
