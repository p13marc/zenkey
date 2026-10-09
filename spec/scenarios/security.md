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

*Spike S14: 13 of 13 checks.*

## §2 Grants under `allow`

**Steps.** The same principals, with each grant compiled into denies of its
complement.

**Expected.**
- Every unauthorized action is blocked, except a put on a wildcard key,
  which R6 discards at the consumer.
- Allow rules alone, under `allow`, block nothing.

*Spike S14: confirmed live.*

**Generator check.** Remove a consumer's wildcard selector from the
provider's egress grant: the fan-in GET gets 0 replies. Restore it, and
remove the matching ingress `reply` grant: a refusal never reaches the
caller.

## §3 Who answers the admin space (core §4.2, §11.1)

**Setup.** One router R1, an owner, a tool, and a client `S` that is no
router. The tool reads S4's two selectors (core §4.2).

**Steps.**
1. With R1's admin space off, `S` declares a queryable on
   `@/<R1's zid>/router` and answers `{"plugins": null}`. The tool reads
   `@/*/router` and runs S4's check.
2. The same, with R1's admin space on, read-only.
3. Under each posture, generate R1's grants (core §11.1) and repeat step 1.

**Expected.**
1. `S`'s answer arrives, on R1's own key, and it is the only answer. Its
   replier id is `S`'s zid, not R1's. The tool holds the answer
   unverified, and the check stays unobservable, never clean.
2. R1 answers too, under its own replier id, and that answer is verified.
   `S`'s is still unverified, so the check is not clean either.
3. R1 refuses `S`'s queryable. No principal's answer arrives, and only a
   router's remains.

*Measured (0.12) on zenoh 1.10.1, steps 1 and 2: the reference runtime's
`admin_spoof.rs` and the fleet's doctor test. Step 3 waits for the grant
generator (#612, FJ7).*
