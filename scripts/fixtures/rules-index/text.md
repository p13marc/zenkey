# A text with known rules and roles

The self-test of `scripts/gen-rules-index.py` reads this text, and
`expect.json` lists the rules it must read: each one's line, roles, ids and
evidence. MUST and SHOULD are used as in RFC 2119 here, and only a rule
below states one.

## 0. Conventions

MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are used as in RFC 2119. Each
MUST cites what checks it, and a MUST is a rule. None of these is one.

## 1. Grammar

Every key MUST build back to the same string. `[F: keys.json]`

## 2. Owners and consumers

An owner MUST publish with the contract's QoS. It SHOULD do so in a new
session. `[Sc: types.md §3]`

A value with several forms SHOULD be canonicalized by the owner.

A caller or tool MUST retrieve a bundle as follows:
1. GET with target `BestMatching`. `[Sc: retrieval.md §1]`
2. Verify each reply. `[Sc: retrieval.md §2]`

`[Sc: retrieval.md §1–§3]`

- **The instance id is an epoch.** An owner MUST mint a new id at a
  restart.
  - It SHOULD do so in a new session.

  `[Sc: presence.md §3]`
- **The tokenless set.** A deployment MAY configure an owner with a
  tokenless set. These SHOULD be the framework's interfaces.

| # | Rule | Evidence |
|---|---|---|
| S1 | **Owner:** every put MUST carry a stamp. **Consumer:** a GET MUST be addressed to the owner's keys, and SHOULD NOT wait. | `[Sc: state.md §1]` |
| R5 | A consumer MAY wait on presence. | `[Sc: bindings.md §1]` |

### 2.1 Clocks (S7)

- **Ahead.** When an owner's clock is ahead, it MUST stop writing state,
  and SHOULD report it.

## 3. Events

The seconds MUST fit an unsigned 64-bit integer (E026).
`[F: contracts/e026-retention]`

## 4. Facts a deployment designs around

Under `allow`, an operation that must not run for everyone MUST be
`fanout = "forbidden"`.
