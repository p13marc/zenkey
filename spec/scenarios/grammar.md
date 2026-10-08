# Grammar scenarios (core §1)

## §1 The guard

**Setup.** One router. An owner `zk2/g/svc` with a plain `stream`, an
`@stream`, a `state`, an `@state`, an `events` resource, an `@op` operation,
and its instance token.

**Steps.**
1. A consumer subscribes to `zk2/g/**`. The owner puts on every data
   resource.
2. The consumer GETs `zk2/g/**`.
3. The consumer runs a liveliness GET on `zk2/g/**`, then on `zk2/g/*/@zk/**`.

**Expected.**
1. The consumer receives the `stream`, `state` and `events` samples only.
2. The GET reaches the state queryable and never the `@op` queryable.
3. The first liveliness GET returns no token; the second returns the
   instance token.

*Spike S1: passed, also under a namespace.*

## §2 Namespaces

**Setup.** An owner in session namespace `dep1`, with every kind token.
Consumers in namespace `dep1`, in no namespace, and in namespace `dep2`.

**Steps.** The owner puts on each resource and serves each queryable. Each
consumer subscribes to and GETs the base-relative key.

**Expected.**
- The `dep1` consumer sees base-relative keys (`zk2/…`).
- The consumer without a namespace sees `dep1/zk2/…`.
- The `dep2` consumer sees nothing.
- `dep1/zk2/**` reaches no verbatim token: the guard holds under a prefix.

*Spike S1: 16 of 16 cases held.*
