#!/usr/bin/env bash
# Condense one trial's trace (zenoh::net::routing=debug,zenoh::api::session=trace)
# from the reader's first liveliness subscriber to the DeclareFinal of the
# first broad GET. Usage: ./excerpt.sh trial-*.log
# Abbreviations: svcN:instance|alive|member = the trial's token keys;
# CI(<zid> #<id> <mode>) = the client's pending current interest (the session's
# interest id); router-side lines are `self=south:0:client`, the reader's own
# routing is `self=north` (its client hat) and `self=local` (its session face).
d=$(dirname "$0")
python3 -I "$d/condense.py" "$1" 600 \
 | grep -v 'mkres\|dispatcher::face: expr=\|return=()$\|propagate_token{self=south:0:peer\|router.token\|register_token{self=south' \
 | sed -E 's/CurrentInterest \{ src: Remote\(FaceState \{ id: [0-9]+, zid: ([0-9a-f]{8})[0-9a-f]*, region: [A-Za-z]+ \}\), src_region: [A-Za-z]+, src_interest_id: ([0-9]+), mode: ([A-Za-z]+) \}/CI(\1 #\2 \3)/g; s/r::hat::(broker|client|router|peer)::(interests|token)/\1.\2/g; s/r::dispatcher::/disp./g; s/other_matches=\{[^}]*\}/om={..}/' \
 | awk '/declare_liveliness_subscriber/ && !started {started=1} started {print}
        /liveliness.get\(zk2\/\*\/\*\/@zk\/\*\*/ && !g {g=1; want=1}
        want==1 && /Register liveliness query/ {q=$NF; want=2}
        want==2 && index($0, "recv DeclareFinal Some(" q ")") {for(i=0;i<12;i++){if((getline l)>0) print l}; exit}' \
 | cut -c1-240
