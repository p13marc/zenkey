# zenctl changelog

`zenctl` ships as Forgejo release binaries, not on crates.io (0.1.x stays
there un-yanked). What that buys is the ability to fix a command tree instead
of carrying it — and what it costs is this file, which has to be complete
enough that a script written against the old spellings can be moved in one
sitting.

## Unreleased (`main`, zk2) — the doctor rules' clocks (#735, a PF follow-up)

| Before | Now | Notes |
|---|---|---|
| `watchdog --rule 'doctor health-*'`: always unobservable | `watchdog --clocks-synced` | The rule's doctor listens to no status, so a `health.v1` status reply's age needs the operator's word that the clocks agree within the HLC delta, as `doctor --clocks-synced`. Without the flag, such a rule still reads unobservable, its reason naming the flag |
| `record --on 'doctor health-*'`: always unobservable | `record --on … --clocks-synced` | The same word for a trigger capture's doctor rules; it requires `--on` |

## Unreleased (`main`, zk2) — `health.v1` and `hostid.v1`'s cloned ids (#721, PF)

A judgement verb over `health.v1` (`spec/profiles/health/v1.md`, text
0.3), and the doctor and `check conform` asking it; the doctor also judges
two sessions claiming one minted address (`spec/profiles/hostid/v1.md`
§2.12).

| Before | Now | Notes |
|---|---|---|
| — | `health [SYSTEM/SERVICE] [--clocks-synced] [--for SECS] [--grace SECS] [--across-face crosses\|denied] [--namespace NS]` | §2.11's reader: presence and descriptors (a service implements `health.v1` when its descriptor lists it, token or not), two readings — each one GET of `health.v1/state/**` answering a status and its checks together — after a window's subscription to every status and to the faults. Per service, `verdict` is `healthy`, `unhealthy` (at its `level`), `stale` (never a level), `unobservable` or `not_asked`, with `readings`, `agrees` (a break of §2.2 is `no` only in both readings), `clock_ahead`, the status and checks as read, and an archive's `last_known` for an absent owner, never current; a `rollup`. Rows tagged `service`; the envelope carries `judgement`. Exit 1 on unhealthy, stale, a break or a clock ahead; 0 every service asked healthy; 2 a service unobservable, none asked, or a run that could not start. `--for` defaults to 31 s, or none with `--clocks-synced` |
| `doctor`: thirteen checks | eighteen: `health-failed` (error), `health-degraded` (warning), `health-stale` (warning), `health-inconsistent` (error), `hostid-duplicate` (warning) | Appended to the check-id vocabulary; `--check`/`--skip`/`--transitions` and a watchdog's `doctor <CHECK-ID>` take them. `hostid-duplicate` names no cause: a collision, a cloned machine id and a second process look alike. A deployment with no `health.v1` service reads the health checks clean ("asked of none"); a scope with no minted address, `hostid-duplicate` clean |
| — | `doctor --clocks-synced` | The doctor listens to no status, so a `health.v1` status reply's age needs the operator's word that the clocks agree within the HLC delta. Without it, the `health-*` checks of every service implementing `health.v1` are unobservable (exit 2), each reason naming the flag. A watchdog's `doctor health-*` rule runs without it (`watchdog --clocks-synced` since #735) |
| `doctor`'s scope: presence and routers | and `health` (the GET selector, the services listing `health.v1`, whether `--clocks-synced` was given) | Absent when no `health-*` check was asked |
| `check conform`: ten cases | twelve: `health` and `health-aggregation`, subject `service` | Asked of a service whose descriptor lists `health.v1`, over the `--for` window: unhealthy or stale, and a break seen in both readings, are violations. Not asked otherwise |

## Unreleased (`main`, zk2) — `freshness.v1` (#720, PC)

`check conform`'s `freshness` case is asked, against the horizon a
resource declares (`spec/profiles/freshness/v1.md`).

| Before | Now | Notes |
|---|---|---|
| `check conform`: one `freshness` row, subject `service`, not asked | one `freshness` row per exposed stream, state and event resource | A resource that declares `freshness.ttl_s` is judged at the end of the `--for` window, each member by this run's receive clock and the GET reply's stamp: stale is a violation (exit 1), and a reply's age is unobservable unless the clock is trusted to the HLC delta. One with no horizon, and an event, is not asked. Scripts that keyed on `subject == "service"` key on the resource |
| — | `check conform --clocks-synced` | The operator's word that this host's clock and the owners' agree within the HLC delta (500 ms): a state reply's stamp is then aged against it. Without it, a stamp is aged only against a clock this run measured on a live put of the same clock (freshness.v1 §2.6) |
| `check conform`'s note: freshness and budget not asked | budget alone not asked | Its profile does not exist yet (#613) |

## Unreleased (`main`, zk2) — `hostid.v1` (#719, PB)

A noun read off the host, not the bus: the system a service gets when it
asks for `@hostid.v1/<service>` (`spec/profiles/hostid/v1.md`).

| Before | Now | Notes |
|---|---|---|
| — | `hostid [--machine-id HEX] [--v1-salt SALT]... [--format F]` | The inputs read in order (`/etc/machine-id`, `/var/lib/dbus/machine-id`, the shared file `/var/lib/zk2/hostid`) with the runtime's own ladder, read-only: the shared file is never created, and no machine id is printed. Exit 0 with the system; 2 when the host fails closed, or no input holds an id and the shared file is still to be made, every path named with its outcome; 2 for a `--machine-id` that §2.1 refuses. Each `--v1-salt` adds a `v1_origin` row for the migration table (Appendix B), none for an id from the shared file, which no v1 application read. Rows tagged `input` and `v1_origin` |

## Unreleased (`main`, zk2) — spec 0.17 (#713)

Two verdicts that read clean or failed on a premise the tool could not see
now need that premise.

| Before | Now | Notes |
|---|---|---|
| `check conform`: a present service's silent call is a violation | the same, only with `--calls-granted`; without it, unobservable (exit 2) | Spec §5.1 (0.17): no tool can observe its grants, so the operator says they let it call, or that the deployment runs no access control. A CI run against a deployment without access control adds the flag |
| `check conform`: `state-stamp` passes on the owner's own stamp | it passes only against a router this run verified; `--trust-admin-space` as `doctor`'s | Spec §4.2 (0.17): an owner that is its own router stamps with its own `meta.zid`. The routers' admin space must be on (zenoh's default is off) for `state-stamp`, and so the run, to pass |
| `doctor --deep`: `state-stamp-foreign` clean on the owner's own stamp | clean only against a router this run verified; a foreign stamp is still the finding | The same rule. The check now reads the admin space |

## Unreleased (`main`, zk2) — zk2's coverage completed (#585, FK1)

Four follow-ups of #612 bring back, in zk2's terms, the questions FJ9 left
unasked on `main`.

