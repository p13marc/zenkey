# Rules whose role the index cannot read

Each rule below names no role the index can read, and the self-test expects
each refused on its line.

## 2. Faces

A batch SHOULD take at most a third of the lease.

It MUST NOT raise it.

An owner MUST mint a new id at a restart.

It SHOULD do so in a new session, since its antecedent is a paragraph back.

| # | Rule | Evidence |
|---|---|---|
| S9 | **Owner:** every put MUST carry a stamp. A batch SHOULD take a third of the lease. | `[Sc: state.md §1]` |

## 3. Events

The seconds MUST fit an unsigned 64-bit integer.

## 4. Facts a deployment designs around

Under `allow`, an operation that must not run for everyone MUST be
`fanout = "forbidden"`.

## 12. Conformance

The static rules are §1's and §9's.
