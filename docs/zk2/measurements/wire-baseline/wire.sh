#!/usr/bin/env bash
# #989's wire baseline: bring a simulated radio path up or down, with no root.
#
#   zk2 owner (client) -> zenohd A -> driver A ~udp~ relay ~udp~ driver B -> zenohd B -> readers (clients)
#
# Usage: wire.sh up <medium> <stack> | down | status | cut | restore | loss <p> | start <name> <cmd...>
#   <medium>: rf, rf-zk2c, sf7, sf7-zk2c, sf9 or sf12 (render.py's MEDIA)
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
#   DICT  the compress dictionary (default rf.zdict here);
#   LOSS_START  the relay's loss until told otherwise (default the medium's);
#   CORES  the cores every process runs on (default 0-1; #989's zk2c runs
#         took 4-5, the lanes' builds holding 0-3);
#   FACE_POLICY  a link.v2 face policy (#989's zk2c runs): `zenctl acl gen`
#         compiles it with zenoh-modem's enrollment, contracts and catalog, as
#         the policy's header says, and render.py puts the plan on router A,
#         with usrpwd credentials made once per WORK under WORK/cfg/auth
#         (users.txt; rf-node.json5, the owner's session config; local.json5,
#         the session config of every client of router B). The far side
#         then opens the lane, and router A authenticates it as `ground`.
#
# Every process runs on CORES under `timeout`, and its PID (timeout's)
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
CORES=${CORES:-0-1}
REPO=${REPO:-$(cd "$HERE/../../../.." && pwd)}

start() { # name, command...
  local name=$1; shift
  mkdir -p "$LOGS" "$RUN"
  timeout "$LIFE" taskset -c "$CORES" "$@" </dev/null >"$LOGS/$name.log" 2>&1 &
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

face() { # router A's face block and the credentials its subjects need
  local m=$REPO/examples/zk2/zenoh-modem auth=$WORK/cfg/auth u
  # Test credentials, made once per WORK and never committed: the node
  # principals of zenoh-modem's enrollment, the ground, and its tools.
  (umask 077 && mkdir -p "$auth" && touch "$auth/users.txt"
   for u in rf-node lora-node ground local; do
     [[ -e $auth/$u.pw ]] && continue
     pw=$(python3 -I -c 'import secrets; print(secrets.token_hex(16))')
     echo "$u:$pw" >>"$auth/users.txt"
     echo "$pw" >"$auth/$u.pw"
     echo "{ transport: { auth: { usrpwd: { user: \"$u\", password: \"$pw\" } } } }" >"$auth/$u.json5"
   done)
  "$BIN/zenctl" acl gen --enrollment "$m/faces/zenoh-modem.enrollment.toml" --contracts "$m" \
    --contracts "$REPO/spec/profiles/config/config.v1.toml" \
    --contracts "$REPO/spec/profiles/health/health.v1.toml" --catalog "$m/catalog.json" \
    --face-policy "$FACE_POLICY" --json5 >"$CFG/face-router.json5"
  "$BIN/zenctl" acl gen --enrollment "$m/faces/zenoh-modem.enrollment.toml" --contracts "$m" \
    --contracts "$REPO/spec/profiles/config/config.v1.toml" \
    --contracts "$REPO/spec/profiles/health/health.v1.toml" --catalog "$m/catalog.json" \
    --face-policy "$FACE_POLICY" --far-config >"$CFG/face-far.json5"
}

render() { # medium, stack
  mkdir -p "$CFG" "$RUN/a" "$RUN/b" "$LOGS"
  local extra=()
  if [[ -n ${FACE_POLICY:-} ]]; then
    face
    extra=(--face "$CFG/face-router.json5" --auth "$WORK/cfg/auth")
  fi
  python3 -I "$HERE/render.py" --medium "$1" --stack "$2" --work "$WORK" --inst "$INST" \
    --base "$BASE" --out "$CFG" "${extra[@]}" >/dev/null
  mkdir -p "$LOGS/configs" && cp "$CFG"/driver-?.toml "$CFG"/zenohd-?.json5 "$CFG/params.json" "$LOGS/configs/"
  if [[ -n ${FACE_POLICY:-} ]]; then
    cp "$CFG/face-router.json5" "$CFG/face-far.json5" "$LOGS/configs/"
    # The run's record keeps no credential: router B's test password is redacted.
    sed -i 's/password: "[0-9a-f]*"/password: "<redacted>"/' "$LOGS/configs/zenohd-b.json5"
  fi
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
    taskset -c "$CORES" timeout 20 "$BIN/modem-driver" --config "$CFG/driver-$x.toml" --init-layer-state >/dev/null
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
    --loss "${LOSS_START:-$(param loss)}" --latency-ms "$(param latency_ms)" --jitter-ms "$(param jitter_ms)" \
    --seed 0 --capture "$LOGS/air.jsonl" --summary "$LOGS/relay-summary.json"
  sleep 0.5
  start driver-a env RUST_LOG="${DRIVER_LOG:-info}" "$BIN/modem-driver" --config "$CFG/driver-a.toml"
  start driver-b env RUST_LOG="${DRIVER_LOG:-info}" "$BIN/modem-driver" --config "$CFG/driver-b.toml"
  if [[ -n ${FACE_POLICY:-} ]]; then
    # The far side opens the lane (render.py --face): A listens, B dials.
    wait_for "$RUN/b/rfb/a/air" 10
    start zenohd-a "$ZENOHD" -c "$CFG/zenohd-a.json5"
    sleep 1
    start zenohd-b "$ZENOHD" -c "$CFG/zenohd-b.json5"
  else
    wait_for "$RUN/a/rfa/b/air" 10
    start zenohd-b "$ZENOHD" -c "$CFG/zenohd-b.json5"
    sleep 1
    start zenohd-a "$ZENOHD" -c "$CFG/zenohd-a.json5"
  fi
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

control() { # cut | restore | "loss <p>": the relay drops every datagram both ways, stops, or takes a loss
  python3 -I -c 'import socket,sys; socket.socket(socket.AF_INET, socket.SOCK_DGRAM).sendto(sys.argv[1].encode(), ("127.0.0.1", int(sys.argv[2])))' \
    "$1" "$((BASE + 14))"
  echo "$1 sent"
}

case ${1:-} in
  up) up "${2:?medium: rf, rf-zk2c, sf7, sf9 or sf12}" "${3:?stack: bare or lean}" ;;
  down) down ;;
  status) status ;;
  cut | restore) control "$1" ;;
  loss) control "loss ${2:?a loss, 0 to 1}" ;;
  start) shift; start "$@" ;;
  *) echo "usage: $0 up <medium> <stack> | down | status | cut | restore | start <name> <cmd...>" >&2; exit 2 ;;
esac
