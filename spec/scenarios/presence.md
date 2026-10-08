# Presence scenarios (core §1.5, §3.3, §8)

## §1 Bring-up order and tokens (§8.1, §8.2)

**Setup.** An owner implementing `nav.v2` (one required state, one
operation), and a pure consumer.

**Steps.** A tool subscribes to `zk2/*/*/@zk/**` with history, and calls the
owner's operation the moment its interface token appears.

**Expected.**
- The call succeeds: alive ⇒ callable.
- A GET of the descriptor, and of the contract bundle by its key, made the
  moment the instance token appears, both answer.
- The owner holds an instance token and one `alive/nav.v2/…` token.
- The pure consumer holds an instance token only.
- An instance exposing nothing of an interface holds no interface token for
  it.

## §2 The descriptor (§3.3)

**Steps.**
1. GET the owner's instance key.
2. The owner loses a capability that gates an optional resource.
3. An owner is started without one of its contract's required resources.

**Expected.**
1. A descriptor that validates against `descriptor.schema.json`.
2. A new descriptor is put on the instance key. The gated resource's
   absence is implied by the missing capability, so it is not listed in
   `unavailable`. A GET returns the new descriptor.
3. The owner does not start: no instance token appears.

## §3 Epochs and re-minting (§1.5, §8.1)

**Steps.**
1. The owner's counters reset without a restart.
2. A member of a template with `epoch` loses continuity.

**Expected.**
1. The owner declares a new instance token, interface tokens and descriptor,
   then undeclares the old ones. A watcher sees 1 or 2 live instance tokens
   throughout, never 0. Consumers treat the instance change as the counter
   discontinuity.
2. That member's token cycles. The others are untouched.

*Spike S2: one re-mint per second over 10k tokens cost 0.7 KiB/s; the
re-minted service always had 1 or 2 live instance tokens.*

## §4 Reading presence at scale (§8.1)

**Setup.** 10,000 tokens.

**Steps.** A session that holds a liveliness subscriber issues a liveliness
GET.

**Expected.** With a callback handler, the GET completes with every token.
With zenoh's default 256-slot handler, it hangs (zenoh#2678), which is why
the rule forbids it.

*Spike S2: hung at every measured size from 996 tokens.*
