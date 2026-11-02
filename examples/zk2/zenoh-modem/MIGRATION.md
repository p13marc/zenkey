# Migrating zenoh-modem to zk2c (#991)

*This guide is refreshed as each branch it names merges. It was last refreshed at zenkey's `main`
`ec5dbdc1`.*

This is the page a zenoh-modem maintainer reads to move zenoh-modem onto zk2c, the
bandwidth-first wire of epic #975. It says what modem-mgmt, the backends, the routers and the
ground tools change, in what order, against what zenkey's `main` provides today. It also says
what is still missing.

It extends [`docs/zk2/migration.md`](../../../docs/zk2/migration.md), the general v1→zk2 guide,
and does not repeat it. The mapping itself is this directory:
- [`README.md`](README.md) maps every v1 entry and lists the gaps;
- [`contract-crate.md`](contract-crate.md) says what `modem-contract` becomes;
- [`config.v1-binding.md`](config.v1-binding.md) binds the configuration plane;
- [`link.md`](link.md) and [`faces/`](faces/) hold the radio faces.

This page is not normative. The rules are [`spec/core.md`](../../../spec/core.md), cited
"core §n", and the profiles in [`spec/profiles/`](../../../spec/profiles/README.md). The byte
counts come from each issue's design, posted on the issue, and from #989's baseline
([`docs/zk2/measurements/wire-baseline/`](../../../docs/zk2/measurements/wire-baseline/README.md)).
A figure is a model unless it says measured. It reads zenoh-modem at `d8c62df`.

**Three states.** *Landed* is on `main`; a row a refresh moved names the PR that merged it.
*Drafted* is pushed on the branch named, or open in the PR named, and not merged: read it with
`git show origin/<branch>:<path>`. *Missing* is not written yet, or not pushed, and its issue
says what it will be.

## 1. Where it stands

