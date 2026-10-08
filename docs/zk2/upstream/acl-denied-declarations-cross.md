# Draft upstream report: an ACL deny does not keep declarations off a router-to-router link

**Status: not filed.** The maintainer decided on 2026-10-08 not to file it
(#599, #605). It is kept as a record of zenoh 1.10.1's behaviour, which r4
designs around. The measurements are
[`../spike-report.md`](../spike-report.md) § S3. The raw data is
[`../spike-results/s3-probes/`](../spike-results/s3-probes/), and the probe is
`spike s3-acl-probe` on branch `zk2-spike`.

---

**Title:** access control: denied declarations still cross a router-to-router
link, key strings included

**Version:** zenoh 1.10.1 (routers in router mode, TCP between them).

**What happens.** Two routers, A and B, are linked by TCP. Both carry the same
ACL, with `default_permission: allow` and one deny rule:
- messages: every type, including `declare_queryable`, `liveliness_token` and
  `declare_subscriber`;
- flows: ingress and egress;
- key: `zk2/**/@zk/**`;
- subject: `link_protocols: ["tcp"]`. Local clients reach both routers by unix
  socket, so the subject is only the A↔B link.

A client of A declares 200 liveliness tokens under `zk2/v/<svc>/@zk/instance/<id>`.

- **Correct:** a client of B sees none of them (liveliness GET or subscriber).
- **Wrong:** the A→B bytes still carry all 200 keys. `@zk/instance` occurs 201
  times in the captured stream: the 200 tokens, plus one more token declared
  by the probe on the same prefix. The link's byte count drops only from
  11,811 B (no ACL) to 10,530 B.

| Setup (A→B bytes for 200 denied tokens) | Bytes | Denied keys in the bytes | B sees the tokens |
|---|---|---|---|
| No ACL | 11,811 | 201 | yes |
| The deny on A and B | 10,530 | **201** | no |
| The deny on A only | 11,097 | 201 | no |
| The deny on A only, egress only | 10,719 | 201 | no |
| The deny on B only | 11,820 | 201 | no |
| **B's session is a client of A** (no router B), the deny on A | **285** | **0** | no |

The same holds for queryable declarations. 200 data queryables cost 10,038 B
without the ACL. With a deny on their key expression they cost 8,737 B, and
all 200 keys still cross.

**Expected.** A declaration that a face's egress rule denies is not sent on
that face. Today it is sent anyway, which has two consequences:
- **Bandwidth:** a deployment cannot use an ACL to keep a family of
  declarations off a constrained link. Our case is a 2,400 bit/s radio. A
  bring-up of 50 services with `@zk` denied still took 37.7 s and 11.1 KB of
  airtime. Over a client link it took 0.2 s and 17 B.
- **Disclosure:** the key names under a denied expression reach the other
  side of the face.

**Not identified:** the mechanism. The key strings may travel as key
expression declarations (wire mappings) ahead of the filtered declaration, or
the filter may apply only at the receiving router. A client link carries only
what its interests ask for, so it does not show the problem.

**Repro:** `s3/probes.sh` on branch `zk2-spike` runs every row above. It writes
[`probes.log`](../spike-results/s3-probes/probes.log) and the ACL files beside
it. The deny is
[`acl-deny-zk.json`](../spike-results/s3-probes/acl-deny-zk.json).
