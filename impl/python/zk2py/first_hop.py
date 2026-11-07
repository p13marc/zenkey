"""A first hop, as a tool reads it (core 0.96 §8.1; observer 0.26 §3.6,
§3.11; link.v2 0.7 §2.5; #609, round 102).

- **How a tool knows** an owner is on a first hop: "The face policy lists the
  owner on a device face" (observer §3.11, link.v2 §2.5).
  :class:`FacePolicy` holds those devices, read from a link.v2 policy
  (:meth:`FacePolicy.of_policy`) or given.
- **The report** (:func:`report`): a provider found by its instance token
  and its descriptor (§3.6), its interfaces the descriptor's, each
  ``"token": false`` no fault; a device whose instance token a complete read
  does not find is ``link_closed``, never ``down`` ("A tool that knows an
  owner is on a first hop MUST NOT report it down from its instance token's
  absence, even where a complete read establishes that absence. It reports
  the owner's link closed, and its liveness from what crosses"); any other
  owner a complete read does not find is ``absent``, and an incomplete read
  ``unobservable``."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any


@dataclass
class FacePolicy:
    """The devices a deployment's face policies list (link.v2 §2.5)."""

    devices: list[str] = field(default_factory=list)

    @classmethod
    def of_policy(cls, text: str) -> "FacePolicy":
        from . import link_v2

        p, _ = link_v2.parse_policy(text)
        return cls(sorted({d for f in (p.faces if p else []) for d in (f.devices or [])}))

    def on_first_hop(self, address: str) -> bool:
        return address in self.devices


def report(session: Any, address: str, policy: FacePolicy) -> dict[str, Any]:
    """One owner's presence as a tool reports it: ``present`` with its
    descriptor's interfaces and their token marks, ``link_closed`` for a
    device on a first hop whose instance token a complete read does not find,
    ``absent`` for any other owner it does not find, or ``unobservable``."""
    from . import live

    pres = live.list_presence(session, f"zk2/{address}/@zk/**")
    if pres.instances:
        inst = pres.instances[0]["instance"]
        d = live.get_descriptor(session, f"zk2/{address}/@zk/instance/{inst}")
        doc = json.loads(d[0].payload) if d and d[0].ok else {}
        ifaces = {e["iface"]: e.get("token", True) for e in doc.get("interfaces", [])}
        return {"state": "present", "instance": inst, "interfaces": ifaces,
                "first_hop": policy.on_first_hop(address)}
    if not pres.complete:
        return {"state": "unobservable", "why": "the presence read is possibly incomplete"}
    if policy.on_first_hop(address):
        return {"state": "link_closed", "why": "a device on a first hop: its instance token follows its link "
                                               "(observer §3.11); its liveness is what crosses"}
    return {"state": "absent"}