| Piece | State | What there is, and where |
|---|---|---|
| **The runtime and codegen** | Landed | `zenkey` and `zenkey-build` 0.20: `ServiceBuilder`, the generated `Server`, `Consumer` and `Client`, packed structs, typed frame writers ([`docs/zk2/codegen.md`](../../../docs/zk2/codegen.md)) |
| A published release | Missing | Every zk2 crate is `publish = false`, and no release tag of them exists: the release tags end at v1's `0.14.0` (#929) |
| `modem-contract` as a zk2 contract crate | Missing: zenoh-modem's to build | [`contract-crate.md`](contract-crate.md) is its design |
| **The contract, `modem.v3` 3.3** | Landed, PR #1059 | Ranges taken from zenoh-modem's code; `frames/fast`, `frames/slow` and `frames/paid` over one record of all 73 readings, the largest frame 216 B; 3.2's nine frames deprecated and `host`; the seven states' packed twins under `packed/`; `uses` `telemetry.v2`, `freshness.v2` and `link.v1`; [`catalog.json`](catalog.json) `e3a529fe`, index 8. The faces list one frame each, SF9's lease is 480 s and SF12's 3,600 s, and `link.v2`'s plans are derived from 3.3 (#991) |
| `modem.v3` 3.2 | Landed, deprecated by 3.3 | Nine frame resources over packed records, at illustrative ranges; catalog `ab76ec37`, index 6 |
| **The wire:** packed records | Landed | Core §7.6; generated structs carried by `zenkey::codec::Packed<T>` (#979) |
| Text, lists and dictionaries in packed records | Missing | #979's B3 |
| The in-band stamp, a run and a seq | Landed | Core §4.3, §7.5 (#976) |
| Frames | Landed | Core §7.7; `zenkey::frame::FrameWriter` (#981) |
| Event keys by run and seq; the compact descriptor; the archive attachment | Missing | #980's C2, C5 and C3 |
| **Telemetry,** `telemetry.v2` | Landed | A frame's fields as metric series. The frame writer meters each counter; `zenctl telemetry` and zengui's Series read frames (#983) |
| **Freshness,** `freshness.v2` | Landed | One heartbeat per interface in place of re-puts. `modem.v3`, `config.v1` and `health.v1` use it (#978) |
| **The catalog,** and keys on a hop | Landed | Core §8.6 and §8.5: `ServiceBuilder::catalog`, `--catalog` on zenctl and zengui, `zk2 catalog`, `zenctl catalog`, zenctl's `--across-face` scope (#980's C1) |
| **Configuration,** `config.v1` | Landed | `ServiceBuilder::config_v1`, `zenkey::config_v1::{Startup, Device}`, the compact status, `zenctl config`, zengui's Config tool |
| A configuration owner's re-mint | Landed | `Service::new_epoch` on a `config.v1` or `config.v2` service restarts its configuration in the new instance (`config.v1` §2.12): a pending change cancelled, the device returned to its startup values through `Device::restore_startup`, revision 0. Refused while an act is in flight, for a runtime value that starts unknown, and when fed through `desired.v1` (#1055, PR #1063; #1064 for `config.v2`) |
| **Calls:** the value and error forms | Landed | Core 0.94 §5.1 and §5.2. A caller writes its request as `CH · record` under `zenoh/bytes`. An owner answers in the value form, or refuses in the error form, each with its as-of mark. `Call::reply`, `summary` and `refuse` write the forms, and `Client` and the generated `Client` write the request (#985's C4a to C4c; C4c is PR #1057) |
| The bare wire's removal | Missing, in progress | An owner on `main` still reads a bare request, under its type's `Encoding` (core §5.1's MAY), and a caller a bare reply. C4d removes both (#985's C4d) |
| **`config.v2`,** its text and its model | Landed | `config.v2` 0.1, draft since G4: the groups declared in the adopter's contract (`config.groups`), one packed record in place of the view and the status, and the derived `Record`, `Set<T>`, `Control` and `Error`. `zenkey_model::config::v2`; `zk2 config derive` prints the derived types, and `zk2 contract lint` runs its lint (#985's G1, PR #1058) |
| `config.v2`'s owner | Landed | `ServiceBuilder::config_v2(Startup, Device)` and `zenkey::config_v2`, over the service's own config contract. zenkey-build lints a contract that uses `config.v2` and generates its derived types. The owner example's `--config-v2` runs the spec's `rfcfg.v1` (#985's G2, PR #1061) |
| `config.v2`'s tools | Missing | `zenctl config` and zengui's Config tool read `config.v1` only (#985's G3) |
| `sx1262_config.v1` and its binding | Landed | [`config.v2-binding.md`](config.v2-binding.md), [`config/sx1262_config.v1.toml`](config/sx1262_config.v1.toml), linted clean on `config.v2` 0.1; catalog `8c808dff`, index 7. `zenkey/tests/zenoh_modem_config.rs` runs the binding's §15 round trip, byte for byte (#985's G4) |
| **The link,** `link.v2` | Landed | The policies in [`faces/`](faces/); `zenctl acl gen --face-policy … --catalog …` and `--far-config`; `zenctl check face` with `--peer-config` and `--catalog` (#988) |
| The far side across a face | Landed | `Consumer::across`, `Client::across`, `FacePolicy::far_side`, and zenctl's `--face-policy F --face NAME`: the face's timeout, transport and re-declaration (#988's L4). A re-declare holds no lock across the network, and a failed one is retried at every check and warned of once a horizon (#1047, PR #1056) |
| `check face` on the fork's binding | Landed | `check face` with its `--peer-config`, and `acl gen --check`, read a `zm-1.10.2-r1` router config as data, in zenoh 1.10.1's shapes and the binding's link protocols. zenoh-modem's five router blocks pass unedited (#1044, PR #1056) |
| The readers' `wait_before_close` per binding | Missing | zenctl's defaults are zenoh 1.10.1's 5 s, and the fork's is 10 s (#1011, item 3) |
| **Presence:** a tokenless provider found | Landed | A consumer reads absence only from a complete read (#1018, for #982) |
| A first hop holds its instance token alone | Drafted, `chunk-h1` (text) | The runtime's `first_hop` and the tools are missing (#982's H2 and H3) |
| A new epoch on a running service | Landed | `Service::new_epoch`; on a service that implements `config.v1` or `config.v2`, a restart of its configuration (#1055, #1064) |
| **Snapshots,** one reply per collection | Drafted, `chunk-e1` and `chunk-e3-spec` (text) | The readers, the owners and the old path's removal are missing (#977) |
| **The measurement** of W1 and W2 on zk2c | Missing | #989 |

## 2. The order to migrate in

Each step says what changes in zenoh-modem, which zenkey API or tool it uses, how to check it,
and what it saves. Steps 2 to 7 change code. Step 8 cuts a node over: its router and its daemon
move together.

| Step | What | Needs |
|---|---|---|
| 1 | The routers' transport | nothing of zk2 |
| 2 | Build on zenkey 0.20 | #929, or a git dependency |
| 3 | `modem-contract` becomes a contract crate | step 2 |
| 4 | modem-mgmt serves each device as a zk2 service | step 3 |
| 5 | Telemetry crosses as frames | step 4; 3.3's frames, never 3.2's |
| 6 | States and events, confirmed by the heartbeat | step 4 |
| 7 | Configuration | step 4; `config.v1` now, `config.v2` once its tools and `sx1262_config.v1` land |
| 8 | Each node's cutover: the generated access control | steps 4 to 7 |
| 9 | The ground side | step 8 |

### Step 1. The routers' transport, now

- **Changes.** `configs/zenohd-rf0.json5` moves from a 10 s lease, 4 keep-alives and a 10 s
  `wait_before_close` to 240 s, 4 and 240 s, and so does the far end's file. The LoRa routers,
  which ship no config yet, take their policy's `[transport]`. SBD's stays as shipped (§3.3).
  Only the routers' files move. The daemons stay as shipped, on v1, until step 8 moves each node's
  router and daemon together. This step reaches a daemon only through the lease on its loopback
  link (below).
- **Uses.** Each policy's `[transport]` block. `zenctl acl gen --face-policy … --json5` writes the
  router's `transport` and `routing` blocks, and `--far-config` writes the far end's.
- **Check.** Diff each file's transport against `acl gen`'s output. `zenctl check face` compares
  each key, and with `--peer-config` both ends. On the fork's binding it reads each file as data,
  so whether the fork's own loader accepts it is not checked (§3.2).
- **Saves.** On RF the idle keep-alives fall from 69,120 a day, both ways, to 2,880. Measured
  under the lean stack, the 10 s lease's keep-alives were 1.15 MB of W1's 1.43 MB a day. At the
  same 17 B each, 2,880 are about 49 KB. At 5 % loss, a link expects 0.36 false closes a day
  (#988's design, §1.4).
- **Know first.** The lease, the keep-alives and `wait_before_close` are node-global: they also
  govern the node's loopback links (#988's design, risk 1). Neither end negotiates them, so both
  ends move at once.

### Step 2. Build on zenkey 0.20

- **Changes.** In the workspace `Cargo.toml`:
  - `zenkey` and `zenkey-build` move from `0.11` to zk2's 0.20, by git until #929 publishes them
    (§5). Add `zenkey-model` for the `zk2` contract tool in CI.
  - `[patch.crates-io]` gains `zenoh-ext` at `zm-1.10.2-r1`. zenkey takes `zenoh-ext` under
    `unstable`, and crates.io's `zenoh-ext` pins its `zenoh` exactly. #1011's readiness run
    patched it so.
  - modem-driver's tests that use v1's `V1Context` and `parse_slice` (`auth_face.rs`,
    `honest_absence.rs`, `telemetry_readings.rs` and the plane tests) move with step 4.
- **Uses.** Nothing yet. To move one crate at a time, v1's crate can stay under a renamed
  dependency, `zenkey-v1 = { package = "zenkey", version = "0.11" }`, as zengui's port did.
- **Check.** `cargo tree -i zenoh` shows one zenoh, the fork's. zenkey's own suite passes on the
  fork (#1011's run, with #1020's fix).
- **Saves.** Nothing.

### Step 3. `modem-contract` becomes a zk2 contract crate

- **Changes,** as [`contract-crate.md`](contract-crate.md) §2 lays it out:
  - `contract/modem.v3.toml` is this directory's [`modem.v3.toml`](modem.v3.toml);
  - `schemas/modem.json` is schemars' output from `src/documents.rs`, committed;
  - the packed files are hand-written: [`schemas/modem-frames.json`](schemas/modem-frames.json)
    for 3.2's deprecated frames, and 3.3's `modem-readings.json` and `modem-states.json`;
  - `build.rs` runs zenkey-build 0.20, and `contracts/.history/` holds the published bundles;
  - `registry/` (`modem.toml` and `types.toml`), the three `.lock` files, `src/schema.rs`
    (`SchemaSet`, `verify_total`), `PROFILE` and `PRODUCER` go;
  - `src/config.rs` converts `modem_device::config` into `zenkey_model::config`'s types,
    `config.v1`'s, in place of v1's `zenkey::config`.
- **Uses.** `zenkey_build::Config::new().contracts_dir(…).history_dir(…)`, `.json_type("json:LinkState",
  "crate::documents::LinkState")` for each Rust-first type (README gap 19), `.contract_crate(true)`
  and `.generate()`. Then `zk2 contract lint`, `zk2 contract bundle --history` and
  `zk2 contract check-history`.
- **Check.**
  - `cargo run -p zenkey-model --bin zk2 -- contract lint contract/modem.v3.toml` exits 0.
  - `zenctl catalog check --catalog catalog.json --contracts contract/modem.v3.toml` finds the
    revision in the catalog.
  - `cargo tree -e normal --no-default-features -p modem-contract` holds no `zenoh`, which
    `scripts/guards.sh` keeps checking.
- **Saves.** Nothing on the wire by itself. `describe` and `introspect` leave the bus: an
  `introspect` read was about 50 KB (README gap 13).

### Step 4. modem-mgmt serves each device as a zk2 service

- **Changes,** in `modem-mgmt/src/lib.rs`, `Mgmt::start`:
  - `V1Context::for_producer(&PROFILE, PRODUCER)` gives way to one `zenkey::ServiceBuilder::new`
    per `Node`, on the one session. Its `ServiceConfig`'s address names the node as the service:
    literal, such as `h-3fa9c2d41b7e/rf0`, or asking `hostid.v1` to mint the system,
    `ServiceConfig::minted(<node's name>)`, which a config file spells
    `address = "@hostid.v1/rf0"`. That spelling is never a key. `ServiceBuilder::new` mints the
    system once per run, and every key carries it, `zk2/h-3fa9c2d41b7e/rf0/…`.
    (`ServiceBuilder::with_hostid` mints through another `HostIdMinter`, a test's.)
  - The generated `modem_v3::Server::declare` implements `modem.v3` on the builder. Then
    `health()`, `catalog(…)` with [`catalog.json`](catalog.json), `run_store(…)` with a
    `zenkey::run::FileRunStore`, and `start()`.
  - An optional resource whose gate the device does not hold is not exposed
    (`ServiceConfig::capabilities`), or is listed with `ServiceBuilder::unavailable` and its cause.
    That replaces `sensor::document`.
  - `rpc::introspect` and `rpc::describe` go: the descriptor and the bundle replace them.
    `rpc::lanes` and `rpc::action_run` become the generated handlers of `@op/sdu/lanes` and
    `@op/action/{name}`. Their answers go out in the value and error forms (core 0.94 §5.1).
    zenoh-modem's own callers move to the generated `Client`, which writes the request form. An
    owner still reads a bare JSON request, until C4d removes the bare wire (§6).
  - `presence_loop`, which cycles a netdev's token when its ifindex moves, becomes
    `Service::new_epoch`, which restarts a `config.v1` or `config.v2` configuration (#1055, #1064).
  - `twin_alive` has no runtime twin. The README's "Tokens" maps it to a liveliness query on the
    instance key before `start`, and the doctor's `hostid-duplicate` sees two instance tokens.
- **Uses.** `ServiceConfig::minted`, `ServiceBuilder::{new, implement, health, catalog, run_store,
  unavailable, start}`, the generated `Server` and `Client`, `Service::new_epoch`.
- **Check,** on the host bus:
  - `zenctl service list` and `zenctl service show h-3fa9c2d41b7e/rf0`;
  - `zenctl check conform h-3fa9c2d41b7e/rf0 modem.v3 --contracts …`, which replaces
    `modem_mgmt::conformance::Wire` (contract-crate.md §3's `conformance-wire` keeps it for
    out-of-tree daemons);
  - `zenctl doctor`, `catalog-missing` among its checks;
  - `zenctl hostid --v1-salt zenoh-modem-host-id-v1` on each host, for the system that replaces
    each v1 origin (README gap 22).
- **Saves.** On the host bus, v1's `sensor`, `describe` and `introspect` documents. Nothing
  changes across the face before step 8. Presence never crosses: it would cost 277 B per device
  per reconnect ([`link.md`](link.md) §5).

### Step 5. Telemetry crosses as frames

- **Changes,** in `modem-mgmt/src/telemetry.rs`, `publish_loop`:
  - The 73 JSON streams stay at `DEFAULT_INTERVAL`, 5 s, on the host bus, through the generated
    writers in place of `READINGS`' hand-declared publishers.
  - Each tick, the same `Snapshot` also fills one record per frame resource the device's face
    lists. `None` is absent, and so is a reading the device lacks. A counter goes in unwrapped.
  - A frame the face does not list is left unpublished, and listed unavailable, cause `config`.
    The cadence and the list are the device's configuration: 3.3's gate is a `[[device]]`'s
    `frames` table, `config:device.frames.<name>`.
  - Write 3.3's `frames/fast`, `frames/slow` and `frames/paid`. 3.2's, `frames/link`, seven
    group frames and `frames/message`, are deprecated: an owner lists them unavailable.
- **Uses.** The generated `Server`'s typed `zenkey::typed::FrameWriter<Packed<T>>`, its `record`
  and `flush`. The writer frames, keyframes and deadbands. On a contract that uses
  `telemetry.v2` it meters each counter, and refuses one below its last count within the run
  (`Error::Telemetry`).
- **Check.**
  - `zenctl telemetry 'zk2/h-3fa9c2d41b7e/rf0/**' --contracts … --catalog …` judges each field's
    series.
  - `zenctl watch` and `zenctl echo` show each frame's records and field states.
  - zengui's Inspector shows a frame's fields, and its Series plots them.
- **Saves,** on RF, a day:

  | What crosses | Bytes | Source |
  |---|---:|---|
  | Today: the four link streams, downsampled to one put a minute | 280.5 KB, measured | the baseline's "W1, a day" |
  | 3.2's `frames/link` alone, the same four readings | 64.8 KB | #981's design, §3 |
  | 3.2's eight frames on rf0's face, all 73 readings present | 401 KB | the README's "Frames" |
  | 3.3's `frames/fast`, all 73 present, 24 moving | 177 KB | the README's "Frames" |
  | 3.3's `frames/fast`, rf0's own 18 readings | 92 KB | the README's "Frames" |

  On LoRa, with all 73 present, 3.3 spends 376 s, 534 s and 785 s of the 864 s daily duty at
  SF7, SF9 and SF12. lora0's own readings spend 248 s, 281 s and 434 s. 3.2's nine frames would
  spend 736 s, 1,420 s and 2,081 s. On SBD, 3.3 sends one frame a paid message, 48 a day, each
  229 B with all 73 present.

### Step 6. States and events, confirmed by the heartbeat

- **Changes.**
  - `state::publish_loop` puts the link and endpoint documents every `ttl_s / 2`. So do the loops
    in `oper.rs`, `identity.rs`, `security.rs`, `radio.rs` and `destination.rs`. Each now puts
    on a change only, through the generated state writers.
  - When an endpoint's peer goes, `StateWriter::set_vouched(false)` clears its bit in the
    heartbeat (#978's design, §3).
  - With 3.3, each `packed/…` twin is put beside its JSON document, on the same change.
  - `events::publish_loop` and `publish_device_loop` write each occurrence on
    `…/events/<template>/<ulid>` through the event writers. The ULIDs from `clock.rs` keep their
    shape.
- **Uses.** The generated state and event writers (`ServiceBuilder::declare_state_writer`,
  `event_writer`). The runtime's heartbeat, `zk2/<system>/<service>/freshness.v2/stream/heartbeat/modem.v3`,
  vouches for each member every half its shortest horizon (`freshness.v2` §2.4).
- **Check.** `zenctl check conform h-3fa9c2d41b7e/rf0 modem.v3` asks the stamps, the GETs and
  freshness. `zenctl get state` reads a member.
- **Saves.**
  - On the host bus, no state is re-put every 30 s.
  - Across the face, nothing: 3.2 and 3.3 keep every state `host`, so no `modem.v3` heartbeat
    crosses. Exposing one later costs its heartbeat too: about 179 KB a day on RF at a 60 s
    horizon, 36 KB at 300 s (#978's design, §3).
  - 3.3's packed records are 8 to 21 B, against 111 to 585 B of JSON. A GET across a face by
    grant carries the record's size.

### Step 7. Configuration

**Now, on `config.v1`** (landed):
- **Changes.** In `modem-mgmt/src/config.rs` and `rpc.rs`, `ConfigState`, `Bucket`,
  `config::publish_loop`, `rpc::config_set`, `rpc::config_control` and `rpc::config_read` give
  way to `ServiceBuilder::config_v1(startup, device)` on each node with a configuration plane:
  - `Startup`'s `schema` comes from the backend's `Configure::schema`, through `modem-contract`'s
    `config.rs`, with each group's `writable` from the node's allowlist;
  - its `floor_s` is `Configure::confirm_floor`, and its `apply_after` is the device's, not v1's
    fixed 1 s (binding §10, finding 5);
  - its `persisted` is the overlay. The boot that reconciles the overlay stays modem-mgmt's
    (binding §8);
  - a `zenkey::config_v1::Device` per backend applies, undoes and persists over
    `modem_device::Configure`.
- **Check.** `zenctl config get h-c2a95e41d7f0/lora0 --status`, then `set`, `confirm`, `cancel`
  and `extend`. `zenctl check conform … config.v1` and the doctor's `config-*` checks judge it.
- **Saves.**
  - On the host bus, the view and the status are put on a change only, and `config.v1`'s own
    heartbeat confirms them. Their re-put every 28.5 s cost about 5 MB a day a device (#978's
    design, §3).
  - Across RF, a far caller GETs the compact status, two SDUs, in place of the 1,286 B view's
    six. Nothing pushes either (README gap 20).

**Next, on `config.v2`** (its text, its owner and `sx1262_config.v1` landed; its tools not):
- **Changes.**
  - One config contract per backend, written from its `Configure::schema`. `sx1262_config.v1`
    is the SX1262's.
  - Each lists `config.v2` and `freshness.v2` in `uses`. It declares its groups in
    `config.groups` on the state `config`, and its packed file holds the derived `Record`,
    `Set<T>`, `Control` and `Error`.
  - Each backend's `zenkey::config_v1::Device` from `config.v1` serves `config.v2` unchanged.
    The schema comes from the contract, not from `Startup`.
  - Tokens and times are the runtime's, and nothing is put on a timer (`config.v2-binding.md`
    §13).
- **Uses.**
  - `zk2 config derive <contract.toml>` prints the derived types for the packed file. zenkey-build
    then lints the contract and generates them.
  - The generated `Server` implements the contract. Then
    `ServiceBuilder::config_v2(zenkey::config_v2::Startup { … }, device)` serves `set/<group>`
    for each hot or reach group, `confirm`, `cancel` and `extend`, and `persist` with a persisted
    layer. It puts the record before any token.
  - `Startup::device_schema` takes the backend's runtime schema. The start is refused on any
    difference from the contract in a group, a class, a parameter, a kind, a bound or a
    sensitive mark (`config.v2` Appendix B). Descriptions, units and `writable` are not compared.
  - `sbd-medium`, `netdev-modem`, `dlep-modem` and `mm-modem` hold text parameters, and wait for
    #979's B3 on `config.v1`.
- **Check.** `zenkey/tests/profile_config_v2.rs` runs `scenarios-v2.md` §1 to §12 over the
  spec's `rfcfg.v1`, and the owner example's `--config-v2` drives `rfcfg.v1` by hand.
  `zenkey/tests/zenoh_modem_config.rs` runs the binding's §15 round trip on `sx1262_config.v1`
  with its bytes. `zenctl config` waits for G3.
- **Saves.** The far-side reach change at 2,400 bit/s, each step one 220 B frame (`config.v2`
  Appendix B; the push and the refusal from the binding, measured by its §15 round trip):

  | Step | `config.v1` | `config.v2` |
  |---|---|---|
  | `set/radio` | about 100 B | 16 B |
  | its reply, before the apply | 114 B | 50 B: the record, pending |
  | the result | a GET: the status, 2 frames, or the view, 6 to 8 | the record's push, 46 B, no GET |
  | `confirm`, then its reply | the view, 8 frames, whole 58 % of the time | 8 B, then 36 B |
  | a refusal | 60 to 90 B of JSON | 7 B |

  The `config.v1` column is its wire before C4c. Since core 0.94 its replies go out in the value
  form and its refusals in the error form, which this table does not model.

### Step 8. Each node's cutover: the generated access control

- **Changes.** Each router's `access_control` comes from its policy in [`faces/`](faces/), in
  place of the v1 block generated from `registry/` with `--face constrained --link-interval 60`.
  The router and its node's daemon move together. One router per medium (§3.4).
- **Uses.**

  ```bash
  zenctl acl gen --enrollment examples/zk2/zenoh-modem/faces/zenoh-modem.enrollment.toml \
                 --contracts examples/zk2/zenoh-modem \
                 --contracts spec/profiles/config/config.v1.toml \
                 --contracts spec/profiles/health/health.v1.toml \
                 --catalog examples/zk2/zenoh-modem/catalog.json \
                 --face-policy examples/zk2/zenoh-modem/faces/zenohd-rf0.face.toml --json5
  ```

  The same line with `--far-config` in place of `--json5` writes the far end's transport.
- **Check.** `zenctl check face <router config> --face-policy … --enrollment … --contracts …
  --peer-config <far config> --catalog …` (§3.2). It reads the fork's files as data, so start
  each router once on the fork to see its own loader accept the file.
- **Saves.** v1 sent four puts of about 35 B a minute, in about 211 B of SDUs. One frame takes
  their place (step 5). `@zk` stays denied.

### Step 9. The ground side

- **Changes.** A ground reader binds statically to `h-3fa9c2d41b7e/rf0` (core R7). It decodes
  with `modem-contract`, default features off, and the catalog. It subscribes by exact key, waits
  the face's reply timeout, and declares a silent subscription again where the radio repairs
  nothing (§4).
- **Uses.** `zenkey_model::link::FacePolicy::far_side`, `Consumer::across`, `Client::across`,
  `FarSide::session_keys` for the session's transport. zenctl's `--across-face`, `--face-policy`,
  `--face`, `--catalog` and `--contracts`.
- **Check.** `zenctl echo --across-face h-3fa9c2d41b7e/rf0 …` prints each frame, and
  `zenctl telemetry … --across-face` judges its series (§4).
- **Saves.**
  - A put on a key the far side subscribed exactly rides a 1 B scope: 35 B in all, measured in
    W1. Under a far subscription to `zk2/**` a gauge's put was 85 B (W2, finding 2).
  - RF replies came 3.3 to 7.4 s after their GET, measured. At a 1 s or 5 s timeout they read as
    silence, and still crossed: 47 KB a day in W2 (findings 6 and 7).

## 3. The routers

### 3.1 The face policies

[`faces/`](faces/) holds one `link.v2` policy per router. Each names the zenoh it runs,
`zm-1.10.2-r1`, and the catalog the ground holds. The three LoRa policies are alternatives for
`lora0`'s one router, one per spreading factor. [`link.md`](link.md)'s first note gives each
plan.

| Policy | Node, service | Under `modem.v3` 3.2 (#988's L5) | Since 3.3 (#991) |
|---|---|---|---|
| [`zenohd-rf0`](faces/zenohd-rf0.face.toml) | `h-3fa9c2d41b7e`, `rf0` | `frames/link` and the seven group frames, every 60 s | `frames/fast`, every 60 s |
| [`zenohd-lora0-sf7`](faces/zenohd-lora0-sf7.face.toml) | `h-c2a95e41d7f0`, `lora0` | the same eight, every 60 s | `frames/fast`, every 60 s |
| [`zenohd-lora0-sf9`](faces/zenohd-lora0-sf9.face.toml) | `lora0` | `frames/link` every 60 s, the groups every 120 s | `frames/fast` every 120 s, full every 15 min |
| [`zenohd-lora0-sf12`](faces/zenohd-lora0-sf12.face.toml) | `lora0` | the eight every 900 s; `frames/link`'s horizon too short | `frames/slow` every 900 s, full hourly |
| [`zenohd-sat0`](faces/zenohd-sat0.face.toml) | `h-71c0e2a9d4b3`, `sat0` | `frames/message` every 1,800 s, no horizon | `frames/paid` every 1,800 s, a 3,600 s horizon |

Each plan denies `@zk`, every plain stream, every state and every event. RF and LoRa draw
`no_arq`, since the radio repairs no lost frame. Every face draws `frame_unlisted` for the frames
it leaves out. The other warnings differ:
- on 3.2, `keep_alive_not_suppressed` at SF7 and SF12, and at SF12 `interval_over_horizon` and
  `horizon_under_face` on `frames/link`;
- on 3.3, `keep_alive_not_suppressed` at SF7, and `frame_over_batch` at SF9, whose largest
  frame, 216 B, a delta of all 73 readings, is above its 117 B batch.

### 3.2 Generating and checking

- **`zenctl acl gen --face-policy F --json5`** writes the router's `access_control`, its
  `transport` and its `routing` blocks (`link.v2` §2.18). Step 8 gives the full line.
  - Given one `--catalog`, the plan refuses a crossing revision the catalog lacks
    (`catalog_missing`).
  - `--far-config` writes the far end's `transport` and `routing`, for the far router or a
    gateway client's session.
  - `--default-permission` is `deny` by default. zenoh-modem's routers run `allow` today, under
    which each grant compiles into denies of its complement (spec §11.2; README gaps 12 and 13).
- **`zenctl check face <ROUTER-CONFIG> --face-policy F --enrollment … --contracts …`** compares
  the file with the plan (`link.v2` §2.19). It reads what the file omits at the binding's
  defaults.
  - `transport_differs`: a key the router runs at another value. `wait_under_lease`: a
    `wait_before_close` below the lease, whatever the policy says.
  - `--peer-config <FAR>`: `peer_differs`, a warning, when the far end's lease or keep-alives
    differ.
  - `--catalog`: `catalog_mismatch`, when the ground's catalog is not the policy's.
- **The fork's binding** (#1044, PR #1056). On `zm-1.10.2-r1` the plan narrows other principals
  over the fork's link protocols, `unixsock-dgram` among them, which stock zenoh 1.10.1's loader
  refuses.
  - `check face` with its `--peer-config`, and `acl gen --check`, read such a file as data, in
    zenoh 1.10.1's shapes and the binding's link protocols. A key that does not read is named at
    its path, exit 2.
  - Each of zenoh-modem's five router blocks, as `acl gen` writes them, passes unedited in both
    postures.
  - No zenoh loads the file, so whether the fork's own loader accepts it is not checked. The
    coverage note says so, and the JSON carries `"read": "data"`.

### 3.3 The transport per medium

Keep-alives a day are 2 × `keep_alive` × 86,400 / lease, both ways (#988's design, §1.4). Every
policy runs `sequence = "16bit"`, and `wait_before_close` equals the lease.

| Router | bit/s | Shipped today | #988's policy | Since 3.3 (#991) | Keep-alives a day | Reply timeout | Batch |
|---|---:|---|---|---|---:|---:|---:|
| RF | 2,400 | 10 s / 4 (`configs/zenohd-rf0.json5`) | 240 s / 4 | the same | 69,120 → 2,880 | 15 s | 222 |
| LoRa SF7 | 5,469 | none | 120 s / 4 | the same | 5,760 | 15 s | 224 |
| LoRa SF9 | 1,758 | none | 240 s / 4 | 480 s / 4 | 2,880, or 1,440 | 15 s | 117 |
| LoRa SF12 | 293 | none | 1,080 s / 4, timeouts 40 s | 3,600 s / 4 | 640, or 192 | 120 s | 224 |
| SBD | 2,400 | 3,600 s / 2 (`configs/zenohd-sat0.json5`) | the same | the same | 96 | 1,000 s | 271 |

- **Why 4 keep-alives.** At 5 % loss, a keep-alive count of 2 closed the link at every lost
  keep-alive, 9 of 9 LoRa runs, and each reopen cost more air than it saved (#988's design,
  §1.4).
- **Why #991 moved SF9 and SF12.** At four frame intervals a lease, the frames replace
  every uplink keep-alive. SF12's 288 uplink keep-alives a day cost 427 s of air, half its duty.

### 3.4 What the policies assume

- **One router per medium.** zenoh reads the transport once per router (`link.v2` §2.16), so
  RF's 240 s lease and SBD's 3,600 s cannot share one. A node with two radios stays open
  (README gap 10).
- **The attachment.** Every policy says `attach = "client"`: the ground attaches as a client of
  the device's router (`link.v1` §2.5). zenoh-modem's shipped RF routers are `mode: "peer"` at
  both ends of the radio, and sat0's router is itself a client of the ground's. Each medium's
  shape is zenoh-modem's to settle against §2.5 before the generated blocks replace its configs
  (§5).
- **The enrollment** ([`faces/zenoh-modem.enrollment.toml`](faces/zenoh-modem.enrollment.toml))
  gives each device service `modem.v3`, `config.v1` and `health.v1`. `config.v2`'s owner is on
  `main`, and `sx1262_config.v1` with it (G4): `lora0` implements it there in `config.v1`'s place.
- **`lora0`'s host id**, `h-c2a95e41d7f0`, is made up. LoRa has no shipped router config yet.
- **The compress layer** finds nothing in a packed payload, and its stored form costs a byte.
  #988's design asks zenoh-modem to turn it off on zk2c lanes (§1.8's tracker list).

## 4. The ground side

**What the ground holds.** It decodes with what it was given, since nothing of `@zk` crosses
(core R7, [`link.md`](link.md) §5):
- the catalog, the same file as the host's. A reader without the owner's catalog refuses to date
  a time field (core §8.6). `zenctl catalog check --catalog … --contracts …` checks it;
- the contracts: `modem-contract` with default features off, or `--contracts` for the tools;
- the face policy, for the timeout and the transport.

**zenctl across a face.**

| Verbs | Across a face they take | So that |
|---|---|---|
| `echo`, `rate`, `field`, `record`, `timeline`, `snapshot`, `watchdog` | `--across-face SYSTEM/SERVICE`, `--face-policy F --face NAME`, `--contracts`, `--catalog` | each resource is subscribed by its exact key, never `zk2/**`; the session runs the face's transport; `--timeout` defaults to the face's `reply_timeout_ms`; a silent subscription is declared again where `arq = false` |
| `telemetry` | `--across-face`, a bare flag: the selector names the one owner; `--contracts`, `--catalog`, `--settle` | the series are read from frames with their epoch in band; `--settle` should be the link's round trip, and `--for` should cover a full record, 900 s for 3.3's `frames/fast` on RF |
| `config get`, `set`, `confirm`, `cancel`, `extend`, `persist` | `--across-face`, a bare flag; `--status`; the global `--timeout` | a silence is unattributed; the status is two frames and the view six; `--timeout` should be the face's 15 s, since these verbs take no `--face-policy` |
| `catalog show`, `catalog check`, `schema show` | `--catalog`, `--contracts` | the ground checks what it holds, offline |

```bash
zenctl echo --across-face h-3fa9c2d41b7e/rf0 \
            --contracts examples/zk2/zenoh-modem \
            --catalog examples/zk2/zenoh-modem/catalog.json \
            --face-policy examples/zk2/zenoh-modem/faces/zenohd-rf0.face.toml --face rf

zenctl telemetry 'zk2/h-3fa9c2d41b7e/rf0/**' --across-face --for 900 --settle 8 \
                 --contracts examples/zk2/zenoh-modem \
                 --catalog examples/zk2/zenoh-modem/catalog.json
```

**zengui across a face.** zengui takes `--catalog` and `--contracts`. Its Inspector reads a
frame's fields, its Series plots them, and its Config tool drives `config.v1`. It has no
`--face-policy` and no across-face scope. Its scopes are wildcards, under which each put across
a face would carry its key's text (core §8.5). Across a face, give it `--selector` with each
frame's exact key, `--zenoh-config` with `acl gen --far-config`'s block, and `--timeout 15`.

**A ground service in code.** Read the policy with `zenkey_model::link::parse`, then take
`FacePolicy::far_side("rf")`. Open the session with `FarSide::session_keys()`, and read through
`Consumer::across(&far)` and `Client::across(&far)`. A ground service attached across the radio
as a client holds an instance token and interface tokens, which cross the radio at each
reconnect: about 1 KB for five interfaces (#982's design, §3). #982's `first_hop` configuration
would save them, and is missing.

## 5. What must be decided

**The maintainer's.**

| Decision | The options | The drafts' default | Where |
|---|---|---|---|
| The crates' path | a publish of the zk2 crates, or a git dependency on a tag | none. No zk2 tag exists, so a git dependency pins a revision until one is cut. #929's boxes (`zenkey-model` in the lane, READMEs, stable zenoh, metadata, what 0.20 promises, whether an adopter goes first) stand before a publish | #929 |
| Whether `modem.v3` lists `link.v2` | in a later revision, or a face policy counted as use | keep `link.v1`, as 3.3 does. Every resource 3.2 and 3.3 expose is already packed, so no plan draws `not_packed`. `link.v2` is draft in the profiles index since G4, whose `sx1262_config.v1` lists it | [`link.md`](link.md), its first note; #991 |
| Whether a reader may reopen its session for a lost `DeclareKeyExpr` | reopen after silence, or rely on link ARQ | none. Re-declaring repairs a lost subscriber, never a lost key expression: only a reopen or link ARQ does. A reopen re-sends every declaration, non-droppable, about 44 s for 1.6 KB at SF12 | `zenkey/src/redeclare.rs`; #988's design, risk 5 |
| #991's frame layout and LoRa leases | 3.3's three frames over one record of all 73 readings, or 3.2's nine per-group frames (#1048) at #983's cadences; SF9's lease at 480 s and SF12's at 3,600 s, or #988's 240 s and 1,080 s | 3.3's, both departing from #983's and #988's designs for the budget. 3.2's nine frames cost 401 KB a day on RF, and 164 % and 241 % of LoRa's duty at SF9 and SF12. A later revision can reverse either, but `.history` is append-only, so 3.3 stays published | PR #1059; #991 |

**zenoh-modem's.**

| Decision | What the drafts did, and what it costs |
|---|---|
| The real ranges and the gate names | 3.3 took each range from the code at `d8c62df` and marks the chosen bounds; its gates are `config:device.frames.<name>`. A range is the wire: once published, changing it within the major is breaking (core §9.8). That is why 3.3 deprecates 3.2's illustrative nine |
| `period_s`'s bound | `sx1262-medium`'s `Configure::schema` gives none; `sx1262_config.v1` says 86,400. Given that schema as `Startup::device_schema`, `ServiceBuilder::config_v2` refuses to start unless one of them moves |
| Link ARQ on RF and LoRa | `arq = false` draws `no_arq`. RF's requirement is 3 retries; a LoRa backend declares its module's ARQ (`architecture.md` §4.13). With ARQ, a face says `arq = true`, and readers stop re-declaring |
| The three link problems, for its tracker (#988's design, §1.8) | 1. W2's transport that never came back: the cause lies below zenoh's wire, and needs a reproduction. 2. Link ARQ, or declarations that are repaired. 3. Its ops tooling and modem-mgmt's own callers wait the face's `reply_timeout_ms` |
| The attachment per medium | the policies plan the ground as a client of the device's router; the shipped RF routers are peers, and sat0's is the ground's client (§3.4) |
| The posture | `allow`, as shipped, or the generator's `deny` (README gap 12's fail-closed shape) |
| One interface or a split | `modem_sdu.v1` and `modem_netdev.v1`, decided before `modem.v3` freezes (README gap 6) |
| The host id | `hostid.v1`'s one salt re-keys every system, so every far side that binds statically is provisioned again (README gap 22) |

## 6. What is not ready yet

A migration started today would hit each of these.

| What it hits | What to do meanwhile | Issue |
|---|---|---|
| **No published crates,** and no zk2 tag | A git dependency on a revision, or a path | #929 |
| **The readers' `wait_before_close` default** is zenoh 1.10.1's 5 s | Pass `--wait-before-close`, or `--face-policy` where a verb takes it | #1011 |
| **The bare wire is still read.** An owner on `main` takes a bare request, and C4d removes it | Write every call through `zenkey::Client` or the generated `Client`, which write the request form; give zenoh-modem's ops tooling no bare request to depend on | #985 (C4d) |
| **`config.v2` has no tools.** Four backends need text | Stay on `config.v1`. Across RF, a GET of the status by grant; the view is never pushed | #985 (G3, G4), #979 |
| **No snapshots.** A seed or a re-read across RF is a reply per key, about 120 B each before its value. A 64-endpoint re-read is 14,788 B in 129 SDUs | Keep states `host`, as 3.2 and 3.3 do | #977 |
| **No first-hop configuration** | Nothing for zenoh-modem's own owners, whose sessions are TCP on the node. A ground service across the radio pays its tokens at each reconnect | #982 |
| **Event keys are ULIDs.** An event cannot cross on its template's prefix | Keep the four events `host` | #980 (C2) |
| **No text in packed records** | Text stays in the JSON states, and in `config.v1` | #979 (B3) |
| **zengui has no face scope,** and `zenctl telemetry` and `config` take no `--face-policy` | Set `--selector`, `--zenoh-config`, `--timeout` and `--settle` by hand (§4) | none filed |
| **The transport that never came back** (W2, finding 5) | zenoh-modem's tracker. Keep nothing `block` larger than a batch on a crossing key | #988's design, §1.8 |
| **The bytes are modelled.** zk2c's run of W1 and W2 on the fake media has not happened | Read every unmarked figure here as a model | #989 |
