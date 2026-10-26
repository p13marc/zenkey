# Security scenarios (core §11)

Common setup: one router with usrpwd principals: a commander, an actuator,
an autopilot, a frontend and two backends. Grants are generated from
contracts and bindings.

## §1 Grants under `default_permission: deny`

**Expected:**
- A commander's put on its own command key reaches the actuator.
- A put on another principal's key is blocked, and so is a queryable or
  token declared there.
- A service using advanced publication declares its `@adv` queryable and
  token under its Own grant, and a consumer with history reads them.
- A subscription the bindings do not name is blocked.
- A frontend's fan-in GET over `zk2/*/tc/…` gets one reply per backend.
- **Presence within the grants** (0.8). A principal with Call on a
  backend's operation reads that backend's instance and interface tokens.
  With its liveliness reads removed from its grant, the same read is
  answered complete and empty (presence.md §6), which is why the grant
  holds them (core §11.1). *Added in 0.8, after S14; not yet run against a
  generator.*
- A contract fetch works for any principal.
- **Inspection** (0.27). A tool that inspects `*/tc`, binding and calling
  nothing, reads every backend's tokens and descriptor on the wildcard
  selectors a doctor sends (`zk2/*/tc/@zk/**`, `zk2/*/tc/@zk/instance/*`),
  complete. It reads none of their data, and no presence it does not
  inspect.
- **One principal, both roles** (0.27). A principal that runs a service
  and a tool holds Own on its service's prefix, and the tool's presence
  and admin read beside it, and no write beyond its own prefix.

*Spike S14: 13 of 13 checks. Inspection: run against the reference
generator (`zenkey-fleet/tests/acl_live.rs`); both roles: the reference
generator's plans (`acl.rs`, `tests/fixtures/acl/`).*

## §2 Grants under `allow`

**Steps.** The same principals, with each grant compiled into denies of its
complement.

**Expected.**
- Every unauthorized action is blocked, with two exceptions:
  - a put on a wildcard key, which R6 discards at the consumer;
  - where the generator leaves a principal's own keys out of its
    complement (core §11.2, 0.31), that principal's reads and deliveries
    of its own services' keys.
- Allow rules alone, under `allow`, block nothing.

*Spike S14: confirmed live.*

**Generator check.** Remove a consumer's wildcard selector from the
provider's egress grant: the fan-in GET gets 0 replies. Restore it, and
remove the matching ingress `reply` grant: a refusal never reaches the
caller. Remove a wildcard presence selector from a provider's egress
grant (0.27): an inspecting tool's wildcard descriptor read gets nothing
from it, while its liveliness read, which the router answers, still holds
the provider's tokens.

## §3 Who answers the admin space (core §4.2, §11.1)

**Setup.** One router R1, an owner, a tool, and a client `S` that is no
router. The tool reads S4's two selectors (core §4.2).

**Steps.**
1. With R1's admin space off, `S` declares a queryable on
   `@/<R1's zid>/router` and answers `{"plugins": null}`. The tool reads
   `@/*/router` and runs S4's check.
2. The same, with R1's admin space on, read-only.
3. Under each posture, generate R1's grants (core §11.1) and repeat step 1
   twice: once with `S` an enrolled principal, and once with `S` an
   authenticated session that is no principal.
4. **A far router** (0.13). A router R2 links to R1, both admin spaces on,
   the tool still a client of R1. Then `S` answers `@/<S's zid>/router`
   under its own replier id.

**Expected.**
1. `S`'s answer arrives, on R1's own key, and it is the only answer. Its
   replier id is `S`'s zid, not R1's. The tool holds the answer
   unverified, and the check stays unobservable, never clean.
2. R1 answers too, under its own replier id, and that answer is verified.
   `S`'s is still unverified, so the check is not clean either.
3. An enrolled principal's queryable is refused under both postures. A
   session that is no principal is refused under `deny`. Under `allow` it
   matches no subject and gets everything (core §11.3), so its answer
   arrives, and only step 1's rule (the replier id) keeps the check from
   clean. (0.12 wrote that R1 refuses `S` under each posture, which
   `allow` cannot do: F-88, fixed in 0.15.)
4. R1's answer lists R2 as a `router` session, so R2's answer, under its
   own replier id, is verified, and the check covers both routers. R1
   lists `S` as a `client`, so `S`'s answer stays unverified, and the
   check is not clean.

*Measured on zenoh 1.10.1:
- steps 1 and 2 (0.12) and step 4 (0.13), by the reference runtime's
  `admin_spoof.rs` and the fleet's doctor tests;
- step 3, for an enrolled principal under `allow`, by the reference
  generator's live test (#612, FJ7);
- step 3, in both cases under both postures, by zk2py's generator, built
  from §11 alone (0.15).*

## §4 The admin read admits no GET of a node's declaration records (core §11.1, §11.2, 0.50)

**Setup.** One router R1, its admin space on, read permitted (core §4.2),
under `default_permission: deny`, with the block a generator compiles for
two principals: a tool that reads the admin space and inspects every
service (core §11.1 Tool), and a service principal that does neither.
R1's storage manager runs one storage, `plans`. A session that is no
principal, R1's own, holds a queryable on `@/**` that answers nothing and
records the selector of every GET R1 routes to it. Nothing declares while
a step runs: every session is open and every queryable declared before
step 1.

**Steps.**
1. The tool reads `@/*/router`,
   `@/*/router/status/plugins/storage_manager/storages/**` and `@/*/*`.
2. The tool GETs `@/*/router/queryable/**`, `@/*/router/**` and `@/**`.
3. The service principal reads step 1's three selectors.
4. **How a query is admitted.** Compile the block again with the tool's
   admin read replaced by 0.15's, `@/*/router` and `@/*/router/**`, and
   repeat step 1's `@/*/*`, then its `@/*/router`.

**Expected.**
1. R1 answers each: its document, `plans` with its `key_expr`, and its
   root document. The recorder hears each selector.
2. Each GET ends complete, with nothing of R1's, and the recorder hears
   none of them: R1 refused each at the tool's face, before
   routing it, so none reached the admin space. `@/**` also reaches the
   tool's own session, whose runtime answers `@/<its zid>/session/**`
   itself.
3. Nothing answers, and the recorder hears none of them. Under `deny`, no
   grant but the admin read reaches the admin space.
4. `@/*/*` is refused, although both of 0.15's keys intersect it, and the
   recorder does not hear it; `@/*/router` is answered. A query is admitted
   by inclusion in an allowed key, never by intersection.

*Measured on zenoh 1.10.1:
- steps 1–4 by the reference generator's live tests (`zenkey-fleet/tests/
  acl_live.rs`, `s4_the_admin_read_reaches_no_declaration_record` and
  `a_query_is_admitted_by_inclusion_never_by_intersection`). The suite
  never asks a declaration record beside a writer;
- beside one, with zenoh alone (`docs/zk2/upstream/adminspace-reply-repro/`,
  "Under access control"): beside 500 records of each kind and a client
  declaring flat out, the tool's GETs of a router's records, its
  `linkstate/*`, `@/*/router/**` and 0.11's storages selector froze R1 in
  45 of 45 trials under 0.15's admin read, 43 at the first GET, and in 0
  of 70 under 0.50's, `@/**` and each kind of writer alone included,
  where each of 6.9 million GETs was refused. The three reads of step 1
  froze nothing under either (0 of 30), and 0.15's refused `@/*/*`.*
