# tcgui, the zk2 pilot (#589)

tcgui is a privileged netem backend, one per host, with an unprivileged
Iced frontend bound to the backends on several hosts. Its v1 surface is
`tcgui-shared/registry/tc.toml` (registry 1.3: 11 subjects, 8 procedures, 1
blob tier), mapped here onto zk2.

## Shape

| zk2 concept | tcgui |
|---|---|
| `system` | the backend host's id, `h-<12hex>`, minted by `hostid.v1` (v1's derivation, unchanged) |
| `service` | `tc`, one per host |
| interfaces | `tc.netif.v1`, `tc.netem.v1`, `tc.scenario.v1`, `health.v1` (from [`../walkthrough/`](../walkthrough/health.v1.toml)) |
| the frontend | a pure consumer on the operator's workstation: an instance token, no interface token. It binds each interface as a role to `*/tc`, every system's tc service, so the wildcard sits at the system position ([`frontend.bindings.toml`](frontend.bindings.toml)) |
| ownership | every resource `exclusive`, every write `fanout = "forbidden"` except `diagnostics`. The backend mutates kernel state, so it also keeps an out-of-bus lock (r3 §3.8). |
| schema kind | **jsonschema**, deliberately. tcgui's types are serde structs, which makes this the pilot of the Rust-first path: at port time schemars generates `schemas/tc.json` and the generated file is committed (#614). The walkthrough contracts exercise protobuf. |

Example keys, for host `h-3fa9c2d41b7e`, namespace `default`, interface `eth0`:

```text
zk2/h-3fa9c2d41b7e/tc/tc.netem.v1/state/config/default/eth0
zk2/h-3fa9c2d41b7e/tc/tc.netem.v1/@op/config/default/eth0/set
zk2/h-3fa9c2d41b7e/tc/tc.netif.v1/stream/bandwidth/default/eth0
zk2/h-3fa9c2d41b7e/tc/tc.netem.v1/stream/applied/01jgxqz4yqk8v6txw3m9f2a7cd
zk2/*/tc/tc.netif.v1/@op/diagnostics                 (fan-out, target All)
zk2/h-3fa9c2d41b7e/tc/@zk/alive/tc.netem.v1/<instance>/<fp16>
```

## Every v1 entry, mapped

| v1 (`tc.toml`) | zk2 |
|---|---|
| state `health` (BackendHealthStatus) | `health.v1` `status`. `host_id`, `backend_name` → the descriptor's metadata. `namespace_count`/`interface_count` are dropped (derivable from the interface states). |
| state `sensor` (SensorDoc: name, version, namespaces) | name and version → the descriptor; `namespaces` → `tc.netif.v1` state `namespaces` |
| state `interface/{ns}/{iface}` | `tc.netif.v1` state `interfaces/{ns}/{iface}` |
| telemetry `bandwidth/{ns}/{iface}` | `tc.netif.v1` stream `bandwidth/{ns}/{iface}` |
| proc `interface/{ns}/{iface}/set` | `tc.netif.v1` op `interfaces/{ns}/{iface}/set` |
| proc `diagnostics` (fan-out allowed) | `tc.netif.v1` op `diagnostics`, `fanout = "allowed"`, `idempotent` |
| state `config/{ns}/{iface}` | `tc.netem.v1` state `config/{ns}/{iface}` |
| telemetry `qdisc/{ns}/{iface}` | `tc.netem.v1` stream `qdisc/{ns}/{iface}` |
| state `plug/{ns}/{iface}` | `tc.netem.v1` state `plug/{ns}/{iface}` |
| events `applied/{ulid}` | `tc.netem.v1` **occurrence-keyed stream** `applied/{occurrence}`, reliable, `retention = "7d"` |
| proc `config/{ns}/{iface}/set` | `tc.netem.v1` op `config/{ns}/{iface}/set` |
| proc `plug/{ns}/{iface}/set` | `tc.netem.v1` op `plug/{ns}/{iface}/set` |
| state `scenario/{id}` | `tc.scenario.v1` state `scenarios/{id}`. A wildcard GET lists the library. |
| state `preset/{id}` | `tc.scenario.v1` state `presets/{id}` |
| state `execution/{ns}/{iface}` | `tc.scenario.v1` state `execution/{ns}/{iface}` |
| proc `scenario/set` (Add/Remove/List/Get/Update) | Add/Update → op `scenarios/put`; Remove → op `scenarios/{id}/remove`; **List/Get → state reads**, not operations |
| proc `execution/{ns}/{iface}/set` (Start/Stop/Pause/Resume/Status/ListActive) | Start/Stop/Pause/Resume → op `execution/{ns}/{iface}/control`; **Status/ListActive → state reads** of `execution/*/*` |
| proc `introspect` | **deleted**: presence + descriptor + contract bundles (r3 §3.10) |
| proc `describe` | **deleted**: the schemas ride in the contract bundle |
| `[[blob]]` artifact (planned) | **out of the pilot**. `blob.v1` when the support bundles exist. |

Nothing in `tc.toml` is left unmapped.

## Decisions recorded here

- **The CRUD and status procedures are split.** Reads become state; only
  mutations stay operations. Under v1 a `List`/`Status` request rode a
  *write* procedure, so a read could not be granted without granting the
  write. In zk2 reading is a GET on `state/…`, and the operations are
  separately grantable.
- **Three interfaces, not one.** The split follows what a frontend or
  another tool would bind separately, and it exercises a multi-interface
  service: one instance token plus one interface token per interface.

## Gaps found

1. **Request types repeat the template parameters.** `TcRequest`,
   `TcPlugRequest` and `InterfaceControlRequest` carry `namespace` and
   `interface`, which zk2 already carries as `{ns}/{iface}` in the key. A
   server would then have to check that they agree.
   - *Proposal:* the spec says operation request types **SHOULD NOT**
     repeat template parameters, and `zenkey-model` lints a request field
     named like a parameter. At port time the fields go.
2. **`freshness.ttl_s` repeats on every state resource.**
   - *Proposal:* allow interface-level `annotations` in `[interface]` as
     defaults, which a resource may override. This is small, and it is in
     the fingerprint either way.
3. **A pure consumer declares its requirements nowhere portable.** The
   frontend implements no interface, so it has no contract to put
   `[requires]` in. The runtime records its bindings in the descriptor (R3),
   and the binding configuration carries the roles. That is enough for the
   graph, but no file states "this component needs `tc.netem.v1`".
   - *Proposal:* keep it that way, since the requirement is code plus
     deployment, not contract. Revisit if codegen needs a manifest (#611).
4. **Identity fields are duplicated in payloads.** `backend_name` and
   `host_id` in `BandwidthUpdate`, `TcConfigUpdate`, `PlugState` and the
   health document duplicate the key and the descriptor.
   - *Not a zk2 gap:* a port-time simplification (#614), noted so the port
     drops them on purpose.
