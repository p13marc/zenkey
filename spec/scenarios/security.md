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