| Before | Now | Notes |
|---|---|---|
| `check conform --producer P [--origin] [--for] [--deep] [--junit]` (v1, left at FJ9) | `check conform <SYSTEM/SERVICE> <IFACE[@FP]> [--for SECS] [--i-know] [--junit FILE] [--seed N]` | #703: one service against the revision its descriptor claims, one verdict per case and resource — `contract-served`, `resource-served`, `payload-type`, `qos`, `operation` (O3, O6, the response type), `fanout-refused` (O2), `state-stamp` (S1, by `meta.zid` compared by value), `state-get` (S2); `freshness` and `budget` not asked until their profiles exist (#613). Only idempotent operations are called unless `--i-know`. Exit 0 every case asked passed, 1 a violation, 2 a case unobservable. `--junit`: a failure per violation, an error per unobservable case, skipped per case not asked |
| — (v1's `why` left at FJ9) | `why <KEY\|SYSTEM/SERVICE> [--for SECS] [--contracts PATH] [--namespace NS]` | #702: zk2's ladder, stopped at the first rung that establishes a cause — `namespace`, `key`, `presence` ("no token visible to this reader", 0.8), `descriptor` (implemented, exposed, or `unavailable` with its cause), `contract` (retrievable, verified, declaring the resource), `answer` (S4's GET, a stream's sample within `--for`, a union storage's occurrence; an operation is never called) and `last-known` (S6, after the owner's silence only). Exit 1 a cause, 0 every rung healthy and the key answering, 2 a rung unobservable. A key outside the namespace, or not zk2, opens no session. Rows tagged `rung`, each with its `verdict` |
| `storage gen --deployment FILE` (required) | `storage gen [--enrollment FILE --contracts PATH] [--deployment FILE]`, one at least | #704: from the enrollment `acl gen` reads, a union storage per event resource of every interface an enrolled service implements (spec §2.6), keyed `zk2/*/*/<iface>/events/<template>/<ulid>`, its `garbage_collection.lifespan` the contract's `retention`. The file's new `[events]` block names their volume (an implicit memory volume otherwise, warned). Nothing on any owner's `state/**` or `@state/**`: a file's selector that intersects one is refused citing S4, **exit 2** in every mode. Enrolled archives are listed, never planned (§4.4). New plan fields `enrollment` and a storage's `derived`; warnings `retention_not_enforced`, `implicit_volume` |
| `storage gen --check` (the admin space) | `storage gen --check [--against <router.json5>]` | #704: `--against` compares a router config file read through zenoh's loader, as `acl gen` does, and opens no session; without it, the admin space as before. The check report gains `source` (`admin_space` \| `file`) and the finding kind `on_owner_state` (S4), planned or not |
| `admin graph` (session flags) | `admin graph [--namespace NS] [--trust-admin-space]` | #705: each zk2 instance of the deployment is joined onto the routers by its descriptor's `meta.zid`, compared by value, through verified routers' session lists only (spec §4.2, 0.12–0.13). New `instance` rows, tagged `attachment`: `attached` (with its routers), `unattached`, `unattributable`; `--dot` draws them. The envelope's `instances` says what the join read |

## Unreleased (`main`, zk2) — what was left of v1 (#612, FJ9)

The last of v1 leaves `main`'s zenctl, and the binary no longer links v1's
`zenkey` (`cargo tree -p zenctl -i zenkey@0.11.1` matches nothing). Three
kinds of thing went. **The v1 registry**: the `--registry` union and RFC 08
§6 introspection, the slice cache, the v1 roster, the served-schema decode
ladder, and the verbs whose question was the registry (`why`, `check
conform`, `check cutover`, `check retired`). **The dark profile features**:
`config`, `blob` and `export`, with the kind and budget checks, which
return when their profiles exist (#613). **v1's leftovers in verbs that
stay**: `get`, `pub` and `replay` read and write a foreign key as bytes, and
`storage`, `admin graph` and `cache` lose their v1 joins. The `v1` branch
keeps all of it (0.14.x). No aliases and no shims: each row names where its
question went, or that nothing on `main` asks it yet.

| v1 | zk2 | Notes |
|---|---|---|
| `why <selector> \| --origin/--class/--producer [--for]` | — | not yet asked on `main`. The rung a key stops at is `echo --format ndjson`'s `identity`; a deployment's breakage is `doctor`'s |
| `why`'s 1 for a found cause | — | the polarity (#307) holds for the zk2 `why` when it lands |
| `check cutover --old-root <KEYEXPR> [--for]` | `compat <old> <new>`; `zk2 contract compat\|check-history` in CI | a zk2 migration is a contract revision, judged before it ships; there is no old keyspace to watch go quiet |
| `check retired [--for]` over `deprecated.lock` | `zk2 contract check-history` | a `.history` is append-only, and the classifier says what a revision breaks |
| `check conform --producer P [--origin] [--for] [--deep] [--junit]` | not yet; `check schema`, `check probe`, `check expect --valid-payload`, `doctor` meanwhile | a contract as a conformance suite is a follow-up; `--deep`'s freshness and `[budget]` judgements need the freshness profile (#613) |
| `config get\|set\|confirm\|cancel\|extend\|persist <origin> <producer> <resource> …` | — | dark until the configuration profile exists (#613) |
| `blob list [--producer] [--tier]`, `blob locate <id>`, `blob fetch <target> --origin [-o] [--root] [--allow-unpinned] [--overwrite] [-q]` | — | dark until the blob profile exists (#613) |
| `export [selector] [--bind] [--validate] [--doctor-every] [--max-series] [--once] [--for] [--prom] [--i-know]` | — | dark until its profile exists (#613) |
| `--registry <DIR>` (every verb that took it) | `--contracts <PATH>` where a verb reads contracts | `--contracts` takes an authoring file, a directory of them, or a `.history` root |
| `--registry` answering alone when the bus was down | — | a question `--contracts` answers alone opens no session at all (`schema show`, `iface show`, `compat`, `check schema`) |
| `context create --registry <DIR>` | — | a stored context's `registry` is ignored by zenctl; zengui still reads it from the shared file (#614) |
| `--origin`/`--class`/`--producer` (the composed selector) | a wire selector, or an address `<system>/<service>` | the last verbs that took them were `why` and `export` |
| `get <selector>` rows: `origin`, `subject`, the registry's `type` | `identity`, and the declared `type` when a contract resolves one | each reply is resolved through the deployment in `--namespace`, as `echo` resolves a sample: a zk2 key decoded as its declared type, a foreign one rendered structurally. A reply error no longer carries an origin |
| `get --body` through `pub`'s encode ladder | `get --body`, sent as typed | calling a zk2 operation through its contract is `call` |
| `get --raw` (ship the body verbatim, print hex) | `get --raw` (print hex) | the body is always verbatim now |
| `get --no-decode` (skip schema decode) | the same: no presence read, no bundle retrieved | |
| `get`/`echo --fmt` `%o %c %p %s` | `%A %i %r %K` (FJ8b) | the v1 placeholders print literally now; `%t` is empty when nothing is declared |
| `pub <key> <body> --qos sampled\|refreshed\|transition\|alert\|frame` | `pub <key> <body> --qos <priority/congestion/reliability[+express]>` | the token `echo` prints as `qos_axes`. Default zenoh's own, `data/drop/reliable` (was the subject's declared profile, else `sampled`); the choice is printed either way |
| `pub --encoding` defaulting to the registry's | `pub --encoding`, none by default | |
| `pub --raw` | — | every payload is sent as typed: there is no encoder to bypass |
| `pub --no-validate` | — | no served schema to validate against; a zk2 key is refused (exit **2**, P3) and written through its contract by `call` or a mock owner |
| `pub --from ndjson` row `qos` (a profile name) | `qos_axes` | the row's axes win over `--qos`; a v1 `qos` name is ignored |
| `pub --from ndjson` delete rows on state keys, unpriced | every delete row needs `--i-know` | a tombstone on a key no contract describes is the operator's act; a row on a key a zk2 service owns is refused either way |
| `replay --qos <profile>` (default `refreshed`) for rows that recorded none | `replay --qos <AXES>` (default `data/drop/reliable`) | a version-3 row publishes with its recorded `qos_axes`; a version-1 or 2 row's `qos` profile name is ignored |
| `replay` recorded deletes on state keys, unpriced | every recorded delete needs `--i-know` | as `pub --from` |
| `replay --registry` | — | `--base` (env `ZENCTL_BASE`, then the context) is still the deployment the capture's base must match unless `--force-base`; `--namespace` and the session flags are unchanged |
| `.zrec`/ndjson rows `origin`, `subject`, `qos` | — | `identity` and `qos_axes` carry what is left; a reader of older rows ignores the three |
| `storage list` `coverage` rows (declared state families against storages) and its `--registry` | `storage list` | the storages the routers' admin space reports; whether a storage answers on an owner's state keys is `doctor`'s `storage-on-state` (S4) |
| `storage gen` `[storages.X] class = "state"\|"telemetry"\|"events"\|"catalog"\|"catalog-pdns"` | `[storages.X] selector = "<relative key expression>"`, required | a file that still names `class` is refused (exit **2**), serde naming the field. The plan's `class`, `covers` and `registry` fields are gone |
| `storage gen` `gc_margin` (lifespan = longest `ttl_s` × margin) | `gc_lifespan_s`, default zenoh's 86400 s | no contract declares a tombstone lifetime to derive one from; the derivation line says which applied. Warnings `volatile_seed` and `lifespan_below_ttl` are gone |
| `storage gen --base`, `--registry` | `storage gen --namespace NS` | the file's `base` still wins |
| `admin graph --origins` | — | v1 origins over the router graph; joining zk2 instances to routers is a follow-up |
| `admin routers\|graph --base\|--registry` | the session flags alone | the admin space is un-namespaced |
| `cache show\|refresh\|clear` over the slice cache | `cache show\|clear` (session flags), `cache refresh --namespace NS` | the cache is zk2's name cache: per namespace, the service addresses and interfaces the last presence read saw. `refresh` re-reads presence |
| completion of producers, classes, QoS profiles, blob tiers | completion of service addresses, interfaces and namespaces | from the name cache (FJ4) |
| `watchdog --rule 'alert-firing …'` | — | still refused (exit **2**) until the alert profile exists (#613) |

Gates: `scripts/check-degradation.sh` (#210, one door out of a missing
registry) left with the registry. zenkey-fleet's feature axes (`decode`,
`decode-protobuf`, `decode-cdr`, `validate-json`) forwarded to v1's codecs and
are gone; `just features` and CI's `features` job check zenkey's
`--no-default-features` build instead. Report families removed: `why`,
`cutover`, `registry-retired`, `conform`, `config` and v1's `call` beside it,
`blob-list`, `blob-probe`, `blob-tree`, `blob-fetch` and `export`; the
`storage-list` family loses its `coverage` rows.

## Observers and checks (#612, FJ8b)

The verbs that watch traffic and judge it read zk2 now. The raw observers
(`echo`, `rate`, `field`, `timeline`, `snapshot`, `watchdog`, `record --on`)
keep a wire selector on a session in no namespace, so they still see every
key — and resolve each one through one session-free **lens**: the
deployment in `--namespace`, its presence read, the contract each
descriptor names (from `--contracts` or its holders, spec §8.4). Where the
ladder stops, the rung is named (the tooling guide's O2): not in this
namespace, not a zk2 key, no provider, no revision, contract unavailable,
no resource. A `.zrec` is read through the same lens, `--contracts` alone,
so a window rendered live and from its file is the same projection. The
checks (`check expect|schema|probe`) are written over an address, an
interface revision and a resource. v1's composed selectors
(`--origin`/`--class`/`--producer`), the registry ladder, the identity
bridge and the origin alignment are gone from these verbs; the `v1` branch
keeps them.

| v1 | zk2 | Notes |
|---|---|---|
| `echo [selector] --origin/--class/--producer` (default `<base>/v1/**`) | `echo [selector] [--namespace NS] [--contracts …]` (default `<ns>/zk2/**`) | a zk2 key is decoded as its declared type (§7.2) and a JSON Schema value validated (§7.3); bytes that do not decode are named as their type and `undecodable`; a foreign key renders structurally, never refused |
| `echo --seed` | — | an owner's current state is `get state` (S4); a router storage answering on an owner's keys is what S4 forbids |
| `echo --no-decode` | the same | no presence read, no bundle retrieved, every payload structural and said to be |
| `echo --fmt` `%o %c %p %s` | `%A` address, `%i` interface, `%r` resource, `%K` the key relative to the namespace | `%t` is the declared type |
| `echo --format ndjson` rows | the same rows, plus `identity` (`is`: `resource`, `control`, `not_in_namespace`, `not_zk2`; the address, interface, kind token and resource; the rung it stopped at) and `verdict` (`valid`, `invalid`, `undecodable`, `not-checked: <why>`), `violations`, `decode_error` | `pub --from ndjson` and `.zrec` still read them back; a reader ignores `identity` |
| `rate --origin/--class/--producer` | `rate [selector] [--namespace NS]` | `group` rows lead, one per zk2 address and resource (`is`, `address`, `iface`, `token`, `resource`, `keys`, `count`, `bytes`), a key that is not this deployment's zk2 data in a group of its own; `--per-key` adds `key` rows, each with its `identity`; `lens` on the envelope |
| `rate --latency` | the same | each stamper named in `clocks`: the owner's (its descriptor's `meta.zid`), another's, or unattributable (O7) |
| `field` over a served `describe` | `field` over payloads decoded through their contract | each `path` row says whether the declared type accounts for it (`declared`); `field-new` is judged from the bundle's own type (a JSON Schema's properties, a protobuf message's fields), `field-vanished` as before; samples no contract resolved are counted (`unresolved`), observed structurally, and judged for nothing that needs a type (O4) |
| `field-stuck` | not asked | it judges against a declared freshness, the `freshness.v1` profile's (#613); every rendering says so |
| `timeline` lanes per origin/producer, `provenance` `self_stamped`/`foreign`/`unattributable` | lanes per zk2 address and resource (`lane.kind`: `resource`, `control`, `foreign`, `unstamped`), `provenance` `owner`/`other`/`unattributable` | a stamp is the owner's when its id is the session zid the owner's descriptor states as `meta.zid` (§3.3), compared by value; a router-stamped stream sample is another clock's; `lens` on the envelope |
| `timeline --from <zrec>` | the same, with `--contracts` | the same lanes; every stamp unattributable, since a file names no owner |
| `snapshot --origin/--class/--producer` (default `<base>/v1/**`) | `snapshot [selector…] [--namespace NS]` (default `<ns>/zk2/**`) | each selector narrowed to its state keys and asked as S4 asks (target All, consolidation Latest), a reply on a wildcard key discarded (R6) |
| `snapshot --no-roster` | `snapshot --no-presence` | every holder `unattributed`, every stamp unattributable, a key resolved past its address only through `--contracts` |
| `.zsnap` version 1 | version 2 | the header's `roster` is `presence` (`selector`, `complete`, `services`), plus `discarded`; a row's `registration` is `identity`, its `verdict` is `conformance` (`state`: `valid`, `invalid` with `violations`, `undecodable`, `not_checked` with `reason`), `stamper` is `owner` or `other` with its id; `holder` is `live` (`address`, `answered_by`: `owner` or `other`), `no_instance` (a value answered for an address presence shows no instance of, which S4 forbids), or `unattributed`. A version-1 file is refused (exit **2**) with a hint: the `v1` branch reads it |
| `snapshot` report `storage_only` | `no_instance`, plus `nonconforming` | |
| `snapshot diff [--normalize-origins] [--map A=B]` | `snapshot diff <a> <b>` | rows line up by **zk2 key**, each file's namespace stripped, so two deployments compare as they are — the key already names the system and the service, and there is no origin to align. Facets `value`, `conformance`, `holder`; between two namespaces a stamp that moved alone is not a change. The `origin_map`, `unmapped` and `subject` rows are gone |
| `check expect <selector> [--origin/--class/--producer]` | `check expect <system>/<service> <iface>[@fp] [resource] [--param k=v]` | the runtime's consumer, subscribed before the window opens (O4); R6's discards and unresolved samples counted apart from the lag. The report adds `address`, `iface`, `fingerprint`, `resource`, `selectors`, `discarded`, `unresolved`, `presence` |
| `check expect --valid-payload` against a served schema | the same, against the declared type | decodes as the type, and a JSON Schema value satisfies it; "not checked" fails the assertion, with why |
| `check expect --qos declared\|PROFILE` | `--qos declared` | priority, congestion control and express against the resource's (§2.4); no v1 profile names |
| — | `check expect --present` | the address holds the interface's token within the window (§8.1); no resource: presence alone. A complete read with none is not met; one that ran to its timeout cannot carry it (exit **2**) |
| `check schema --type T --producer P \| --schema-set FILE` | `check schema <iface>[@fp] <resource> [--member type\|attachment\|request\|response\|error\|summary] --from … [--encoding]` | the revision's own type, from `--contracts` (no session) or its holders. Exit 0 conforms, 1 invalid or undecodable, 2 not checked (no revision, no such resource or member, a raw type, an unreadable payload). The `schema-check` report is `iface`, `fingerprint`, `resource`, `member`, `declared`, `encoding`, `size`, `conformance`, `value` (was `type`, `kind`, `verdict`, `detail`) |
| `check probe <origin\|hostname> <producer> <procedure>` | `check probe <system>/<service> <iface>[@fp] <resource> [--param k=v] [--for SECS]` | a resource read the way a consumer reads it: a state resource's current state from its owner first (S4), then the subscription. Exit 0 a conforming value arrived; 1 nothing usable did, while a complete presence read shows the provider up, or no token visible to this reader; 2 the presence read timed out, or the probe could not stand up. The `probe` report is new (`received`, `conforming`, `nonconforming`, `current`, `first`, `presence`, `verdict`); the RFC 06 §6 bridge is gone |
| `watchdog --rule 'invalid-payload <SEL>'` against a served schema | the same, against the declared type | a payload that does not decode, or fails its JSON Schema; a tick with nothing checked is `unobservable`, never `ok` |
| `watchdog --rule 'qos-mismatch <SEL>'` against a registry profile | the same, against the resource's declared QoS | priority, congestion control, express (§2.4); reliability is the link's and not compared |
| `watchdog --rule 'origin-down <origin>'` | `instance-gone <system>/<service>` | no instance token visible to this reader (§8.1); the old spelling is refused and names the new one |
| `watchdog --rule 'alert-firing …'` | — | dark until the alert profile exists (#613); refused (exit **2**) |
| `watchdog` summary `facts_evicted` | — | the v1 facts cache is gone |
| `record --on 'invalid-payload\|qos-mismatch …'` (refused in FJ8a) | armed again, judging zk2 | and `instance-gone`; `alert-firing` stays refused |
| `watch` samples | each sample's `conformance`, and its `qos_mismatch` (`declared`, `observed`, `differs`) when it rode another QoS; the summary's `unresolved` (#671), `qos_mismatched`, `nonconforming` | the runtime's `Subscription::unresolved`: a sample on a key that resolves to no member, never delivered |
| `call`, `bench call` with a request its JSON Schema refuses | refused, exit **2**, every violation named, nothing sent | §5.1 "The request", #671. `bench call` refuses the act first (not idempotent, `--calls 0`) |
| `call` fan-out `presence.unheard` | the same, from the runtime's `Fleet::presence` (#671) | its `complete` says whether the read could carry "nobody" |
| `get state --last-known` with a parameter left out: refused | read: every member the archive holds (`archive::last_known_all`, #671) | one row per origin |

The observed QoS is judged against the declared one in three places:
`watch` (per sample), `check expect --qos declared`, and the watchdog's
`qos-mismatch`. Report families re-shaped: `schema-check`, `probe`, `rate`
(`group` rows), `field`, `timeline`, `snapshot`, `snapshot-diff`, `expect`,
`watch`. Still v1 on `main` until FJ9 retires the registry: raw `get`'s
decode ladder, `why`, `export`, `check cutover`, `check conform`, `check
retired`, `config`, `blob`.

## Writes and captures (#612, FJ8a)

The verbs that put traffic on a bus or keep it are zk2's now. `gen` and
`serve` are **mock owners**: real zk2 services at an address the operator
names, brought up through the runtime's `ServiceBuilder` (descriptor, tokens,
writers, spec §8.2's order), so a mock writes only what it owns (P3). `bench
call` times an operation per reply; `.zrec` is version 3; `pub --from ndjson`
and `record --on` keep their shape with the zk2 rules applied. v1's registry
generator, the schema ladder, the fault injector, `serve`'s any-key mock and
`bench rpc` are gone from `main`; the `v1` branch keeps them.

| v1 | zk2 | Notes |
|---|---|---|
| `gen --producer <p> [--subject s] [--var k=v] [--origin host]` | `gen <system>/<service> [iface[@fp]…] [--member resource=v/v,v/v] [--bind role=sys/svc]` | every stream, state and event resource of the named interfaces (all `--contracts` loads, if none) is published through the runtime's writers, with its contract's QoS and encoding, state stamped by the owner (S1), events on fresh ULID keys within their declared rate. A templated resource publishes the `--member` values, or two synthetic ones (`<param>-1`, `<param>-2`) the plan states; the template declaring `epoch` holds a member token per value (§8.1). Operations are answered with a synthesized reply |
| `gen` against a host already publishing (`--origin`, `--i-know`) | an address whose instance token is present: exit **2** unless `--i-know` | a mock beside a real owner is a second writer of its keys and a split-brain on every exclusive resource (§6) |
| payloads from the registry's schema (`--schema-set`, a served `describe`) | synthesized from the bundle, deterministic per (`--seed`, tick) | JSON Schema in spec §7.3's subset, every value checked by `zenkey_model::validate`; protobuf with every field filled (the first of each `oneof`); raw bytes of the media type's size class (video 16 KiB, image 4 KiB, audio 2 KiB, text 64 B, else 256 B) |
| the `synthetic` sample attachment | `meta.synthetic` in the descriptor (`{"synthetic": true, "tool": "zenctl gen", "seed": 42}`) | informative (§3.3); a sample's attachment is the contract's to type (§2.3), so it carries no marker |
| `gen --fault <kind>`, `--wide`, `--serve-describe`, `--schema-set` | — | the fault injector and the v1 ladder went with the registry |
| `gen --rate`, `--pattern`, `--duration`, `--seed`, `--dry-run` | the same | `--rate` overrides streams and state (1 Hz and a re-put every 2 s by default); an event keeps within its declared rate whatever it says. `--dry-run` prints the plan (`gen-plan`) and, with `--contracts` holding every interface, opens no session |
| `serve <keyexpr> <reply> [--encoding] [--raw] [--no-validate] [--complete] [--i-know]` | `serve <system>/<service> <iface>[@fp] <operation> [JSON\|@FILE\|-]` or `--refuse <code> [--message] [--cause]` | one operation through `ServiceBuilder::serve`: the reply encoded as the response type from JSON (as `call` encodes a request), or the chosen §5.2 envelope. Every call is logged (`call` rows: the key, the request decoded from the bundle, the call metadata, O7). The interface's other optional resources answer `unavailable` (config), its other required operations `internal` |
| `serve --count`, `--for` | the same | a stop bound and a window |
| `bench rpc <origin> <producer> [procedure] [--calls] [--concurrency] [--i-know]` | `bench call <system>/<service> <iface>[@fp] <operation> [JSON\|@FILE\|-] [--param] [--calls] [--concurrency] [--i-know]` | latency per reply on this tool's clock, attributed by the replier's key (O3); envelopes a population of their own by code; malformed, transport, silent and discarded counted and never averaged in (O5); each token holder tallied against the calls it sent no value in. A fan-out only to `fanout = "allowed"` (O2, not overridable). Exit 0 values only, 1 an envelope or malformed reply, 2 no value at all |
| `record <selector>` (default `<base>/v1/**`) | `record [selector…] [--namespace NS]` (default `<ns>/zk2/**`) | several selectors; a hint when none lies in the stated namespace |
| `.zrec` version 2 | version 3 | the header adds `excluded`, the verbatim chunks (`@stream`, `@state`, `@op`, `@zk`, `@adv`) no selector names, so the file cannot hold them (O5); each row adds `qos_axes`, which `replay` publishes with. Versions 1 and 2 still read |
| `record --on <rule>` | the same, judging zk2 | the state preamble is each owner's S4 GET (All + Latest, R6's discards dropped); `invalid-payload`, `qos-mismatch`, `origin-down` and `alert-firing` judge v1 and are refused (exit **2**); `doctor` is FJ6's |
| `pub --from ndjson` rows with `"qos"` (a profile name) | `"qos_axes"` (`priority/congestion/reliability[+express]`) wins over `"qos"` | `echo --format ndjson \| pub --from ndjson` publishes a foreign row with the QoS it was seen with. A row whose key a zk2 service owns is refused and counted (P3), and a pipe of only such rows is refused without opening a session |

Report families: `gen-plan` and `gen` (the plan and the run's totals), `serve`
(`call` rows and a `summary`), and `bench` re-cut (`repliers`,
`refusals`, `presence`, `clock: "round_trip"`).

## `acl gen` compiles zk2's grants (#612, FJ7)

`acl gen` is zk2's now: spec §11's three grant shapes (Own, Consume, Call),
compiled from the contracts and an enrollment into a router's
`access_control` block, under either posture, with a constrained face when
asked. v1's planner (RFC 09 §3's role matrix over registry slices, and the
RFC 09 §4 face over `exposure` markers) is gone from `main`; the `v1` branch
keeps it.

| v1 | zk2 | Notes |
|---|---|---|
| an enrollment of `[[principal]]` `cn`/`user` + `role` + `origin`/`machine_id`, `[fleet]` | `[[principal]]` `user`/`cn` + `services`/`archives`/`tools`; `[[service]]` (`address`, `implements`, `bindings`, `calls`), `[[archive]]` (`records`, `peers`), `[[tool]]` | a v1 enrollment does not load: exit 2 naming the unknown field. `examples/zk2/acl/` holds two |
| `--registry <dir>` narrowing the planes and the write set | `--contracts <path>` (authoring files, a directory, a `.history` root), required for every binding and call | the plan names the revisions it was compiled from; regenerate on every revision (§11.2) |
| `default_permission: deny` always (a face: allow) | `--default-permission deny\|allow` (alias `--posture`), default deny | under allow, every grant compiles into denies of its complement, and every principal is denied queryables in the admin space (#684) |
| `--allow-zid-subjects` | — | a `zid` principal is refused, with §11.3's reason: exit 1 |
| `--face constrained --link-protocol … --link-interface … --link-interval …` | `--face constrained --attach client\|south-region --far <principal> [--region <name>]` | the far principal's policy carries the `@zk` and `@stream` denies; a south region adds `gateway.south` and the declarations a far router needs; `--attach router` is refused (exit 2): its key strings cross |
| rule rows with `purpose` | rule rows with `grant` (`own`, `fan_in`, `fan_in_reply`, `consume`, `history`, `presence`, `call`, `contracts`, `deny_write`, `deny_read`, `deny_receive`, `deny_admin_space`, `face_declarations`, `face_presence`, `face_stream`) and `holder` | every rule names the grant it instantiates and the fact it exists for (`cite`) |
| subject rows with `role` | subject rows with `runs` | |
| the plan's `base`, `registry`, `downsampling` | `namespace`, `contracts`, `face`, `gateway` | |
| check findings `unknown_cn`; `interest_probe` | `unknown_identity` (a CN or a user), `gateway_differs`; no `interest_probe` | |
| explain `via[].purpose` | `via[].grant` | |

## `doctor` judges a zk2 deployment (#612, FJ6)

`doctor` is zk2's now: thirteen checks of a deployment against the core, each
a question whose finding is the *yes*, each answered in the judgement shape —
a finding, clean with its evidence, unobservable with what stood in the way,
or not asked. The deployment is read through a session in its namespace
(`--namespace`, alias `--base`); the routers' admin space and the presence
domain through one in no namespace. v1's checks are not the doctor's any more:
the slice diff, describe coverage, schema drift by registry, the listen phase,
fields, kind and budget live on only as `check conform`'s projection, and leave
with it.

| v1 | zk2 | Notes |
|---|---|---|
| `doctor --registry <dir>` | `check conform <producer> --registry <dir>` | the served-vs-declared diff is conform's; the doctor reads no registry |
| `doctor --for <SECS>` | — | the listen phase (payloads, QoS, unregistered traffic, rate, kind, fields) is conform's `--for`; a zk2 subscription is `watch` |
| `doctor --deep [--sample N]` | `doctor --deep` | now asks `state-stamp-foreign`: each owner's state GET, every reply's stamp against the owner's session (S1–S2) |
| — | `doctor --grace SECS` | the two presence reads split-brain and token-missing compare (default 2 s) |
| — | `doctor --presence-budget N` | what `presence-over-budget` judges the domain's token count against (default 10000) |
| — | `doctor --check ID … \| --skip ID …` | ask only some checks; the rest read `not_asked`, which neither passes nor fails |
| `doctor --fail-on error\|warning`, default: always 0 | `doctor --fail-on error\|warning`, default **warning** | exit **1** on a finding at or above the floor; an info finding never fails |
| a run that judged nothing: exit 2 | an empty scope, or a check left unobservable, with no finding at or above the floor: exit **2** | a wrong namespace or endpoint is never green; a pre-run failure is the reserved 2 too |
| findings as rows (`finding`), coverage counts on the envelope | one `check` row per check: `check`, `section`, `verdict` (`answer`: `established`, `not_established`, `unobservable`, `not_asked`), `findings`, `unjudged`; `scope` and `unobservable` on the envelope | a finding is `severity`, `check`, `subject`, `evidence` |
| `doctor --transitions` | `doctor --transitions` | one baseline line per check asked; firing on a finding, ok when clean, unobservable otherwise |
| `watchdog --rule 'doctor <v1 id>'`, `record --on 'doctor …'`, `export --doctor-every` | the same, over zk2's ids | the rule's doctor runs in `--base`, through a second session; a run asks only the checks its rules name |

The checks, and what each enforces:

| id | the question (the finding is the yes) | spec |
|---|---|---|
| `split-brain` | do two instances of one service hold one interface's token past the grace period, with two exposing an exclusive resource? | §6 |
| `binding-unsatisfied` | does a role's binding select no provider visible to this reader? | §3.2 R5 |
| `contract-drift` | do providers of one interface serve revisions the classifier calls review or breaking against each other? | §9.8 |
| `contract-unavailable` | does a descriptor name a revision no holder serves verified? | §8.4 |
| `descriptor-invalid` | does a descriptor fail the descriptor check (a D-code)? | §3.3 |
| `token-missing` | do an instance's tokens disagree with its descriptor? | §8.1 |
| `presence-over-budget` | does the presence domain hold more tokens than its budget? | §8.3 |
| `storage-on-state` | does a router storage answer on an owner's state keys? | §4.2 S4 |
| `archive-unaligned` | does an archive serve keys its alignment has not confirmed? | §4.4 |
| `state-stamp-foreign` | does an owner answer its state with a stamp that is not its own? (`--deep`) | §4.2 S1–S2 |
| `shm-memlock-low` | is this host's `RLIMIT_MEMLOCK` below what a shared-memory pool needs? | §7.4 |
| `admin-unreachable` | does no router answer the admin space? | §4.2 |
| `router-version-skew` | do the routers run different zenoh versions? | App. B |

v1's check ids are not the doctor's any more. `check conform` and `field`
still report the ones they project (`slice-parse`, `slice-sync`,
`describe-totality`, `schema-drift`, `stale-state`, `payload-undecodable`,
`payload-invalid`, `qos-observed-mismatch`, `unregistered-traffic`,
`rate-over-declared`, `cardinality-over-declared`, `field-*`, `kind-mismatch`,
`budget-exceeded`); the ones only the doctor filed (`introspect-coverage`,
`describe-missing`, `unstamped-state`, `storage-coverage`,
`timestamp-stamped-elsewhere`) are gone from `main`. `admin-unreachable` and
`router-version-skew` keep their spelling with zk2's meaning. The `v1` branch keeps the v1 doctor whole.

## FJ5 — acting and reading through a contract (#612, FJ5)

zk2's acts and reads, each aimed at an address, an interface revision and one
resource of its contract, through the zk2 runtime's own client and consumer in
the deployment's namespace. `service call` and `retire` are gone; `get` keeps
its raw form and gains a resolved one beside it.

| v1 | zk2 | Notes |
|---|---|---|
| `service call <origin> <producer> <procedure> [--body] [--param k=v]` | `call <system>/<service> <iface>[@fp] <operation> [JSON\|@FILE\|-] [--param name=value]` | the request is encoded as the operation's type (JSON or CBOR, protobuf from JSON through the bundle's descriptor set, raw bytes); one address is called `BestMatching` + `None`, and only an idempotent operation is retried (`--retries`, after silence) |
| `service call '*' …` (+ `--i-know` for an unknown procedure) | `call '*/tc' …`, or a `--param` left out | a fan-out (`All` + `None`) only to an operation declaring `fanout = "allowed"`; any other is refused before anything is sent (exit **2**), and no flag moves it |
| `service call --trace [--for]` | — | dropped with `service call`: the trace window attributed by the v1 registry's declared chain |
| `service call --attachment`, `--raw`, `--no-validate` | — | the contract decides the encoding; the request is not checked against its JSON Schema (the owner refuses what does not decode) |
| a call's silence: `exit 2` and a paragraph | `answer: "silent"`, `silence.presence` | `present`, `instance_only`, `no_token_visible` (what *this reader* could see: a refused presence read is empty too, spec §8.1 0.8), `unknown` (a read that may be incomplete) |
| — | a fan-out's `replies` | repliers by key (rows), envelopes unattributed (`refusals`), `presence.unheard` (token holders that sent no value), `possibly_partial` per replier with a declared summary |
| `get <selector>` | `get <selector>`, unchanged | raw: a wire selector on a session in no namespace |
| — | `get state <system>/<service> <iface>[@fp] <state> [--param]` | resolved: the owner's current state (target All, consolidation Latest, S4); silence is exit **2**, never "no value" |
| — | `get state … --last-known <archive>` | an `archive.v1`'s answer (S5), one key at a time: `reading: "last_known"` in every format, with `confirmed` and the type identity |
| — | `watch <address> <iface>[@fp] <resource> [--for] [--count]` | a stream, state or event resource, every sample decoded through the contract; R6's discards and the lag counted apart; exit **2** when nothing arrived |
| `retire <key> [--qos] [--i-know]` | — | a tombstone has no zk2 meaning a tool may send (P3); `check retired` is a different verb and stays |
| `pub <key> <body>` | `pub <key> <body>` | a key a zk2 service owns (`…/zk2/<system>/<service>/…`) is refused, exit **2**, not overridable; foreign keys are written as before. `--from ndjson` refuses such a row and counts it |
| `replay <file>` | `replay <file> [--namespace NS]` | `--namespace` publishes through a session in `NS`, every key moved from the capture's base (spike S13); a zk2 service's own key replayed where it runs — as recorded, or into the capture's own namespace — is refused and counted unless `--i-know` (P3) |

Report families: `operation` (a call; v1's `call` family stays with
`config`'s read-back) and `state`; `watch` streams `sample`, `discarded`,
`lagged` and `summary` rows with no envelope, and a typed `--format json` is
refused, as on every stream. `--timeout` on the zk2 verbs also bounds a state
GET and a call; a call given none waits its operation's `timeout_ms`.

## FJ4 — the inspection nouns speak zk2 (#612)

`main` is the zk2 line; nothing is released from it until FJ9, and fixes to
the v1 tool ship from the `v1` branch (0.14.x). This chunk replaces v1's
registry nouns with zk2's: presence (instance and interface tokens),
descriptors and contract bundles, read through a session opened **in** the
deployment's namespace (decided 2026-10-08) — `--namespace`, with `--base`
as its alias and the context's `base` as its rung. Raw verbs and the admin
space stay un-namespaced. No aliases and no shims, as with #307: the v1
spellings below are gone, and each row names where its question went.

| v1 | zk2 | Notes |
|---|---|---|
| `topic list [--producer] [--class] [--type] [--deprecated] [--watch] [--budget]` | `iface list`, `iface show <iface>` | a zk2 interface's resources are its contract's (`iface show` lists them with kinds, types, QoS and fan-out); the cardinality judgement is `doctor`'s (FJ6) |
| `topic info <key>` | `iface show <iface>`, `schema show <iface> [resource]` | a key's meaning is its contract's resource; resolved decoding of one sample is FJ5's `get` |
| `node list [--verbose] [--watch]` | `service list [--system S]` | instances from their instance tokens, interfaces from tokens and descriptors side by side, the tokenless set (U22) from descriptors; a timed-out read says *possibly incomplete* |
| `node info <origin>` | `service show <system>/<service>` | the descriptor as served; exit **2** when presence shows no instance |
| `base list [--watch]` | `namespace list` | reads `**/zk2/*/*/@zk/instance/*` from a session in **no** namespace; the bus root is `(empty)`, selected with `--namespace ''` |
| `service list [--producer]` | `service list`, `iface show` | a procedure is an operation resource of an interface |
| `service info <producer> [procedure]` | `iface show <iface>`, `schema show <iface> <resource>` | an operation's request, response, error and summary types, `fanout`, `serving` and `idempotent` |
| `service call …` | `call` (FJ5, above) | |
| `interface list` | `iface list` | |
| `interface show <type> [--schema] [--full]` | `iface show <iface>[@fp]`, `schema show <iface> [resource] [--full]` | schemas come from the revision's bundle — `--contracts` or retrieved from its holders (spec §8.4) — never from a served `describe` |
| `schema show <producer> [--type T] [--full]` | `schema show <iface>[@fp] [resource] [--full]` | protobuf artifacts shown as their messages and enums; a resource named implies its documents; exit **2** when the revision cannot be had |
| `registry export\|diff\|lint\|lock\|migrate\|infer` | the contract CI binary (`zk2 contract lint\|fingerprint\|bundle\|compat\|check-history`) and `compat` | contracts are authoring files with a `.history`; the compatibility gate is `compat <old> <new>` here and `zk2 contract compat` in CI |
| `registry consumers\|impact <subject>` | `iface show <iface>` (its consumers), `graph` | consumers are the roles descriptors declare (R3), never admin-space subscriber declarations |
| — | `graph [--dot]` | the binding graph from descriptors and tokens (R3): an edge per binding that selects a provider present now, the edges the runtime computes; `--dot` is Graphviz, not a `--format` |
| — | `compat <old> <new>` | each side a `*.toml`, a `*.bundle.json` or `<iface>[@fp]`; exit **0** compatible, **1** review or breaking, **2** no verdict |

New bus flags on the zk2 verbs: `--namespace NS` (alias `--base`, env
`ZENCTL_BASE`), and `--contracts PATH` (repeatable: an authoring file, a
directory of them, or a `.history` root) on `iface show`, `schema show` and
`compat` — a revision held there is never retrieved, and a question it
answers alone opens no session. `namespace list` takes neither.

Exit contract: a zk2 `show` asked about something nothing on the bus
answers for — a service presence does not show, an interface nobody
provides, requires or describes, a revision no holder serves — is the
reserved **2** (`exit::Unanswered`, a sibling of `NoSession`). `check
retired` moved with its ledger out of the retired `registry` module; its
spelling and exits are unchanged.

Completion offers the service addresses, interfaces and namespaces the last
presence read and `namespace list` saw, per namespace, from
`zk2-names.json` in the context's cache directory (`cache clear` removes it).

## 0.12.0 (2026-10-06) — config speaks RFC v1.50, and principals on a face

### config speaks RFC v1.50

The configuration verbs catch up with RFC 05 §5.1 v1.50 (marcpardo/zenkey#518,
#560). One argument became optional; nothing that worked before stops.

| Invocation | Before | Now |
|---|---|---|
| `config set … --token T` | unknown flag (exit 2) | joins pending change `T`: the group rides its window and is confirmed, cancelled or rolled back with it; a reach group still asks for a yes, the prompt naming the window it joins |
| `config set … --token T --confirm S` | — | exit 2: a joining change takes the pending change's window |
| `config persist <o> <p> <r>` (no token) | exit 2: `<TOKEN>` required | persists the read-back's `last_change`, and says which on stderr; exit 2 when there is none (naming a pending change, if one is waiting to be confirmed) |
| `config confirm`/`cancel`/`extend`/`persist` output | the reply drawn as a generic call reply (JSON in the table) | the read-back after the act, drawn as `config get` draws it — `parameter` rows, `pending` and `last_change` rows in NDJSON |
| `config get`, `config set` | — | a `last_change` row (NDJSON) and `last change <token> on <groups>` on the table's head line, when the producer serves one |

### user principals, and principals on a face

For a per-operator write grant on a constrained face (marcpardo/zenkey#529,
RFC v1.49; zenoh-modem#153). No spelling moved.

| Invocation | Before | Now |
|---|---|---|
| `acl gen` with a `[[principal]]` carrying `user = "…"` | exit 2: `unknown field user` | planned: the subject carries `usernames: ["…"]` |
| `acl gen --face …` with `[[principal]]`s in the enrollment | every principal silently ignored | a `user` console or watch is planned onto the face (its own subject on the face's transport, repeating every face deny); every other principal is refused by name — **exit 1** |
| `acl gen` with a `writes` pattern narrower than its declared write (`modem/config/rf0/air/set` under `config/{device}/{group}/set`) | a dead `writes-…` allow, and a `grant_matches_nothing` note claiming it was undeclared | no allow; a `grant_cannot_carve` warning naming the declared write — grant the write as the registry spells it |
| `acl gen --explain <user> …` | `principal … is not enrolled` | resolves the principal by user |
| `acl gen --check` against a subject with `usernames` the plan carries | `subject_unplanned_property` | compared: `subject_differs` when the names differ, nothing when they agree |

The table renderer's `bound by` column names every property a subject is
bound by (`user ops; link_protocols unixsock-stream`). An enrollment that
used to be reused for `--face` only for its `base` — a fleet's, with
certificate principals in it — now exits 1 with a refusal per principal:
give the face an enrollment of its own (`base`, and its operators).

## 0.11.0 (2026-10-02) — zenctl in production

Built for a production bus (marcpardo/zenkey#498). **Scripts should read
the tables below before upgrading**: no spelling moved, but several
invocations that used to write, or exit 0, now refuse with exit 2 or exit
on a judgement — that is the point of the release. The session is a Zenoh
client by default; `--version` names the commit it was built from.

### The session stops being a peer (chunk DQ)

Three defaults changed, all in the session every verb opens (epic
marcpardo/zenkey#498, chunk DQ). No spelling moved; the exit contract gained
one row.

**The session is a zenoh client** (marcpardo/zenkey#501). It was a peer:
a listener on `tcp/[::]:0` on every interface, gossip on, and a mesh that
learned a laptop's locator and opened links to it. It now holds no listener
and gossips nothing. `--listen` (or a context that listens) still makes it a
peer, and a `--zenoh-config` that states `mode` keeps its own.

**`--zenoh-config` no longer turns multicast on** (marcpardo/zenkey#502).
zenoh fills every key a file leaves out with its own default, and its
default is multicast *on*, so a file holding only `connect.endpoints` used
to scout. Multicast and mode now follow the file only where it names them
(`scouting.multicast.enabled`, `mode`); `--scouting` still wins over both.

**A typo or a dead router is no longer an empty bus** (marcpardo/zenkey#503).
An endpoint that does not parse is refused by name — the missing `tcp/` in
its own sentence — where it was silently dropped (and, with a config file,
the file's endpoints were dialled instead). A router that does not answer
fails the session, because a client does not open onto nothing. Both exit
**2**, for every verb, `pub` and `retire` included: a session that never
opened asked nothing and attempted nothing. A verb holding `--registry`
dirs still answers from them and says so (#196); with no endpoint named at
all, that note now appears where the old peer session ran silently against
nothing.

| before | now |
|---|---|
| `zenctl echo 'prod/v1/**' -c tcp/r:7447` holds `LISTEN *:<port>` | holds no listener (a client); `--listen` for a peer |
| `--zenoh-config z.json5` (no `scouting` key) joins `224.0.0.224:7446` | multicast off unless the file states it, or `--scouting` |
| `--zenoh-config z.json5` (no `mode` key) opens a peer | opens a client; state `mode` in the file to keep a peer |
| `-c 127.0.0.1:7447` (no `tcp/`) → "no live producers", exit 0 | `Error: connect endpoint "127.0.0.1:7447": has no protocol …`, exit 2 |
| `-c tcp/127.0.0.1:1` (nothing listening) → empty answer, exit 0 | `Error: failed to open the Zenoh session`, exit 2 |
| `pub … -c tcp/127.0.0.1:1` → `published`, exit 0 | exit 2, nothing published |
| no endpoint, no context, no `--scouting` → empty answer, exit 0 | "nothing to connect to: …", exit 2 (or the `--registry` dirs alone, with the note) |

### The write guards (chunk DR)

Every one of these used to write with no consent and exit 0; each now ends
in a refusal — exit 2, before a session opens wherever the input alone
decides it — or asks the acknowledgement its verb already had. The
table below is the whole migration.

**`pub` refuses a wildcard key** (marcpardo/zenkey#504): `pub 'prod/v1/**'
x` printed `published` and reached every subscriber the expression
intersects. It is the blast radius `retire` already refused, and now one
refusal serves both (`zenkey_fleet::check_concrete`), not overridable. A
`pub --from ndjson` or `replay` put row on a wildcard is refused and
counted with the other refused rows (the closing line says "refused
row(s)", where it said "refused delete row(s)").

**A fleet `service call` whose procedure nobody could establish needs
`--i-know`** (marcpardo/zenkey#505): the forbidden-fanout guard ran only
when it found a declaration, so `service call '*' p reset --no-validate` —
or any `*` call during a degraded introspect sweep, or to a procedure the
registry does not declare — fanned out to every origin. `service call`
gains `--i-know` for exactly that; a declared (or defaulted) forbidden
fan-out stays refused, and so does a write under RFC 05 §5.1's `config/`
keys. The convention's reads — `introspect`, `describe`, a configuration
read-back — fan out with no registry, so `config get '*'` and `bench rpc
'*'` of introspect are unchanged. `bench rpc '*'` answers to the same
guard: an idempotent write declared forbidden-fanout is refused under `*`
even with `--i-know`.

**`replay` from the empty base onto the empty base needs `--force-base`**
(marcpardo/zenkey#506): two empty bases compared equal, so a base-less
staging capture republished onto a base-less production bus under its
original origins. `--dry-run` is unchanged. The base mismatch refusal
beside it exits 2, where it exited 1.

**`serve` on a wildcard, or with `--complete`, needs `--i-know`; `gen
--origin <host>` needs `--i-know`** (marcpardo/zenkey#507): a mock that
answers real GETs, and a generator that publishes as a real host, are
decisions about a bus. `serve` gains `--i-know`; `gen` reuses the one it
had, which now means "this traffic lies on purpose" — about its body
(`--fault`) or its sender (`--origin`). `gen --dry-run` asks nothing. The
`--complete`-on-`@rpc` refusal exits 2, where it exited 1, and `--i-know`
does not move it.

**`config set --confirm` with no read-back asks for the yes**
(marcpardo/zenkey#508): a reach change was asked about only when the
read-back said "reach", so under `--no-validate` — or when the read-back
met silence — a windowed change went out unasked. With no read-back the
class is unknown, and a change with `--confirm` is a reach change's shape,
so it now needs `--yes` from a script (or a terminal's yes). Without
`--confirm` nothing changed: RFC 05 §5.1 has the producer refuse a reach
change that carries no window.

| 0.10.0 | 0.11.0 | to mean it |
|---|---|---|
| `pub 'prod/v1/**' x` — exit 0, delivered | exit 2 | not overridable: name the concrete key |
| `pub --from ndjson` put row on a wildcard — published | refused and counted, exit 1 | not overridable |
| `service call '*' p proc --no-validate` — fanned out | exit 2 | `--i-know` |
| `service call '*' p proc`, `proc` undeclared or no slices served — fanned out | exit 2 | `--i-know` |
| `service call '*' p config/<r>/<g>/set --no-validate` — fanned out | exit 2 | not overridable: name one origin |
| `bench rpc '*' p proc --i-know`, `proc` a write declared forbidden-fanout — fanned out | exit 2 | not overridable: name one origin |
| `replay cap.zrec` (capture and target base both empty) — republished | exit 2 | `--force-base` |
| `replay cap.zrec` (capture base ≠ target base) — exit 1 | exit 2 | `--force-base`, as before |
| `serve 'prod/v1/**' x` — served | exit 2 | `--i-know` |
| `serve <key> x --complete` — served | exit 2 | `--i-know` |
| `serve <…/@rpc/…> x --complete` — exit 1 | exit 2 | not overridable, as before |
| `gen --origin h-…` — published as that host | exit 2 | `--i-know` (or `--dry-run`) |
| `config set … --confirm S --no-validate`, no `--yes`, not a terminal — sent | exit 2 | `--yes` |
| `config set … --confirm S`, read-back unanswered, no `--yes`, not a terminal — sent | exit 2 | `--yes` |

### Exit honesty (chunk DS)

**Three exits that said less than the run knew** (epic marcpardo/zenkey#498)
now say it, on the one contract in `src/exit.rs`.

`doctor` on a reachable bus with **nothing on it** — no producer holding an
alive token, no router answering the admin space — judged nothing, and
exited 0 even under `--fail-on error`: the coverage check compared 0 with 0.
It now exits **2**, under every `--fail-on` and without one, with the reason
in the report (`"unobservable"` in `--format json`, a silence note in the
table) and on stderr (marcpardo/zenkey#510). `doctor --transitions` reads
such a run as `unobservable` for every check. A run with anything in scope
is unchanged: findings are still output by default.

`watchdog --count N` exited 0 whatever its rules ended on. A bounded run now
exits **1** if any rule ended firing, else **2** if any ended unobservable,
else 0 — the transition stream is unchanged, and an unbounded run still
exits 0 (marcpardo/zenkey#511).

A producer, procedure path, config resource or group that is not a plain
chunk (RFC 03 §2) **panicked with exit 101** wherever it reached a key
builder — `service call h-… MyApp foo`. Every such argument (`service
call|info|list`, `config *`, `bench rpc`, `check probe|conform|schema`,
`schema show`, `topic list`, `registry export`, `blob list`, `gen`) is now
refused by clap: exit **2**, naming the argument and the grammar. `-` stays
the producer of a service-origin call; against a host or `*` it is refused
(2). `node info <hostname>`'s refusal moved from 1 to **2** with the other
refusals of input (marcpardo/zenkey#509).

| before | now |
|---|---|
| `doctor --fail-on error` on a bus with no producer and no router answering → exit 0 | exit 2, `unobservable: "nothing in scope: …"` |
| `doctor` (no `--fail-on`) on the same → exit 0 | exit 2 |
| `doctor --transitions` on the same → every check `ok` | every check `unobservable` |
| `watchdog --rule … --count N`, a rule still firing at the end → exit 0 | exit 1 |
| `watchdog --rule … --count N`, a rule ending unobservable → exit 0 | exit 2 |
| `service call h-… MyApp foo` (and every chunk argument) → panic, exit 101 | `error: invalid value 'MyApp' for '<PRODUCER>': …`, exit 2 |
| `service call h-… - introspect` → panic, exit 101 | ``Error: producer `-` stands for no producer chunk, …``, exit 2 |
| `node info myhost` → exit 1 | exit 2 |

### The operator surface (chunk DT)

What an operator meets first — the version line, the command list, the
README, a mistyped selector, an output file — said in their terms (epic
marcpardo/zenkey#498, chunk DT).

**`record -o` and `snapshot -o` refuse an existing file**
(marcpardo/zenkey#514). Both truncated it, so re-running yesterday's
command line during an incident destroyed yesterday's capture. An existing
regular file is now refused by name, exit 2, before any session opens —
`record --on` included, though its file is only written when a rule
fires. `--overwrite` replaces it; `/dev/null` and other non-regular paths
are written as they stand. The create is `create_new`, so a file that
appears meanwhile is refused too. `record` now opens its session before its
file, so a dead bus leaves no header-only capture behind.

**A base-relative selector under `--base` is hinted** (marcpardo/zenkey#512).
Wire verbs take wire keys (RFC 09 §5), so `echo 'v1/**' --base prod` was a
subscription to nothing, in silence. A typed selector or key whose first
chunk is `v1` and that does not sit under a non-empty base now gets one
stderr line naming the wire key it probably meant — `get`, `echo`, `rate`,
`field`, `record`, `snapshot`, `export`, `timeline`, `check expect`, `pub`,
`retire`. Nothing is rewritten, stdout is untouched, and `why` stays quiet
because its `key-parse` rung already says it.

**`--version` names the build** (marcpardo/zenkey#513): `zenctl <crate
version> (<git describe --tags --always --dirty>)`. The release tarball
carries its description through `git archive` (`export-subst`); a build
with neither git nor an archive behind it says `unknown`.

**The command lists speak to an operator** (marcpardo/zenkey#515). Every
short help line — `zenctl --help` and each noun's list — is one plain
sentence: no issue numbers, rule codes, RFC sections or capitals for
emphasis, at most 80 columns, checked by walking the clap tree. The
citations moved into the long help, where `<verb> --help` still prints
them.

**The README is an operator document** (marcpardo/zenkey#516): install,
the tested zenohd line, a secured-router quickstart
(`examples/prod.json5`, which a test opens a session through), the session
posture, the exit contract with a Nagios mapping, monitoring recipes, the
write guards, and every command — a test fails when a top-level verb goes
unnamed.

| 0.10.0 | 0.11.0 | to mean it |
|---|---|---|
| `record -o cap.zrec`, `cap.zrec` exists — truncated, exit per the run | exit 2, the file untouched | `--overwrite` |
| `record -o cap.zrec --on <rule> --pre S`, `cap.zrec` exists — truncated when the rule fired | exit 2 before arming | `--overwrite` |
| `snapshot -o fleet.zsnap`, `fleet.zsnap` exists — truncated | exit 2, the file untouched | `--overwrite` |
| `record -o new.zrec` against a dead bus — exit 2, a header-only `new.zrec` left behind | exit 2, no file | — |
| `echo 'v1/**' --base prod` — silence | the same silence, plus one `hint:` line on stderr | type the wire key, `prod/v1/**` |
| `zenctl --version` → `zenctl 0.10.0` | `zenctl 0.10.0 (0.12.0-…-g<commit>)` | — |

## 0.10.0 (2026-09-29) — the contract executed, and the second spelling

One new judgement and no moved spelling: a script written against 0.9.1
runs unchanged. **`check conform --producer P [--origin O] [--for SECS]
[--deep] [--junit PATH]`** (marcpardo/zenkey#222, RFC 13 §3) runs a
producer's registry as a conformance suite — one assertion per declared
surface, met / not met / unknowable, never folding the third into the
second. Every rostered origin is called (introspect and each concrete read
procedure; a write is never called), the doctor's checks are projected per
surface, and `--junit` writes the assertions with unknowable as *skipped*.
Exit 0 conforms, 1 violates, 2 unproven — on the one contract.

`zenctl` reads a registry file in either spelling RFC 08 v1.44 allows —
TOML or KDL (§5.1) — wherever it reads one: `--registry <dir>` takes
`registry/*.{toml,kdl}`, mixed file by file, and a producer's `introspect`
reply is read in the spelling its `Encoding` declares (`application/toml`
or `application/kdl`, §6; an undeclared reply is sniffed, anything else is
unreadable and named as such). **`registry export --as kdl`** is new: the
same document as `--as toml`, in the second spelling, one
`// ── <producer>.kdl` block per producer (marcpardo/zenkey#374). The
completion cache writes each slice as `<producer>.toml` or
`<producer>.kdl` by its spelling.

**`registry migrate --to kdl <dir> (--in-place | --out <dir>)`** is new
too (marcpardo/zenkey#374): the directory respelled, every `<stem>.toml` a
`<stem>.kdl` meaning the same document — unknown columns included — and
the `.lock` ledgers kept beside unchanged. Comments cross onto the nodes
they stood before; one written on or above a key is hoisted into its
node's leading block as `// <key>: …`, and the report counts them. All or
nothing: the source must lint, the result is staged, proven and linted
before it lands; `--out` refuses a non-empty directory and `--in-place`
removes each `.toml` only after every `.kdl` is written. A refusal exits 2;
a migration attempted and failed exits 1, as an act does. No spelling
moved; the exit contract is unchanged.

An `introspect` reply that answered and did not read is no longer drawn as
silence (marcpardo/zenkey#491, RFC 13 §3 O4): `node info` says
"introspect answered, slice unreadable (`<encoding>`: <first line>)" where
it said "no introspect reply", and its JSON producer row carries
`unreadable: {encoding, error}`, present only then, as does a
`node list --verbose` row, which says the same where it said "(no served
slice)" (marcpardo/zenkey#495); `doctor` without `--registry` counts that
producer as answered and files `slice-parse`; `check conform` holds its
`procedure/introspect` not met.

| 0.9.1 | 0.10.0 |
|---|---|
| — | `zenctl check conform --producer sysinfo --registry registry --for 10 --junit conform.xml` |
| — | `zenctl registry export --registry registry --as kdl` |
| — | `zenctl registry migrate --to kdl registry --in-place` |

## 0.9.1 (2026-09-29) — a fragment that pastes

`acl gen --json5` ends the `access_control` and `downsampling` blocks with a
member comma, so the printed fragment merges into a `zenohd` config as is
(marcpardo/zenkey#486). No spelling moved; the exit contract is unchanged.

## 0.9.0 (2026-09-27) — a noun for configuration, a face for a link

One new noun and one new flag family, and no moved spelling: a script
written against 0.8.0 runs unchanged. **`config get|set|confirm|cancel|
extend|persist`** (marcpardo/zenkey#473, RFC 05 §5.1) reads a resource's
served schema beside its values, changes one group typed against that
schema — the producer's own validator runs here, so a refusal reads the
same as it would on the wire — refuses a `reach` group without
`--confirm`, and asks before sending one (`--yes` for a script); `*` is
refused at the edge. **`acl gen --face constrained --link-protocol … |
--link-interface … --link-interval <SECS|none>`** (marcpardo/zenkey#475,
RFC 09 §4) plans one constrained face from the registry's `exposure`
markers instead of the enrollment's principals — the `access_control`
and `downsampling` blocks under a permissive default — and needs
`--registry`. `topic info` gains an `exposure` row. The exit contract is
unchanged.

| 0.8.0 | 0.9.0 |
|---|---|
| — | `zenctl config get h-3fa9c2d41b7e modem rf0` |
| — | `zenctl config set h-3fa9c2d41b7e modem rf0 radio frequency_khz=868100 --confirm 60` |
| — | `zenctl config confirm h-3fa9c2d41b7e modem rf0 <TOKEN>` |
| — | `zenctl acl gen --enrollment e.toml --registry registry --face constrained --link-protocol unixsock-stream --link-interval 60 --json5` |

## 0.8.0 (2026-09-25) — the watchdog reads the alert plane

One new rule and no moved spelling: a script written against 0.7.0 runs
unchanged. `zenctl watchdog --rule` and `record --on` accept
**`alert-firing <SEL> [<MIN-SEVERITY>]`** (#463): `firing` while any alert
document at or above the floor (`info | warning | critical`, default
`warning`) answers a per-tick GET under `<SEL>`, with the count and the
first alert in the evidence; `ok` at zero; `unobservable` when the ask
failed. A GET, not a subscription, so an alert already firing when the
watchdog starts is seen on the first tick. The exit contract is unchanged.

| 0.7.0 | 0.8.0 |
|---|---|
| — | `zenctl watchdog --rule "alert-firing v1/*/state/*/alert/* critical"` |

## 0.7.0 (2026-09-07) — the explorer suite, executed

Eight new observation surfaces and no moved spelling: a script written
against 0.6.0 runs unchanged. `snapshot` (+ `snapshot diff`, with
`--normalize-origins`/`--map`), `timeline`, `export`, `service call
--trace`, `record --on/--pre/--post/--every/--preamble`, `replay
--seed-state`, `registry consumers|impact|infer`, `registry lint
--allow-drafts`. And one row that was empty: `get` prints the responder's
HLC on a reply when the responder stamped it.

**`export` — a metrics surface that exports its own blind spots** (#228).
A root wire verb: `zenctl export --bind 127.0.0.1:9184` serves
`/metrics` as Prometheus text (RFC 13 §3 *Exporter obligations*, v1.34),
two families deliberately apart. **Observer and contract metrics**:
`zenkey_observer_dropped_total`, `zenkey_observer_evicted_total{population=
keys|retained_bytes|retained_age|unwatched}` (four lines, never summed),
`zenkey_observer_coalesced_total` (between two scrapes only the newest
value per series is exposed; the rest are counted), `zenkey_observer_
unstamped_total`, `zenkey_qos_judged_total` and `zenkey_qos_mismatch_total
{producer,subject}`, `zenkey_payload_verdict_total{verdict=valid|invalid|
not_validated}` (three lines, never a ratio; without `--validate` everything
is `not_validated`), `zenkey_doctor_finding{check_id,severity,subject}` and
`zenkey_doctor_info{state=not_asked|ran}`, `zenkey_scope_info{selector,
excluded}` naming the planes a wildcard cannot reach, `zenkey_registry_info
{state=loaded|not_loaded}`, `zenkey_series_suppressed_total{reason}`,
`zenkey_unregistered_keys`. **Key metrics from the contract**:
`zenkey_subject_<producer>_<literal chunks>[_<unit>][_total]` — name and
unit from the registry's `unit` and `kind`, never sniffed; every `{var}` a
label by its declared name; the declared `cardinality` bounds the
population and the refusals are counted. A series that stopped keeps its
labels and `zenkey_series_state{state=evicted|origin_down|retired}` and
loses its value line — absence is named, never a flat line; `quiet` is
judged only for `state` subjects against `ttl_s`; every series carries
`zenkey_key_last_seen_timestamp_seconds` (constant between scrapes, so an
idle scrape is byte-identical) and `zenkey_series_drop_exposed_total`.
Flags: the `SelectorArgs` (default `<base>/v1/*/**`), `--bind`
(non-loopback needs `--i-know`; `--listen` stays the transport's), `--validate` (2 decodes per key per
second), `--doctor-every SECS` (off by default — it costs the control
plane), `--max-series N` (10 000), `--once` (observe `--for` seconds, fold
once, print the `export` report through `--format`; `--prom` prints the
exposition instead and is refused with `--format`). Refused up front: OTLP,
histograms and summaries, push gateways and remote write. The HTTP server
is hand-rolled (`cmd/export/http.rs`): `hyper` is not in zenctl's graph and
one route does not earn a framework; the switch point is a second endpoint.
**`record --on <RULE> --pre <SECS>` — trigger capture** (#218; RFC 13 §4.1
version 2). New flags on `record`, no moved spelling: `--on` (repeatable,
the watchdog's rule vocabulary; requires `--pre`), `--pre <SECS>` (the
retained window's age budget), `--post <SECS>` (default 10), `--every
<SECS>` (the one period flag, default 1), `--preamble
absent-from-window|full|none` (default `absent-from-window`). With `--on`
the verb arms instead of records: nothing is written until a rule
transitions to `firing`, and then one `.zrec` version-2 file carries the
state preamble (a bounded GET on the state-class projection of the watch
set — the header names what it could not fetch), the pre-roll at its real
`t`, the trigger record where it fired, and `--post` seconds more. `--for`
keeps its passive-window meaning — give up after this long, exit 0 with a
silence note (a rule not firing is not a finding, RFC 05 §3.1) and no
file; `--count` stays the post-roll's stop bound. The report says the
pre-roll covers only the watched selectors (O5), how much of `--pre` the
ring could give, and the ring's two eviction kinds apart (O6).

**`replay --seed-state`.** A version-2 capture's preamble rows are skipped
by default and said per row — ndjson `{"row":"would","would":
"skip-preamble",key,reason}`, table `would skip <key>  (preamble — …)` —
with the closing count "preamble rows skipped: N (--seed-state to publish
them)" (RFC 13 §4.2: re-stamped state-at-capture-start republishes a
snapshot over the live fleet). `--seed-state` publishes them, counted as
seeded apart from the observed rows. A trigger record is announced where
it fell and never published. `timeline --from` counts preamble rows it
did not place.
**`registry infer --from <selector|capture.zrec> --for SECS --out DIR` —
draft a registry from the wire, marked as a draft** (#225, RFC 08 §6.1
v1.34). The registry is the adoption cliff: a fleet without one gets nothing
from this suite's best half, and hand-writing two hundred entries is why it
never gets one. This verb watches a selector for `--for` seconds (default
60; `--from` defaults to `<base>/v1/**`) or reads a `.zrec` capture under
its own base, and writes one `<producer>.toml` per producer seen, a
`types.toml` with inferred JSON Schemas and their `schemas/*.json`
sidecars. Every file carries `draft = true`, `compat = "none"`, no `since`
and a header saying every field is a guess; `zenkey-build` **refuses** the
marker until a review removes it, and `registry lint --allow-drafts <dir>`
closes the loop meanwhile. `{var}`s come from sibling structure and
per-origin populations (the heuristic and its six named failure modes are
in `zenkey_fleet::model::infer`'s doc; each rides the entry it produces as
a `#` comment), units from RFC 08 §4's suffix rule, `rate` and a `ttl_s`
hint from counts over the window, `cardinality` from the largest
population one origin published — and every field the observation could
not establish is **absent**, never defaulted. Observed QoS is a comment
and never a field (writing it would launder a current publisher bug into a
contract); the one `qos` written is the alert family's `"alert"`, which
RFC 08 §5 requires and the entry says so. `--out` must not already hold any
file the run would write: refused whole, exit 2, before a byte is written.
`--app` names the owning application (default `"unknown"`); `--max-keys`
and `--max-paths` bound the observation (O6) and the report states what
each refused. Family `registry-infer`: the ndjson rows are the subjects
and types, each naming the file it lands in.

**`registry lint --allow-drafts`** admits `draft = true` files as a build
with `Config::allow_drafts(true)` would; each draft is still a warning.

Two new verbs under the `registry` noun and no moved spelling (#224).

**`registry consumers <target>` — who declares a reader of a subject.** A
join over the admin space: every declared subscriber and querier with its
verbatim keyexpr, related to the target by key algebra (`exact` <
`narrower` < `wider` < `intersects` < `total`), one row per declaring
session on the admin `sources` (reported-only when they name none), joined
to the origin its alive token attaches (#131) and the topology's `whatami`.
`<target>` is `<producer>/<subject-path>` resolved through the slices to
the family's wire selector, or a raw key/selector (anything with `*`/`@`,
or starting at the base or `v1/`). The honesty is the product: an admin
space that does not answer is `admin: not_available` with no rows — *not
asked*, never an empty set (RFC 13 §3 O4); a declaration is not proof of
use; a `**` declaration is `total` and flagged `total_wildcard` rather than
read as a consumer of this subject; the tool's own session appears and is
named `(this zenctl session)`; the answering admin spaces are counted. It
is not matching status (RFC 12 §9), and the render corpus greps the
family for "matching", "listening", "unmatched" and "no consumers".

**`registry impact <producer>/<path>` — the blast radius of changing one
declared subject.** The consumers above, plus the RFC 04 §2 storage
coverage row (made only when an admin space answered — an empty storage
list would read "uncovered"), the distinct sessions declaring a publisher
or queryable on the family, and the `[[deprecated]]` ledger entry. A path
that survives only in the ledger still resolves, class wildcarded.

Families `registry-consumers` and `registry-impact`; rows tagged
`consumer` and `coverage`; the admin discriminator rides flat in the
envelope (`admin`, `answered`, `nodes`).
**`service call --trace [--for SECS]` — call → effect, the RPC trace
window** (#215). RFC 05 §3's long-running idiom is a declared causal chain
— `GET @rpc/<p>/artifact/request` → `state/<p>/artifact/<kind>` →
`events/<p>/artifact/<ulid>` → `@blob` — and nothing followed it: you
called a write procedure and then hunted three panes for what it did. With
`--trace` the call keeps a window open on the called origin (default 10 s)
and lists, after the reply, every sample observed there: Δ on the arrival
clock always, Δ on the HLC against the reply's where both are stamped (the
stamper named — `self`, `foreign:<id>`, `unattributable:<id>`), each
tagged `declared-chain` (the registry refines it under the called producer
and it shares the procedure's first chunk — a **naming heuristic**, and the
report says so in a fixed `chain_rule` field), `same-origin, not declared`,
or `same-origin, registry not loaded` (unjudgeable is not undeclared, O4).
Other origins are a count and a few keys in a *concurrent, not attributed*
lane, never rows. **Subscribe, then call, then hold** is the order and the
report pins it (`subscribed_before_call`): a window opened after the call
would turn "not asked" into "no". The wording is *observed after the call*
throughout — never *caused*; no edge, no arrow, no trace-id attachment.
`**` never crosses an `@`-chunk, so the `@blob` bytes are outside the
window and the report says so rather than widening. Not a verb of its own:
one act, one spelling. `--trace` with `*` exits 2 (a trace attributes to
one origin). The exit code stays the call's — what the window saw is an
observation, not a judgement. A zenctl-level smoke against `gen
--serve-describe` shows naming attribution only: the generator never
publishes *after* a reply, so the ordered chain is pinned by the engine's
bus test (`zenkey-fleet/tests/trace.rs`). Not in this cut: the zengui hook
(`SendForm.trace`, an Effects section) is named and left for the GUI.

**`timeline` — the fleet timeline, and deliberately no edges** (#216).
A new wire verb at the root. `zenctl timeline <SEL>… --for <SECS>
[--order arrival|hlc]` watches the selectors for the window and emits one
merged ordering of everything seen, partitioned into lanes per
origin/producer, with the clock stated per report and the stamper(s) per
lane. Every ndjson row carries `order_by`, so a line cut out of the
stream still says which axis its `pos` is on. Under `--order hlc` the
envelope carries the claim the axis can make (RFC 09 §5.1 O7): one
stamper is that node's happened-before; several are skewed wall clocks;
an empty axis claims nothing. Unstamped samples get their own lane on
the arrival axis and **cannot be placed on the HLC axis at all** — the
engine's `Placed<HlcAxis>` refuses them, and the report counts the
exclusion. Drops render as breaks at their arrival position and as a
total with no position on the HLC axis. The per-publisher
sequence-number lane is reported *unavailable* on this zenoh (no
`SourceInfo` reaches a subscriber), never empty. `--from <FILE>` reads a
`.zrec` through the same projection under the capture's stated base, and
the engine's identity test is what makes "the same window from the
file" a claim. No line is ever drawn between lanes: a merged ordering
shows when things were seen on which clock, never that one caused
another.

**`snapshot` — a fleet moment you can keep, verify and diff** (#219,
RFC 13 §4.4). A new wire verb off the root, no spelling moved. `zenctl
snapshot [SELECTOR] --out fleet.zsnap` runs one fan-in GET per selector
(the `--origin/--class/--producer` composition every watcher has), folds
the replies per key last-writer-wins, and writes one row per key: the
exact payload as base64 `bytes`, whose clock stamped it (`stamper`, O7),
the registry rung (`registration`, the `topic info` vocabulary — including
`registry_not_loaded`, which is not `unregistered`), the three-valued
`verdict`, and who **holds** it — `live {origin, answered_by}` (its origin
held an `alive` token during the collection, and whether the replier was
the stamping entity), `storage_only {origin}` (a value answered, nobody is
saying it now) or `unattributed {reason}` (`--no-roster`, or a key that
names no origin). The header states the **collection span**: a fan-in GET
is collected *over* a span, never at an instant, and every rendering of a
snapshot says so. `--max-replies` bounds what is kept; what the bound cost
rides the header as `elided`, beside `superseded` (LWW losers) and
`errors`. **Exit 0** wrote the file; **exit 2** nobody answered — silence
is not a snapshot, and no file is written for it (`exit.rs`).

`zenctl snapshot diff a.zsnap b.zsnap` opens no session. Rows tagged
`added | removed | changed`, a `changed` row carrying only the facets that
moved (`value` or `bytes`, `verdict`, `registration`, `holder`) with both
stamps; the envelope carries both headers whole. **Exit 0** identical,
**1** they differ (a difference *is* the finding), **2** a file could not
be read — through the one judgement projection, never a hand-rolled
match.

**`snapshot diff --normalize-origins` — two deployments, one diff** (#220).
"It works in staging" is unfalsifiable on a bus until two fleets can be
compared subject by subject; RFC 03 §1.1 makes that possible, because
publishing identity sits at one fixed base-relative position. The engine
profiles every host on both sides and plans the alignment on three kinds
of evidence, in order and never by guessing: an explicit `--map A=B` (a's
origin = b's; repeatable, requires `--normalize-origins`, refused at the
edge when it names an origin the files do not hold), the `source` label
the health/sensor documents carry (RFC 06 §6.2) when it is verified —
`host_id` is the origin it sits under — and unique among the unpaired on
both sides, and a producer set unique on both sides. `b` is then read
through the plan (origin chunk, base, holder, the bridge document's
`host_id`) and compared as before, and the diff **rolls up per subject**:
one `subject` row per subject across every origin — "`state/sysinfo/health`
differs on 2 of 2 origin(s); `state/logs/rotated` 1 only in a" — with
one example key change; subjects identical everywhere are counted, not
listed. Every pair rides `origin_map` with its evidence (`explicit` /
`label <source>` / `producer set`); two deployments' clocks are not
compared, so a stamp that moved alone is not a change here. **Exit 0**
identical, **1** they differ, and **2 — refused**: an origin the plan
could not pair is listed as an `unmapped` row with the count it failed on
("label `node` claimed by 2 origins in b; producer set {sysinfo} shared by
2 origins in b"), the comparison is *not made* — no `added`/`removed`/
`changed`, no roll-up, the word is NOT COMPARED — and the report still
goes out so a script sees exactly what to `--map`. A diff that compared
around an origin it could not place would be confident nonsense; "I
cannot map these" is the finding.

## 0.6.0 (2026-09-06) — the generators, and three rows that name a host

Two new verbs and no moved spelling: a script written against 0.5.1 runs
unchanged. Besides the two generators below: `interface show --schema`
names both hosts of a schema disagreement instead of recomputing one over
producer-keyed rows (#410); `call` and `check probe` print *stopped early*
under a partial page and a caveat when the cursor is null (RFC 05 §3.2,
#424); `topic info` gains a `kind` row (RFC 08 §2 v1.32, #422); `doctor`
carries `kind-mismatch` and, under `--deep`, `budget-exceeded` (#391);
`registry export --as toml` keeps a producer's `[budget]`.

**`acl gen` — RFC 09 §3's grant matrix, generated** (#392). A new `acl`
noun with one verb. `zenctl acl gen --enrollment <file.toml>` reads a small
TOML binding certificate CNs to roles (`host`, `catalog`, `console`,
`desired-author`, `watch`) and origins — given, or *computed* from a
machine-id with the RFC 06 §1 derivation, and refused when the two disagree
— and plans the router's `access_control` block: one rule per plane per
host because `**` never crosses `@rpc`/`@media`/`@blob` in inclusion
(fact 1), the catalog spelled on its own because `*` never covers it
(fact 2), rules *and* subjects *and* policies (fact 3), every consumer's
declarations allowed by name (fact 4), and — the fifth fact, from the
reference deployment — a shared egress-only `interest-prop` rule on every
publishing policy, without which a peer-mode publisher publishes to
nobody. With `--registry`, the planes narrow to what host producers declare
and `no-remote-actions` denies exactly the declared write procedures;
without one, the plan says so. `--json5` writes zenohd's block (field for
field zenoh 1.10, a comment per rule naming its row and its fact), a foreign
schema that conflicts with `--format` the way `--dot` does. A refused
principal exits 1.

`--check --against <router.json5>` diffs the plan against the block a
router's config file carries, read through zenoh's own loader — the running
block is not observable, because zenoh 1.10's admin space serves no GET
under `config/**` — and exits 0/1/2 through the one judgement projection;
the interest-propagation probe is *not asked*, and the report says why.
`--explain <principal> <key> <message>` answers per direction with the rules
that decided, inclusion by zenoh-keyexpr, exit 0. Zid-bound subjects need
`--allow-zid-subjects`, because zenoh's own config says a ZID is not
authenticated.
**`storage gen` plans the router's storages from the registry** (#393) — a
new verb under the `storage` noun, no flag or spelling elsewhere moved.
RFC 09 §2 specifies class-driven storages whose `garbage_collection.lifespan`
must be ≥ the longest `ttl_s` in the registry, and the registry knows that
number; nobody computed it. `zenctl storage gen --deployment <file.toml>`
takes a small TOML — the base, the volumes with their plugin (and, for
`redb`, the per-volume history mode of §2.1 v1.28), and per storage a class
(`state | telemetry | events | catalog | catalog-pdns`, or a base-relative
`selector`) and a volume — and derives the rest: the selector (`@catalog`
explicit, because `*` never matches it), the `strip_prefix` as the literal
leftmost run, and the lifespan as ceil(max covered `ttl_s` × `gc_margin`),
with the computation shown.

It refuses what the router would refuse — replication on an all-mode volume
(§2.2), a volume nobody declared, a class the registry declares nothing
under — and emits the rest of the plan around the refusal, naming it. It
warns, citing the clause, where a caveat applies: overlapping selectors
(§2), `complete = true` off the replicated latest storage (§2.2, emitted as
`false`), retention that is the database's and not zenoh's (§2.3, influx),
`redb`'s mandatory retention in all mode (§2.1), a seed on a volatile volume
(§2.1). Without a registry every lifespan is §2.3's default and says so
(`slices_optional`, #210) — not asked is not empty.

Four ways out. The plan report in the usual three renderings (families
`storage-plan`, rows `volume` / `storage` / `refusal`); `--json5`, the zenohd
`plugins.storage_manager` block with every derivation and warning as a
comment beside the storage it concerns — a foreign schema, so it is a flag
of its own and conflicts with a typed `--format`, like `--dot` and `--as`
(#243); `--check`, a verdict verb comparing the plan with the storages the
admin space reports (family `storage-check`: missing, extra, a differing
`key_expr` / `strip_prefix` / `volume`, a `gc.lifespan` below the computed
minimum; exit 0 as planned, 1 a difference, 2 no verdict — an empty admin
sweep included, since a peer-only mesh is not a router running the plan);
and `--explain <key>`, which planned storage takes a key and why (family
`storage-explain`). `gen` alone is an act: 0, or 2 when every storage was
refused.

Deliberately **not** in the RFC yet: 09 §2's notes will say that this verb
emits the block with `lifespan` derived, and that sentence needs a changelog
entry and a header bump of its own. The verb's `--help` carries it meanwhile.

## 0.5.1 (2026-08-29) — what the fleet does not agree about

No command moved and no flag changed. Three behaviour changes, all of them
this tool saying something it used to leave out.

**`registry diff` no longer prints `agree` about a comparison it made
against one of several answers** (#399). The diff is computed from one slice
per producer, so against a fleet mid-rollout it was computed from one
arbitrary host's — and the producer that happened to match your checkout
read as clean. A split producer now marks `✗` and details both hosts and
both versions, the envelope carries `self_disagreeing`, and `--format json`
gains a `collapsed` key.

The key is **absent**, not `[]`, when the served side did not come off the
bus: a diff computed from `--registry` files never asked how many origins
serve each producer, and a note says so. Scripts keying on `collapsed`
should treat absence as "not asked", never as "the fleet agrees".

**Every registry-aware verb says when its answer came from a pick** (#399).
A new stderr note, beside the existing bus-versus-checkout one and worded to
be unmistakable for it: that one says the fleet disagrees with your
checkout, this one says the fleet does not agree with itself. It appears on
any verb that loads slices from the bus.

**`watchdog` reports a write failure where it happens** (#397). It used to
keep the first `io::Error` and answer for it after the run, because the
engine's emit callback could not fail — so the run went on ticking against a
consumer that had gone, emitting into a local variable. A closed pipe still
exits 0 (`| head -1` is not a failure of the checks) and any other write
error still exits non-zero, but both now **stop the run** at the failed
write instead of at the tick bound, which is what Ctrl-C already did and
what `doctor --transitions` has always done. The summary line is not printed
in that case: the run did not finish, and a count of the ticks it happened
to reach is not the summary of anything.

**`doctor`'s `schema-drift` finding names the host** (#398), as
`producer@origin (hash)`. And it fires in a case it previously could not see
at all: one producer served by two hosts at two different schema hashes —
a half-rolled-out sensor, which is the likeliest schema disagreement there
is. The `check` id and the finding's `subject` are unchanged; the hosts ride
in `evidence`.

## 0.5.0 (2026-08-25) — the command tree, the flags, and the exit contract (#307)

**This is a breaking change with no alias layer and no deprecation shims.**
The old spellings are gone, not warned about. That is deliberate: an alias
layer is a second tree to keep honest, and the whole point of the change was
to stop having two of anything.

### Why

Three vocabularies had drifted, each in a way you could only see by reading
the whole surface at once:

* **Depth meant nothing.** `topic echo` subscribed to live traffic while
  `topic list` read a registry document; `schema <producer>` was a noun that
  was also a verb, and `zenctl schema` with no argument exited 1 where every
  other missing argument exits 2. Meanwhile `expect`, `cutover` and `probe`
  sat at the root beside `get` and `scout`, sharing an exit contract with each
  other and nothing else.
* **One concept had four spellings.** A passive observation window was
  `--window` on `cutover` and `hz`, `--within` on `expect`, `--duration` on
  `record`, and `--listen-for` on `doctor`, `why` and `registry retired` — and
  three of those were `u64` while three were `f64`.
* **Exit 2 meant two things.** The corpus pinned both doctrines side by side:
  `session-config-error.trycmd` argued *your input is a 1*,
  `key-algebra.trycmd` argued *an invalid expression is a 2*. No script could
  hold both.

The contract is now written once, in prose, in `zenctl/src/exit.rs`, and every
verb cites it.

### The tree

| was | is |
| --- | --- |
| `topic echo` | `echo` |
| `topic pub` | `pub` |
| `topic retire` | `retire` |
| `topic hz` | `rate` |
| `topic bw` | `rate --bytes` |
| `schema <PRODUCER>` | `schema show <PRODUCER>` |
| `schema check` | `check schema` |
| `expect` | `check expect` |
| `cutover` | `check cutover` |
| `probe` | `check probe` |
| `registry retired` | `check retired` |
| `blob probe` | `blob locate` |

`topic` keeps `list` and `info` — the two things that read a registry
document. `get`, `field`, `record`, `replay`, `serve`, `gen`, `scout`,
`doctor`, `why` and `watchdog` are unmoved. `bench rpc`, `key`, `node`,
`base`, `service`, `interface`, `storage`, `admin`, `registry`, `context`,
`cache` and `completions` are unmoved.

`rate` is a merge, not a rename: `--bytes` selects the byte view and
`--per-key`/`--loss`/`--latency` work as they did on `hz`.

### The flags

One spelling per concept; every duration is `f64` seconds.

| was | is | on |
| --- | --- | --- |
| `--window <SECS>` | `--for <SECS>` | `check cutover`, `rate` |
| `--within <SECS>` | `--for <SECS>` | `check expect` |
| `--duration <SECS>` | `--for <SECS>` | `record` |
| `--listen-for <SECS>` | `--for <SECS>` | `doctor`, `why`, `check retired` |
| `--budget [<SECS>]` | `--budget` + `--for <SECS>` | `topic list` |
| `--watch [<SECS>]` | `--watch` + `--every <SECS>` | `topic list`, `base list`, `storage list` |
| `--tick <SECS>` | `--every <SECS>` | `watchdog` |
| `--ticks <N>` | `--count <N>` | `watchdog` |
| `--runs <N>` | `--count <N>` | `doctor` |
| `--watch` | `--transitions` | `doctor` |
| `--interval <SECS>` | `--every <SECS>` | `pub` |
| `--repeat <N>` (default 0) | `--times <N>` (default 1) | `pub` |
| `--count <N>` | `--at-least <N>` | `check expect` |
| `--count <N>` | `--calls <N>` | `bench rpc` |
| `--from <ORIGIN>` | `--origin <ORIGIN>` | `blob fetch` |
| `--i-know` (width guard) | `--wide` | `gen` |

The rules those follow, so the next flag has somewhere to go:

* **`--for`** is every passive observation window.
* **`--timeout`** is reply-wait and nothing else.
* **`--duration`** bounds *generated output*, so only `gen` has it.
* **`--watch`** is a bare bool; **`--every`** is the one period.
* **`--count`** is a stop bound. An assertion is `--at-least`, an experiment
  size is `--calls`, a repeat count is `--times`.
* **`--from`** names an input source, never an origin.
* **`--i-know`** is one guard per verb. `gen` had two on one flag —
  acknowledging a wide run also armed the fault injector — and now spells the
  second one `--wide`.

`pub --repeat 0` and `--repeat 1` used to be the same number. `--times`
counts: 0 publishes nothing, 1 publishes once.

New: **`echo`, `rate`, `record`, `field`, `check expect` and `why` all take
the same selector shape** — a positional selector *or* `--origin`/`--class`/
`--producer` composed server-side, never both. `check expect`, `field` and
`why` used to force a hand-written selector.

New: **`field --fail-on <SEVERITY>`**, the opt-in `doctor` already had. The
asymmetry was the bug; the default is still exit 0.

### The exit contract

| code | meaning |
| --- | --- |
| **0** | asked, and the answer is clean — values, assertion met, pass, valid, healthy, act completed |
| **1** | asked, and the answer is a **finding** — not met, the old family still speaks, an invalid payload, an error reply, rows refused or malformed, `--fail-on` tripped, an act that failed |
| **2** | **no verdict** — the question could not be asked, or could not be proven |

What moved:

* **`why` flipped polarity.** It exited 0 on "a cause was found" and 1 on "the
  fleet looks healthy", which pages you for a healthy fleet. It now exits 1 on
  a finding and 0 when nothing is wrong. `zenkey_fleet::WhyVerdict::to_judgement`
  carries the inversion and `judgement_exit_code` projects it, so zenctl and
  zengui cannot drift on what a `why` verdict means.
* **Refused input is 2, everywhere.** An unresolvable `--context`, a `--qos`
  outside RFC 04 §3's five, a class outside RFC 04 §1's three, an unknown
  `--fault` kind, a `$*` selector (RFC 03 §2), a `--zenoh-config` this tool
  refuses, a `--registry` dir that will not read, a window given as zero
  seconds, an inert flag for the target you named, `gen`'s two guards: all
  exited **1** before, and all exit **2** now. Clap already exits 2 for every
  mis-shaped command line and cannot be argued with, so everything else joined
  it rather than competing with it.
* **`bench rpc` exits 1 when any measured reply was an error envelope.** It
  used to exit 0 with a latency distribution over failures.
* **`registry diff` with no `--registry`, and `registry export` with nothing
  to export, exit 2.** Neither is a finding: one question cannot be put, the
  other found silence.

Unchanged: acts (`pub`, `retire`, `replay`, `gen`, `blob fetch`) keep 1 for
their own failures — "unprovable" has no meaning for an act — and the verdict
verbs keep the pre-run guard that turns *every* setup failure into 2 rather
than a claimed verdict.

### Also

* `doctor`'s summary said what `registry diff` says. It now says what doctor
  does: check the fleet against the contracts it claims.
* `zenctl/tests/cmd/` is regrouped to match: `help-<noun>` per family,
  `help-wire` for the root verbs, `help-check` for the assertions.
