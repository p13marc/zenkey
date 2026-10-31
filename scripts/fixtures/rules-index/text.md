# A text with known rules and roles

The self-test of `scripts/gen-rules-index.py` reads this text, and
`expect.json` lists the rules it must read: each one's line, roles, how
each was read, ids and evidence. MUST and SHOULD are used as in RFC 2119
here, and only a rule below states one.

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
  - The owner SHOULD do so in a new session.

  `[Sc: presence.md §3]`
- **The tokenless set.** A deployment MAY configure an owner with a
  tokenless set. These SHOULD be the framework's interfaces.

| # | Rule | Evidence |
|---|---|---|
| S1 | **Owner:** every put MUST carry a stamp. **Consumer:** a GET MUST be addressed to the owner's keys, and SHOULD NOT wait. | `[Sc: state.md §1]` |
| S2 | **Owner:** a GET reply MUST carry its stamp. The owner's queryables cover its state. It MUST answer a selector over them. | `[Sc: state.md §2]` |
| R5 | A consumer MAY wait on presence. | `[Sc: bindings.md §1]` |

A reader MUST NOT conclude absence from a withdrawal alone.
`[Sc: presence.md §9]`

### 2.1 Clocks (S7)

- **Ahead.** When an owner's clock is ahead, it MUST stop writing state,
  and SHOULD report it.

## 3. Events

### 3.1 Requirements

`resources` MUST NOT be empty. `[F: contracts/e030-requirement]`

### 3.2 Retention

The seconds MUST fit an unsigned 64-bit integer (E026).
`[F: contracts/e026-retention]`

## 12. Conformance

The static rules are §1's and §9's, and those of §3.1 on what a contract
states.
