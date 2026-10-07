# The zk2 spike (#591)

Throwaway code whose output is measured numbers (r3 §7). It lives on branch
`zk2-spike`, has its own `[workspace]`, and is **never merged**. Its results
and `docs/zk2/spike-report.md` reach `main` through docs PRs. The branch is
tagged `zk2-spike-final` when #605 closes.

## Layout

| Path | What it is |
|---|---|
| `rt/` (`zk2rt`) | A throwaway runtime over `zenkey-model` (`../zenkey-model`, by path). It has mock services brought up in r3 §3.10's order, S1–S3 state (producer timestamps, a tombstone window, `reply_del`), `complete` operation queryables, the compact descriptor, contract serving at the location-free key, the measuring client (contract retrieval by hash with `BestMatching` then `All`, state GETs with `All` + `Latest`, calls with consolidation `None`), and collection (procfs RSS/CPU, a counting allocator, CSV rows). |
| `harness/` (`spike`) | Routers, mock services from contract files or synthetic ones, and one subcommand per spike group. |
| `justfile` | `spike-<group>` recipes. Each writes `results/<group>/*.csv` and a `summary.md`. |
| `results/` | Raw data, committed on this branch. |

## Running

```bash
just -f spike/justfile spike-smoke          # from the repository root
```

- **zenoh is pinned to 1.10.1** (the latest release, checked 2026-10-07).
- **Routers** are this harness's own binary (`spike router`): zenoh 1.10.1 sessions in router mode, so every process runs the same version. The `zenohd` on the PATH is 1.10.0 and is not used. A spike that needs plugins (the storage manager for S5, #601) records which `zenohd` it built.
- **Endpoints** are free ports picked at run time, never fixed ones, so parallel runs do not collide.
- **Release builds only:** numbers from a debug build are not numbers. The build tree is large and the disk is shared, so `just -f spike/justfile clean` reclaims it.

## What each piece covers so far

| #591 asks for | State |
|---|---|
| Throwaway runtime layer (state S1–S3, `complete` operations, tokens, descriptor, contract serving, a consumer) | Done in `zk2rt`. The consumer reads by GET and presence. Bindings arrive with S10 (#593). |
| Contracts it runs on | Every walkthrough contract and the tcgui pilot's (`justfile`'s `contracts`). Mock values for any type come from `zk2rt::mock`: JSON Schema minimal instances, default protobuf messages, raw placeholders. |
| A tc mock service from the pilot contracts | The generic mock serves `tc.netif.v1`, `tc.netem.v1` and `tc.scenario.v1` with no kernel access. |
| Load generator (N services × M interfaces × K resources) | `spike services --count N --synthetic M:K`. |
| Topologies | Router chains (`spike router --connect`), clients or peers (`--mode`), and the namespace variant (`--namespace`). |
| Link emulation with netem, through tcgui | Not yet. It needs root and network namespaces; S3 (#599) builds it. |
| Collection | RSS and CPU from procfs, latency from monotonic clocks, HLC stamps on every state sample, allocations from the counting allocator. Wire bytes (admin space or capture) come with S2/S3. |
| Recipes, CSV and summaries | `spike-smoke`. Each spike adds its own recipe. |

## spike-smoke

Two routers (A ← B), N mock services behind B (one session each, client
mode), and one client on A. The client:
1. waits until a liveliness GET sees every instance token;
2. parses every instance and interface token with `zenkey-model`;
3. reads svc-0's descriptor and fetches its contract by hash, verifying the bundle;
4. does a state GET (requiring a value and a producer timestamp) and a wildcard state GET;
5. makes one call;
6. writes a row to `results/smoke/smoke.csv`.

It fails unless every step holds.
