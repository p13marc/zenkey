import re, sys
# condense zenoh trace lines: time, thread, span chain, target, message
pat = re.compile(r'^\S+T\d\d:\d\d:(\S+)Z\s+(\w+)\s+(\S+)\s+ThreadId\((\d+)\)\s+(.*)$')
for line in open(sys.argv[1], errors='replace'):
    line = re.sub(r'\x1b\[[0-9;]*m', '', line.rstrip('\n'))
    m = pat.match(line)
    if not m:
        if line.startswith('result'):
            print(line[:300])
        continue
    t, lvl, th, tid, rest = m.groups()
    rest = re.sub(r'zk2/h1/(svc\d)/@zk/(instance|alive|member)/[^ }"]*', lambda mm: f'{mm.group(1)}:{mm.group(2)}', rest)
    rest = re.sub(r'zenoh::net::routing::', 'r::', rest)
    rest = re.sub(r'zenoh::api::', 'api::', rest)
    rest = re.sub(r'make_resource\{[^}]*\}', 'mkres', rest)
    print(f'{t} {lvl[0]} {th}:{tid} {rest[:int(sys.argv[2]) if len(sys.argv)>2 else 400]}')
