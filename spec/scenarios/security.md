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
