#!/usr/bin/env bash
# #989's wire baseline: bring a simulated radio path up or down, with no root.
#
#   zk2 owner (client) -> zenohd A -> driver A ~udp~ relay ~udp~ driver B -> zenohd B -> readers (clients)
#
# Usage: wire.sh up <medium> <stack> | down | status | cut | restore | start <name> <cmd...>
#   <medium>: rf, sf7, sf9 or sf12 (render.py's MEDIA)
#   <stack>:  bare (no layers) or lean (compress with a 1 KB dictionary, aead
#             aes-128-ccm with an 8-byte tag, a 2-byte overhead for address;
#             the fragment layer under it at SF12)
#
# Environment:
#   WORK  (default /srv/dev/targets/zenkey-989) holds the binaries (WORK/bin:
#         modem-driver, zenctl, owner; WORK/zenohd/bin/zenohd, the fork) and
#         everything a run writes;
#   INST  (default rf) names this path, so two can run at once: its sockets,
#         configs and aead state live under WORK/{run,cfg,state}/INST;
#   BASE  (default 17400) its ports: routers BASE+1 and +2, drivers +10 and
#         +11, the relay +12, +13 and its control port +14;
#   LOGS  where this run's logs go (default WORK/logs/INST);
#   DICT  the compress dictionary (default rf.zdict here).
#
# Every process runs on cores 0-1 under `timeout`, and its PID (timeout's)
# goes to WORK/run/INST/pids, which `down` signals and nothing else.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
WORK=${WORK:-/srv/dev/targets/zenkey-989}
INST=${INST:-rf}
BASE=${BASE:-17400}
BIN=${BIN:-$WORK/bin}
ZENOHD=${ZENOHD:-$WORK/zenohd/bin/zenohd}
CFG=$WORK/cfg/$INST
RUN=$WORK/run/$INST
LOGS=${LOGS:-$WORK/logs/$INST}
DICT=${DICT:-$HERE/rf.zdict}
PIDS=$RUN/pids
LIFE=${LIFE:-3h}

start() { # name, command...
  local name=$1; shift
  mkdir -p "$LOGS" "$RUN"
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

param() { python3 -I -c "import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])" "$CFG/params.json" "$1"; }

render() { # medium, stack
  mkdir -p "$CFG" "$RUN/a" "$RUN/b" "$LOGS"
  python3 -I "$HERE/render.py" --medium "$1" --stack "$2" --work "$WORK" --inst "$INST" \
    --base "$BASE" --out "$CFG" >/dev/null
  mkdir -p "$LOGS/configs" && cp "$CFG"/driver-?.toml "$CFG"/zenohd-?.json5 "$CFG/params.json" "$LOGS/configs/"
  [[ $2 == lean ]] || return 0
  cp "$DICT" "$CFG/rf.zdict"
  if [[ ! -e $WORK/cfg/rf.keys ]]; then
    # A test key, made once per WORK and never committed: 0600 or aead refuses it.
    (umask 077 && python3 -I -c 'import secrets; print("0", secrets.token_hex(32))' >"$WORK/cfg/rf.keys")
  fi
  local x
  for x in a b; do
    mkdir -p "$WORK/state/$INST/$x" && chmod 700 "$WORK/state" "$WORK/state/$INST" "$WORK/state/$INST/$x"
    # Creates the nonce counters once; later runs keep them, so no nonce repeats.
    taskset -c 0-1 timeout 20 "$BIN/modem-driver" --config "$CFG/driver-$x.toml" --init-layer-state >/dev/null
  done
}

up() {
  [[ -s $PIDS ]] && { echo "already up: $PIDS" >&2; exit 2; }
  render "$1" "$2"
  rm -f "$RUN"/a/mgmt.sock "$RUN"/b/mgmt.sock
  : >"$PIDS"
  start relay python3 -I "$HERE/relay.py" \
    --a-side "127.0.0.1:$((BASE + 12))" --b-side "127.0.0.1:$((BASE + 13))" \
    --driver-a "127.0.0.1:$((BASE + 10))" --driver-b "127.0.0.1:$((BASE + 11))" \
    --control "127.0.0.1:$((BASE + 14))" \
    --loss "$(param loss)" --latency-ms "$(param latency_ms)" --jitter-ms "$(param jitter_ms)" \
    --seed 0 --capture "$LOGS/air.jsonl" --summary "$LOGS/relay-summary.json"
  sleep 0.5
  start driver-a env RUST_LOG="${DRIVER_LOG:-info}" "$BIN/modem-driver" --config "$CFG/driver-a.toml"
  start driver-b env RUST_LOG="${DRIVER_LOG:-info}" "$BIN/modem-driver" --config "$CFG/driver-b.toml"
  wait_for "$RUN/a/rfa/b/air" 10
  start zenohd-b "$ZENOHD" -c "$CFG/zenohd-b.json5"
  sleep 1
  start zenohd-a "$ZENOHD" -c "$CFG/zenohd-a.json5"
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

control() { # cut | restore: the relay drops every datagram both ways, or stops
  python3 -I -c 'import socket,sys; socket.socket(socket.AF_INET, socket.SOCK_DGRAM).sendto(sys.argv[1].encode(), ("127.0.0.1", int(sys.argv[2])))' \
    "$1" "$((BASE + 14))"
  echo "$1 sent"
}

case ${1:-} in
  up) up "${2:?medium: rf, sf7, sf9 or sf12}" "${3:?stack: bare or lean}" ;;
  down) down ;;
  status) status ;;
  cut | restore) control "$1" ;;
  start) shift; start "$@" ;;
  *) echo "usage: $0 up <medium> <stack> | down | status | cut | restore | start <name> <cmd...>" >&2; exit 2 ;;
esac
