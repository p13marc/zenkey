# Deployment shapes (#604, spike S8 on paper)

**The question (U6, r3 §3.5).** Does the one mandatory `system` chunk, plus a
one-chunk `service`, give every real deployment natural names, without
fakes?

**The answer: yes, for all nine shapes below.** One convention carries most of
the weight: a **dotted service name** names a sub-entity (`snmp.router01`,
`sysinfo.c2`). The v1 charset already allows it, and selection by
interface keeps working.

| # | Shape | `system` | `service` | Natural? | Notes |
|---|---|---|---|---|---|
| 1 | **tcgui** (the pilot) | backend host id, `h-<12hex>` (`hostid.v1`) | `tc`; the frontend is `tcgui-frontend` on its own workstation's host id | yes | The frontend binds `*/tc`. |
| 2 | **A single robot** | `rover`, or its host id | `navigation`, `imu0`, `thruster-l`, … | yes | One constant chunk, the cost r3 accepted. |
| 3 | **A vehicle plus a ground segment** | `vehicle-01` … `vehicle-NN`, `ground` | per system | yes | Durable commands are the author's state keyed by target: `zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01`. |
| 4 | **A fleet-global service** (mission manager, cloud) | `fleet` (or `cloud`) | `mission`, `catalog`, … | yes, by convention | `fleet` is a logical ownership unit, not a host. It replaces v1's `@catalog` service origin with an ordinary system. |
| 5 | **A simulator run** | the *real* system names, under a `sim-42` session namespace | as in production | yes | Consumers' bindings are unchanged. When simulation and reality must share one bus without namespaces, use `sim-vehicle-01`, a different system. |
| 6 | **A multi-device driver** | the host's system | one service per device: `imu0`, `imu1` | yes | Device-as-service. The template variant (`devices/{device}/…` in one service) stays for large dynamic populations (U18). |
| 7 | **ZenSight** | host id | `sysinfo`, `netlink`, …; proxied devices `snmp.router01`, `modbus.plc3`; singletons on `fleet`: `zk2/fleet/catalog`, `zk2/fleet/desired` | yes | Mapped in #622. |
| 8 | **zenoh-modem** | the node's host id | one per device: `rf0`, `sat0`, `wwan0` | yes | Mapped in #623. |
| 9 | **A vehicle with several computers** | `vehicle-01` | logical services (`navigation`), plus per-computer host services `sysinfo.c1`, `sysinfo.c2` | yes | "All sysinfo" is the interface chunk (`zk2/*/*/sysinfo.v1/…`). One computer's host services are `zk2/vehicle-01/sysinfo.c2/**`. |

## What is deliberately *not* key-selectable

**"Everything hosted by computer c2"** in shape 9 cannot be selected by key.
That set includes the logical services that happen to run there, and a
logical service moves between computers without changing its key. This is
r3's deployment independence, by design. The descriptor's `host` metadata
answers the question by introspection.

## Proposals for the spec (#606)

- **Dotted service names are a documented convention** for sub-entities: a
  device served by a driver or poller, or a host-scoped service inside a
  multi-computer system. The grammar needs no change.
- **`fleet` is a recommended name**, not a reserved token, for the logical
  system of deployment-wide singletons.
- **No change to U6.** The single mandatory chunk holds in every shape
  checked here; the running spike S8 will confirm it on the live harness.
