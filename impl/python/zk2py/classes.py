"""The conformance classes zk2py claims (core §12, 0.66 to 0.69), and its walk of
``spec/RULES.md`` (#609, rounds 89 to 92).

``python -m zk2py.classes`` prints the walk as a Markdown table, one row per
verdict, with what shows it, and on standard error the counts and every
``RULES.md`` row of a claimed class that has no verdict yet.

**The claim.** zk2py plays four of core §0's roles, and claims their four
classes, each at the full level (core §12: not the constrained modifier):
- **owner**: :mod:`zk2py.owner`'s :class:`~zk2py.owner.Owner`, and every
  profile's owner built on it;
- **consumer**: a role's subscription through its bindings
  (:meth:`~zk2py.owner.Owner.subscribe_role`), S4's GET
  (:func:`zk2py.live.get_state`), a late joiner (:mod:`zk2py.seed`), and the
  profiles' consumers (a ``desired.v1`` target, a ``timing.v1`` reader, an
  ``arbitration.v1`` actuator, a ``redundancy.v1`` follower);
- **caller**: :func:`zk2py.live.call` and :func:`zk2py.live.retrieve_bundle`;
- **observer**, the tool role: zk2py's tools (:mod:`zk2py.echo_tool`,
  :mod:`zk2py.timing_tool`, :mod:`zk2py.arbitration_tool`,
  :mod:`zk2py.desired_tool`, the readers of :mod:`zk2py.live` and
  :mod:`zk2py.presence`), and its grant generator (:mod:`zk2py.acl`), a
  generator being a tool (core §0).

It does **not** claim the deployment class: zk2py configures no deployment.
Its runners bring up routers for a scenario, and its generators
(:mod:`zk2py.acl`, :mod:`zk2py.link`) write blocks a deployment runs, which
is a tool's work (core §0, §11.1, §11.2).

The static rules are in every class: :data:`WALK` walks them once, under
``static``.

**Statuses.** ``met``: zk2py does what the entry says, everywhere it plays
the role, and the cited module or family shows it. ``not met``: a claimed
rule zk2py does not keep, a gap to implement or a finding. ``n/a``: the
entry binds a part of the role zk2py does not play (an archive, the
constrained level, a contract's CI), said why.

**Keyed to the rule, not to the index.** ``RULES.md`` is the index, not the
texts (its header says so), and it is regenerated as the texts and its
generator move. So each verdict is keyed to its rule as the text states it:
its section, and a phrase of its own sentence (:func:`norm`). A verdict
walks the row of its class and section that shows that phrase, whatever
antecedent in brackets, cut or evidence the row shows around it.
- A row of a claimed class with no verdict is **reported**, named, by
  :func:`report`, never failed: the next round gives it its verdict.
- A verdict that walks no row is **beyond the index**: a rule the texts bind
  the class to and ``RULES.md`` does not list under it. Since core 0.67 and
  ``observer.md`` 0.9 those are the rules the texts state without a
  keyword, which a class holds and the index does not list (core §0, §12;
  round 89's F-362). Round 89's other rules beyond the index (F-361, F-363)
  are listed rows since 0.67.
- The one failure (:func:`check`) is a verdict whose rule its text no
  longer states, named.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

MET, NOT_MET, NA = "met", "not met", "n/a"

#: core §12's classes zk2py claims, each at the full level.
CLAIMED = {"owner": "full", "consumer": "full", "caller": "full", "observer": "full"}

#: The class zk2py does not claim, and why.
NOT_CLAIMED = {"deployment": "zk2py configures no deployment: its generators are tools (core §0, §11.1)"}

#: ``RULES.md``'s role headings, by class.
HEADINGS = {"Owner": "owner", "Consumer": "consumer", "Caller": "caller", "Deployment": "deployment",
            "Observer: the tool role, and a reader": "observer", "Implementation: the static rules": "static"}

#: The profiles' classes zk2py claims (core §12: "An implementation claims a
#: profile's class by following the profile"), by the roles it plays in each,
#: and what of each it does not do (README, "What it does not cover").
PROFILES = {
    "hostid.v1": ("owner (a minted system), tool (the duplicate check)", ""),
    "freshness.v1": ("owner (re-puts), consumer and tool (the judgements)", ""),
    "health.v1": ("owner, tool (§2.11's reader and roll-ups)", ""),
    "config.v1": ("owner (the simulated device), caller, tool (§5's reader)", ""),
    "link.v1": ("tool (the generator and the checks)", ""),
    "alarms.v1": ("owner, tool (the reader and §5)", ""),
    "telemetry.v1": ("owner, tool (§5's reader)", ""),
    "desired.v1": ("owner (author), consumer (target), tool (§2.17)",
                   "no archive's last-known, so §2.9's store-and-forward is not run"),
    "redundancy.v1": ("owner (a candidate), consumer (the follower), tool (§5)", ""),
    "timing.v1": ("owner (the stamping clock), consumer (deadline and lifespan), tool (§2.7, §5)", ""),
    "arbitration.v1": ("owner (a commander), consumer (an actuator), tool (§2.14, §5)", ""),
}

#: zk2py's verdicts, each keyed to one rule of the texts: (class, where it
#: is stated, its key, status, what shows it). ``where`` is the text and the
#: section the rule is stated in, as ``RULES.md`` heads it; the key is a
#: phrase of the rule's own sentence, from its start, normalized as
#: :func:`norm` does. A verdict walks the ``RULES.md`` row of its class and
#: section whose sentence holds its key, whatever brackets, cuts or evidence
#: the index shows around it. A rule listed under two claimed classes has a
#: verdict in each. The last rows are rules a class holds that the texts state
#: without a keyword, which the index does not list (core §0, §12, 0.67):
#: they walk no row, and are reported as beyond the index.
WALK: list[tuple[str, str, str, str, str]] = [
    # -- owner ---------------------------------------------------------------------------
    ("owner", "core §1.4", "A value with several textual forms SHOULD be", NOT_MET,
     "zk2py's owner slugs the value it is handed (slug.slug); nothing in it knows a value's textual form, and it canonicalizes no IP address or ULID"),
    ("owner", "core §1.5", "An owner MUST mint a new instance id", MET,
     "owner.Owner: 64 random bits at every start, and new_epoch (presence_scenarios §3)"),
    ("owner", "core §1.5", "The owner SHOULD do so in a new", NOT_MET,
     "a restart opens a new session; owner.Owner.new_epoch keeps its session, on purpose (telemetry.v1 scenarios §2, 'the process unchanged'), so its stamps' id stays"),
    ("owner", "core §1.5", "An owner's counter that crosses a face where", MET,
     "telemetry_scenarios' owner: a counter's Point carries its epoch, the instance id (telemetry family)"),
    ("owner", "core §2.3", "An owner MUST expose every required one, or", MET,
     "owner.Owner._plan, §8.2 step 2 (presence_scenarios §2)"),
    ("owner", "core §2.4", "An owner MUST publish with it", MET,
     "owner.qos_of, every publisher and each event's (round 89: reliability, through zenoh's unstable API, and an event's congestion, priority and express; readings)"),
    ("owner", "core §2.7", "An owner MUST NOT hold more live members", MET,
     "owner.Owner._check_budget (budget_scenarios §4–§6)"),
    ("owner", "core §2.7", "An owner MUST NOT publish more: `<n>` +", MET,
     "owner.Owner.occur (budget_scenarios §7)"),
    ("owner", "core §3.2", "Owner: the descriptor (§3.3) MUST list every requirement", MET,
     "owner.Owner._descriptor, a self.system provider resolved (owner.resolve_providers; live_interop, bindings.md §3, §5)"),
    ("owner", "core §3.2", "An owner whose configuration binds a required role", MET,
     "owner.Owner._plan refuses before anything is declared (presence_scenarios §2)"),
    ("owner", "core §3.3", "The owner's descriptor MUST NOT raise it", MET,
     "owner.Owner._plan refuses a stated bound outside 1 to the contract's (round 89; readings)"),
    ("owner", "core §3.3", "One member is used: an owner SHOULD state", MET,
     "owner.Owner._descriptor's meta.zid, unless a test turns it off (state_zid)"),
    ("owner", "core §3.3", "An owner SHOULD write `meta.zid` as zenoh writes", MET,
     "owner.Owner._descriptor writes str(session.zid())"),
    ("owner", "core §3.3", "An owner's descriptor SHOULD stay within 1 KB", NOT_MET,
     "zk2py neither checks nor warns: an owner of enough contracts writes a longer descriptor"),
    ("owner", "core §3.3", "At the constrained level (§12), it MUST fit", NA,
     "zk2py claims no class at the constrained level"),
    ("owner", "core §3.3", "The owner MUST put the descriptor on its", MET,
     "owner.Owner._put_descriptor and its queryable (presence_scenarios §2)"),
    ("owner", "core §4.2", "Owner: every state mutation, put and delete, MUST", MET,
     "owner.Owner.set_state and delete_state, minted (live_interop state.md §1)"),
    ("owner", "core §4.2", "Owner: a GET reply MUST carry the timestamp", MET,
     "owner.Owner.answer_state (live_interop state.md §1; state_scenarios §10)"),
    ("owner", "core §4.2", "It MUST answer a selector over them with", MET,
     "owner.Owner.state_answer, reply_del within the window (state_scenarios §10)"),
    ("owner", "core §4.2", "Owner: for a key it deleted within the", MET,
     "owner.Owner.delete_state and its tombstone window (state_scenarios §10)"),
    ("owner", "core §4.3", "A session that serves state MUST enable Zenoh's", MET,
     "owner.Owner.open_session: timestamping on"),
    ("owner", "core §4.3", "An owner MUST NOT stamp at or below", MET,
     "owner.Owner.mint (the tick) and catch_up from a record (state_scenarios, live_interop state.md §1)"),
    ("owner", "core §4.3", "With neither source, the owner MUST start a", MET,
     "every owner.Owner start is a new instance on a new session, its zid zenoh's random one"),
    ("owner", "core §4.3", "An owner SHOULD NOT stamp beyond its router's", MET,
     "owner.Owner's heartbeat and clock guard (freshness_scenarios §2, health_scenarios §4)"),
    ("owner", "core §4.3", "The owner SHOULD detect drift against a reference", MET,
     "owner.Owner._beat, a router-stamped heartbeat key (freshness_scenarios §2)"),
    ("owner", "core §4.3", "When a detection shows it beyond the delta", MET,
     "owner.ClockAhead stops every state write, re-puts included, and Owner.events reports it (freshness_scenarios §2, health_scenarios §4)"),
    ("owner", "core §4.4", "An archive's backend MUST NOT accept a put", NA,
     "zk2py runs no archive"),
    ("owner", "core §4.4", "When an owner becomes reachable again, the archive", NA,
     "zk2py runs no archive"),
    ("owner", "core §4.4", "An archive SHOULD repeat an alignment that left", NA,
     "zk2py runs no archive"),
    ("owner", "core §4.4", "The archive MAY drop such a key after", NA,
     "zk2py runs no archive"),
    ("owner", "core §4.4", "An archive MUST hold its `archive.v1` interface token", NA,
     "zk2py runs no archive; its owner refuses archive.v1 in a tokenless set (owner.Owner._plan, D011)"),
    ("owner", "core §5.1", "Owner: an operation's queryable MUST be declared on", MET,
     "owner.Owner._declare_resources, complete=True (live_interop operations.md §1)"),
    ("owner", "core §5.1", "Owner: a call whose key expression is not", MET,
     "owner.Owner._op_handler, before any handler (live_interop operations.md §2)"),
    ("owner", "core §5.1", "Owner: a reply MUST go on the operation's", MET,
     "owner.OpCall.reply on the named member's key (live_interop operations.md §2, §3)"),
    ("owner", "core §5.1", "Every call that reaches an owner's queryable MUST", MET,
     "owner.OpCall.finish: internal, never silence"),
    ("owner", "core §5.1", "The active instance of a service MUST answer", MET,
     "owner.Owner._unavailable_handler, with its cause"),
    ("owner", "core §5.1", "With a declared `summary`, each replier MUST end", MET,
     "owner.OpCall.summary: a summary owed at finish, a second refused and no value after it (round 89; readings)"),
    ("owner", "core §5.1", "An instance whose exposed resources of an interface", MET,
     "owner.replica_only: no unavailable queryable on an exclusive operation (round 89; readings)"),
    ("owner", "core §5.1", "Replicas SHOULD expose the same replicated operations, since", MET,
     "an owner.Owner exposes every resource it is not told to list or withhold, so replicas configured alike expose the same"),
    ("owner", "core §5.1", "It names the member each reply answers for", MET,
     "owner.OpCall.name refuses a member the call does not select (live_interop operations.md §2)"),
    ("owner", "core §5.2", "The `Encoding` of the owner's reply MUST say", MET,
     "owner._refuse, the envelope's encoding (envelope.envelope_encoding)"),
    ("owner", "core §5.2", "An operation that declares no `error` type has", MET,
     "owner.OpCall.refuse refuses a detail for it, nothing sent (round 89; readings)"),
    ("owner", "core §5.2", "A detail that does not fit its envelope", MET,
     "envelope.encode raises before anything is sent, and the call ends internal (errors family)"),
    ("owner", "core §6", "An owner MUST NOT write under another service's", MET,
     "owner.Owner builds every key under its own prefix"),
    ("owner", "core §6", "An owner SHOULD keep a re-mint's overlap below", MET,
     "owner.Owner.new_epoch undeclares the old at once (presence_scenarios §3)"),
    ("owner", "core §7.2", "An owner MUST set Zenoh's predefined `Encoding` for", MET,
     "owner._value_encoding and every publisher's encoding"),
    ("owner", "core §7.2", "It MUST NOT add a schema suffix to", MET,
     "owner's encodings carry no suffix; envelope.PROTOBUF_ENCODING alone does"),
    ("owner", "core §7.4", "A generated `@stream` publisher SHOULD accept SHM buffers", NA,
     "zk2py generates no publisher"),
    ("owner", "core §8.1", "Every instance MUST hold one, including pure consumers", MET,
     "owner.Owner._declare_instance (presence_scenarios §1)"),
    ("owner", "core §8.1", "An owner MUST hold one per interface of", MET,
     "owner.Owner, the tokenless set and a standby's none (presence_scenarios §1, §5; redundancy_scenarios §1)"),
    ("owner", "core §8.1", "The owner MUST hold one member token per", MET,
     "owner.Owner.hold_member and release_member (presence_scenarios §3)"),
    ("owner", "core §8.1", "An owner that starts a new instance id", MET,
     "owner.Owner.new_epoch, make before break (presence_scenarios §3)"),
    ("owner", "core §8.2", "An owner MUST bring itself up in this", MET,
     "owner.Owner._bring_up (presence_scenarios §1)"),
    ("owner", "core §8.2", "A state's value is not a declaration, so", MET,
     "owner.Owner._put_held before the tokens (presence_scenarios §1)"),
    ("owner", "core §8.4", "Every holder MUST declare a `complete` queryable on", MET,
     "owner.Owner, a complete queryable per bundle (live_interop's retrieval)"),
    ("owner", "core §12", "An owner's descriptor MUST fit one fragment (Appendix", NA,
     "zk2py claims no class at the constrained level"),
    # -- consumer ------------------------------------------------------------------------
    ("consumer", "core §3.2", "Consumer: MUST discard a sample, or a GET", MET,
     "owner.Owner.subscribe_role, live.get_state, seed.LateJoiner (live_interop bindings.md §4)"),
    ("consumer", "core §3.2", "Consumer: a binding MUST NOT require presence", MET,
     "owner.Owner.subscribe_role subscribes at once, without presence"),
    ("consumer", "core §3.2", "A consumer's runtime MUST resolve a provider whose", MET,
     "owner.resolve_providers at start (live_interop bindings.md §5)"),
    ("consumer", "core §4.2", "Consumer: a state GET MUST be addressed to", MET,
     "live.get_state and seed.LateJoiner: All and Latest (state_scenarios §3)"),
    ("consumer", "core §4.2", "Consumer: last-known state MUST be read from an", NA,
     "zk2py's consumers read no last-known state"),
    ("consumer", "core §4.2", "The archive's answer is last-known, never current, and", NA,
     "zk2py's consumers read no archive"),
    ("consumer", "core §4.2", "A late joiner MUST declare its subscription first", MET,
     "seed.LateJoiner (state_scenarios §10)"),
    ("consumer", "core §4.2", "A late joiner MUST merge the GET's replies", MET,
     "seed.LateJoiner's seed.Merge: one per-key merge of both paths, through one queue in arrival order (readings; "
     "state_scenarios §10)"),
    # core 0.69 split the merge's MUST into three sentences (F-401): the
    # second and the third are rows of their own under consumer.
    ("consumer", "core §4.2", "It MUST keep the newer by the owner's stamp", MET,
     "seed.LateJoiner's seed.Merge, rule S7: within one id by the stamp, as Latest compares (seed.compare, "
     "seed.follows; seed family; state_scenarios §10)"),
    ("consumer", "core §4.2", "Across two timestamp ids, a consumer MUST instead keep what", MET,
     "seed.follows: across two ids, a value under an id other than the held value's kept, its seed's replies "
     "included (readings; state_scenarios §7 step 2, after the seed and inside it; live_interop s7)"),
    ("consumer", "core §4.3", "A consumer MUST order each key's values by", MET,
     "seed.LateJoiner, a consumer's: seed.Merge's rule S7 from the first value it receives, its seed's replies "
     "included (round 91; readings; state_scenarios §7); desired.TargetRole keeps desired.v1 §2.6's stricter "
     "order, as §4.3's MAY lets a profile; owner.Owner.subscribe_role carries a stream alone in zk2py"),
    ("consumer", "core §4.3", "It MUST accept a value under an id other", MET,
     "seed.follows: an id other than the held value's is kept, whatever its time, from the seed's first reply on "
     "(readings; state_scenarios §7 step 2, a new epoch 5 s behind; live_interop s7)"),
    ("consumer", "core §8.1", "Every instance MUST hold one, including pure consumers", MET,
     "every zk2py service is an owner.Owner, which holds one (presence_scenarios §1)"),
    ("consumer", "core §8.4", "Every holder MUST declare a `complete` queryable on", MET,
     "a zk2py consumer is an owner.Owner service, and the bundles it holds, those of the interfaces it implements, "
     "are each on a complete queryable (live_interop's retrieval); it holds none of an interface it only requires"),
    ("consumer", "core §11.1", "History (§2.5): a consumer's advanced subscriber MUST NOT", NA,
     "zk2py's consumers declare no advanced subscriber (budget_scenarios §3's, a measurement, declares no detection token)"),
    # -- caller --------------------------------------------------------------------------
    ("caller", "core §5.1", "Caller: a concrete call MUST use target `BestMatching`", MET,
     "live.call (live_interop operations.md §1)"),
    ("caller", "core §5.1", "Caller: a call to a fan-out operation MUST", MET,
     "live.call(fanout=True) (live_interop operations.md §2)"),
    ("caller", "core §5.1", "Caller: MUST NOT retry an operation that is", MET,
     "live.call never retries"),
    ("caller", "core §5.1", "Caller and tool: MUST NOT treat an empty", MET,
     "live.CallResult.silent, live.o3_verdict (observer_scenarios, operations.md §5)"),
    ("caller", "core §5.1", "Caller: MUST use consolidation `None`, since `Latest` and", MET,
     "live.call"),
    ("caller", "core §5.1", "A caller MUST NOT depend on a refusal", MET,
     "live.call sends what it is handed, and reads a refusal as an answer, never as a check"),
    ("caller", "core §8.1", "Every instance MUST hold one, including pure consumers", MET,
     "a zk2py service that calls is an owner.Owner, which holds one (presence_scenarios §1; F-361)"),
    ("caller", "core §8.4", "Every holder MUST declare a `complete` queryable on", MET,
     "a zk2py service that calls is an owner.Owner, and the bundles it holds, its own interfaces', are each on a "
     "complete queryable (live_interop's retrieval); a tool session that calls holds none"),
    ("caller", "core §8.4", "A caller or tool MUST retrieve a bundle", MET,
     "live.retrieve_bundle (live_interop's retrieval, retrieval.md §2, §3)"),
    ("caller", "core §8.4", "A caller or tool MUST set consolidation `None` on both", MET,
     "live._answers, both of live.retrieve_bundle's attempts"),
    ("caller", "core §8.4", "A caller or tool MUST NOT depend on the encoding", MET,
     "live.retrieve_bundle verifies the hash, never the encoding (retrieval.md §1)"),
    ("caller", "observer §3.1", "A caller or tool's liveliness GET on a", MET,
     "live.list_presence, a callback handler (live_interop presence.md §4)"),
    ("caller", "observer §3.1", "Every liveliness subscriber the caller or tool holds", MET,
     "live.declare_liveliness_subscriber, callbacks only"),
    # -- observer ------------------------------------------------------------------------
    ("observer", "core §4.2", "A late joiner MUST declare its subscription first", MET,
     "seed.LateJoiner, the seeding tools' reads (state_scenarios §10); a late joiner is \"a consumer or a tool\""),
    ("observer", "core §4.2", "A late joiner MUST merge the GET's replies", MET,
     "seed.LateJoiner, a tool's: one per-key merge of both paths, bounded at seed.MAX_KEYS; the alarms, health and "
     "config readers merge their recordings and GETs through seed.merged (round 91)"),
    # core 0.69 (F-401): the merge's second sentence, a row of its own.
    ("observer", "core §4.2", "It MUST keep the newer by the owner's stamp", MET,
     "seed.LateJoiner, a tool's: seed.Merge's rule merge, the newer by the stamp as Latest compares, for as long as "
     "it follows, across two ids too (observer §4.1; seed family; state_scenarios §7's control); seed.merged for the "
     "readers"),
    ("observer", "core §5.1", "Caller and tool: MUST NOT treat an empty", MET,
     "judgement, live.o3_verdict (observer_scenarios, operations.md §5)"),
    ("observer", "core §5.2", "It MUST refuse", MET,
     "envelope.decode (errors family)"),
    ("observer", "core §8.1", "Every instance MUST hold one, including pure consumers", NA,
     "no zk2py tool is a service, so none is an instance (observer §7.1's refusal of a self.system provider, met)"),
    ("observer", "core §8.4", "Every holder MUST declare a `complete` queryable", NA,
     "no zk2py tool holds bundles"),
    ("observer", "core §8.4", "A caller or tool MUST retrieve a bundle", MET,
     "live.retrieve_bundle (live_interop's retrieval, retrieval.md §2, §3)"),
    ("observer", "core §8.4", "A caller or tool MUST set consolidation `None` on both", MET,
     "every zk2py tool retrieves through live.retrieve_bundle (arbitration_tool, desired_tool, redundancy_live, "
     "live's contract store), whose two attempts go through live._answers (core 0.68, F-387)"),
    ("observer", "core §8.4", "A caller or tool MUST NOT depend on the encoding", MET,
     "live.retrieve_bundle verifies the hash, never the encoding, for every tool (core 0.68, F-387)"),
    ("observer", "core §11.1", "A generator MUST refuse an input that binds", MET,
     "acl.Principal binds by one name: no input can bind one by both"),
    ("observer", "core §11.1", "Under `deny`, a generator MUST NOT compile an", MET,
     "acl, admin_records (security_scenarios §4)"),
    ("observer", "core §11.2", "A generator MUST therefore add to a provider's", MET,
     "acl.compile_grants (acl_interop security.md §2)"),
    ("observer", "core §11.2", "A generator MUST grant the consumer selectors above", MET,
     "acl.compile_grants (acl_interop security.md §2)"),
    ("observer", "core §11.2", "A generator MUST deny, in each principal's policy", MET,
     "acl.compile_grants (acl_interop security.md §2)"),
    ("observer", "observer §3.1", "A caller or tool's liveliness GET on a", MET,
     "live.list_presence"),
    ("observer", "observer §3.1", "Every liveliness subscriber the caller or tool holds", MET,
     "live.declare_liveliness_subscriber"),
    ("observer", "observer §3.2", "A tool that decides a fault from the", MET,
     "live.presence_faults, two reads a grace apart (observer_scenarios, operations.md §8)"),
    ("observer", "observer §3.3", "A tool SHOULD treat a liveliness GET that", MET,
     "live.list_presence (presence_scenarios §6)"),
    ("observer", "observer §3.3", "A reader MUST read as possibly incomplete a", MET,
     "live.list_presence's at_interests_timeout (presence_scenarios §10)"),
    ("observer", "observer §3.3", "A reader MUST read as not standing a", MET,
     "live.settle (presence_scenarios §10)"),
    ("observer", "observer §3.3", "A reader whose reads may wait longer SHOULD", MET,
     "live.open_client, a 120 s interests timeout"),
    ("observer", "observer §3.3", "A reader that cannot read its session's interests", MET,
     "live.open_client opens its readers' sessions and records the value"),
    ("observer", "observer §3.4", "Where it cannot rule out a refusal, it", MET,
     "live.Presence.reading: 'absent, or refused by access control'"),
    ("observer", "observer §3.5", "A reader MUST NOT conclude absence from it", MET,
     "live.list_presence, presence.PresenceView (presence_scenarios §7)"),
    ("observer", "observer §3.5", "A reader SHOULD NOT make such a read", MET,
     "live.list_presence: not made (presence_scenarios §7)"),
    ("observer", "observer §3.6", "A reader MUST NOT conclude absence from a", MET,
     "presence.PresenceView (presence_scenarios §8)"),
    ("observer", "observer §3.6", "A reader that follows presence live SHOULD read", MET,
     "presence.PresenceFollower (presence_scenarios §8)"),
    ("observer", "observer §3.6", "The reader SHOULD report what that read establishes", MET,
     "presence.PresenceFollower (presence_scenarios §8)"),
    ("observer", "observer §3.7", "A reader SHOULD make every liveliness read it", MET,
     "live.list_presence refuses a session that held a subscriber (presence_scenarios §9)"),
    ("observer", "observer §3.7", "The reader SHOULD hold its liveliness subscribers on", MET,
     "presence.PresenceFollower's watch session (presence_scenarios §9)"),
    ("observer", "observer §3.7", "A reader that seeds on one session and", MET,
     "live.Barrier (presence_scenarios §9, §10)"),
    ("observer", "observer §3.7", "The subscribers of a reader's watch session SHOULD", MET,
     "presence.PresenceFollower, no history (presence_scenarios §10)"),
    ("observer", "observer §3.7", "A reader that follows presence in a namespace", MET,
     "live.wire_selector, live.Stripped (presence_scenarios §10)"),
    ("observer", "observer §3.7", "A reader that keeps a namespaced watch session", MET,
     "presence.PresenceFollower handed a namespaced watch: live.flight_timeout and a 120 s interests timeout"),
    ("observer", "observer §3.7", "A reader that follows presence live SHOULD make", MET,
     "presence.PresenceFollower's transport events (presence_scenarios §10)"),
    ("observer", "observer §3.7", "So a reader that holds back what a", MET,
     "arbitration_live, the move begun at a withdrawal with no link (arbitration_scenarios §7)"),
    ("observer", "observer §3.7", "A reader SHOULD ask the round trip after", MET,
     "arbitration_live.open_margin_s; presence.PresenceFollower's attempt 0.1 s after a move (arbitration_scenarios §7)"),
    ("observer", "observer §3.7", "After a move, a reader SHOULD wait for", MET,
     "presence.PresenceFollower's move_wait, until it ends (presence_scenarios §10)"),
    ("observer", "observer §3.7", "It SHOULD run its interests timeout above that", MET,
     "live.open_client, 120 s"),
    ("observer", "observer §3.7", "A reader SHOULD NOT ask a round trip", MET,
     "live.Barrier, one in flight (presence_scenarios §10)"),
    ("observer", "observer §3.7", "It SHOULD NOT ask a read while one", MET,
     "live.Flight (presence_scenarios §10)"),
    ("observer", "observer §3.7", "It SHOULD NOT make a read that it", MET,
     "presence.PresenceFollower._watch_cut_off (presence_scenarios §10)"),
    ("observer", "observer §3.9", "A reader that concludes absence of a token", MET,
     "live.open_client: every reader is a client of a router (redundancy_scenarios §5)"),
    ("observer", "observer §4.3", "A tool MUST NOT ask a selector that", MET,
     "live.admin_guard (security_scenarios §4, state_scenarios §12)"),
    ("observer", "observer §4.4", "A tool MUST compare two zids by value", MET,
     "live._zid_value (timing family, lifespans.json)"),
    ("observer", "observer §6.1", "A tool's grace period SHOULD exceed the longest", MET,
     "live.SHAPE_GRACE_S, 1 s, above an owner's overlap below one second"),
    ("observer", "observer §7.1", "Where nothing crosses, liveness is *unobservable*, and a", MET,
     "timing_tool and telemetry_scenarios across a face: owners unseen, never down (link_scenarios §6)"),
    ("observer", "observer §7.1", "A tool that is not a service MUST", MET,
     "owner.resolve_providers with no system raises"),
    ("observer", "observer §7.3", "Such a tool MUST decode each payload by its declared", NOT_MET,
     "observer 0.10 asks each payload decoded by its own declared type (F-379's reading (b)): the profiles' readers "
     "(alarms, telemetry, desired_tool, health, config) meet it, each payload they judge being their profile's JSON "
     "Schema type, which they decode; echo_tool shows each sample, its size and encoding in its payload's place, "
     "and decodes neither family, so zk2py fails it there (types.md §2 not run)"),
    ("observer", "observer §7.3", "A tool that shows a payload it cannot decode MUST", NOT_MET,
     "echo_tool renders each sample by its size and its sample's encoding, never its declared type: it reads no "
     "contract; the profiles' readers show no payload they cannot decode, an undecodable one being their "
     "judgement's own answer"),
    ("observer", "observer §7.3", "It MUST NOT show garbage, and MUST NOT", MET,
     "echo_tool shows no payload bytes and drops no sample (observer_scenarios)"),
    ("observer", "observer §7.4", "A tool SHOULD check the limit at start", MET,
     "live.memlock_said, at the start of timing_tool, arbitration_tool and echo_tool (round 89)"),
    # -- static --------------------------------------------------------------------------
    ("static", "core §1.1", "An implementation MUST refuse to build one, and", MET,
     "keys family"),
    ("static", "core §1.1", "Every key it accepts MUST build back to", MET,
     "keys family"),
    ("static", "core §1.3", "Two consequences follow, and an implementation MUST NOT", MET,
     "templates family, observer_scenarios (grammar.md §1)"),
    ("static", "core §1.4", "Both MUST be implemented exactly as follows", MET,
     "slugs family"),
    ("static", "core §1.4", "The decoder MUST accept exactly the canonical chunks", MET,
     "slugs and templates families"),
    ("static", "core §1.6", "Application code SHOULD NOT spell a deployment prefix", MET,
     "keys are base-relative; a namespace is the session's (live.open_client)"),
    ("static", "core §2.2", "A rest parameter MUST NOT appear except as the last segment", MET,
     "contracts family (E010: e010-template, a rest parameter that is not last; core 0.67's restatement of "
     "\"MAY appear only\", F-363)"),
    ("static", "core §2.2", "Within one interface, two templates under the same", MET,
     "contracts family (E021)"),
    ("static", "core §2.2", "A template with parameters MUST declare `cardinality`, a", MET,
     "contracts family (E013)"),
    ("static", "core §2.2", "A template without parameters MUST NOT declare it", MET,
     "contracts family (E014)"),
    ("static", "core §2.2", "A contract MAY write the number instead: the", MET,
     "contracts family (ok-cardinality-unbounded, ok-cardinality-no-ceiling)"),
    ("static", "core §2.4", "A contract's state and events SHOULD keep `reliable`", MET,
     "every state and event of zk2py's own contracts (interop/) keeps reliable and block; no lint has a code for it"),
    ("static", "core §2.4", "A contract's author SHOULD set it only on", MET,
     "no contract zk2py wrote sets express (arbitration/steer.v1 does, copied from its scenarios)"),
    ("static", "core §2.5", "A contract MUST NOT declare `history` on an", MET,
     "contracts family (E019)"),
    ("static", "core §2.6", "The seconds, `<n>` times the unit's, MUST fit", MET,
     "contracts family (E026)"),
    ("static", "core §2.6", "Like every canonical integer, they MUST also be", MET,
     "contracts family (E028)"),
    ("static", "core §3.1", "`resources` names resources of the required interface, and", MET,
     "contracts family (E030)"),
    ("static", "core §3.3", "Its schema is `descriptor.schema.json`, and a checker MUST", MET,
     "descriptors family"),
    ("static", "core §5.1", "A request type SHOULD NOT repeat a template", MET,
     "contracts family (W102)"),
    ("static", "core §7.1", "The name MUST be unique across the listed", MET,
     "contracts family (E024)"),
    ("static", "core §9.1", "A contract MUST NOT need later TOML", MET,
     "contracts family (the five e000-toml11 cases)"),
    ("static", "core §9.2", "An implementation MUST report, for each fixture, exactly", MET,
     "contracts and sets families"),
    ("static", "core §9.3", "A defaulted contract and the same contract with", MET,
     "contracts family"),
    ("static", "core §9.4", "Its `name` is the file's stem, the last", MET,
     "contracts family (E024)"),
    ("static", "core §9.4", "The id MUST be unique among the listed", MET,
     "contracts family (e024-identical)"),
    ("static", "core §9.4", "Otherwise it resolves as a path relative to", MET,
     "contracts family (E032: e032-ref-outside)"),
    ("static", "core §9.4", "The pointer MUST resolve", MET,
     "contracts family (E032)"),
    ("static", "core §9.5", "An implementation MUST produce, for every valid fixture", MET,
     "contracts family"),
    ("static", "core §9.6", "To verify bundle bytes, an implementation MUST check", MET,
     "bundles family"),
    ("static", "core §9.7", "The history check MUST verify", MET,
     "history family"),
    ("static", "core §9.8", "Breaking: a contract's CI MUST NOT publish the", NA,
     "zk2py is no contract's CI and publishes no revision; its compat family classifies"),
    ("static", "core §9.8", "Review: a human MUST accept it before publication", NA,
     "zk2py publishes no revision; a human is no implementation"),
    ("static", "core §9.8", "The classifier MUST classify every case of `compat/`", MET,
     "compat family"),
    ("static", "core §10", "A contract that uses one MUST list the", MET,
     "contracts family (E020, W105)"),
    # -- rules a claimed class holds that the texts state without a keyword,
    # -- which RULES.md does not list (core §0 and §12 since 0.67, F-362)
    ("observer", "observer §1", "Every verdict a tool presents fits one shape", MET,
     "judgement (budget, freshness, telemetry, desired, timing, arbitration, redundancy families; F-362)"),
    ("observer", "observer §1", "The two Unestablished kinds stay apart in every medium", MET,
     "judgement's cells, fields and notes (readings; F-362)"),
    ("observer", "observer §1", "An empty scope is Unobservable", MET, "budget, timing (budget family; F-362)"),
    ("observer", "observer §2", "Unattributable silence is Unestablished, never Established(no)", MET,
     "live.o3_verdict, judgement (observer_scenarios; F-362)"),
    ("observer", "observer §4.1", "A tool keeps core §4.2's merge for as long as it follows", MET,
     "seed.LateJoiner, a tool's: seed.Merge's rule merge, across two ids too, after the seed as inside it "
     "(state_scenarios §7 and its run inside the seed; live_interop s7; observer 0.10, round 91)"),
    ("observer", "observer §4.1", "A tool that bounds what it holds counts the keys the bound forgot, and reports "
     "the count", MET,
     "seed.Merge and freshness.Subscriber, each bounded at seed.MAX_KEYS, the least recent key forgotten whole and "
     "counted in forgotten, holding nothing after; every reading of what they hold reports the count: "
     "alarms.OwnerReading, health.Reading, config.Reading and freshness.read_service's forgotten, and a tool's "
     "LateJoiner.bound_note (readings; observer 0.11, round 92)"),
    ("observer", "observer §5.2", "A complete reading of a stream or an event", MET,
     "budget.Window (budget family, budget_scenarios; F-362)"),
    ("observer", "observer §7.3", "Decode order for a tool is: the declared type, then the sample's", NOT_MET,
     "echo_tool decodes nothing: it shows each sample's size and encoding, reads no contract, and sniffs no "
     "foreign payload; the profiles' readers decode their profile's JSON Schema type on the wire their profile "
     "names (telemetry.v1's resource encoding, alarms.v1's strict JSON), never by the sample's Encoding, which "
     "0.11 says picks the wire (round 92's F-409)"),
    ("observer", "observer §7.1", "The data-flow graph is read from descriptors, never inferred", MET,
     "live.get_descriptor (live_interop, bindings.md §3; F-362)"),
    ("observer", "observer §8.1", "A tool never hides or refuses a foreign key", MET,
     "echo_tool's foreign rows, zk2 false (observer_scenarios; F-362)"),
    ("observer", "observer §8.2", "A tool that offers a wildcard scope does one of two things", MET,
     "echo_tool's scope, live.blind_spots (observer_scenarios, grammar.md §1; F-362)"),
    ("observer", "observer §8.3", "A tool says whose clock a time, an age or a latency is on", MET,
     "timing_tool, live.attribute_stamp (readings; F-362)"),
]


def norm(text: str) -> str:
    """A sentence as a verdict's key is matched against it, in the texts and
    in ``RULES.md`` alike: evidence tags out, a link read as its text, any
    other bracket the index inserts (an antecedent shown, ``[an instance
    token]``) out, ``(n items)`` out, emphasis and escapes out, whitespace
    collapsed."""
    text = re.sub(r"`\[(?:F|Sc): [^\]]*\]`", "", text)
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = re.sub(r"\s*\[[^\]]*\]", "", text)
    text = re.sub(r"\s*\(\d+ items?\)", "", text)
    text = text.replace("**", "").replace("\\|", "|")
    return " ".join(text.split())


def _sections(path: Path) -> dict[str, str]:
    """A text's sections by number (``1.5``, ``12``), each normalized, a
    chapter's holding its subsections."""
    out: dict[str, list[str]] = {}
    open_: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        m = re.match(r"^(#{2,3}) (\d+(?:\.\d+)?)\.? ", line)
        if m:
            depth = len(m.group(1))
            open_ = [n for n in open_ if n.count(".") < depth - 2] if depth == 3 else []
            open_.append(m.group(2))
            out.setdefault(m.group(2), [])
        elif re.match(r"^#{1,2} ", line):
            open_ = []
        for n in open_:
            out[n].append(line)
    return {n: norm(" ".join(lines)) for n, lines in out.items()}


def _texts(spec: Path) -> dict[str, dict[str, str]]:
    return {"core": _sections(spec / "core.md"), "observer": _sections(spec / "observer.md")}


def _rules_md(spec: Path) -> list[tuple[str, str, str, str]]:
    """``RULES.md``'s entries as (class, where, rule column, sentence), in its
    order, the rule-id table left out. ``where`` is the row's section heading,
    ``core §1.5``."""
    out, role, where = [], None, None
    for line in (spec / "RULES.md").read_text(encoding="utf-8").splitlines():
        m = re.match(r"^## (.*)", line)
        if m:
            role = HEADINGS.get(m.group(1).strip())
            continue
        m = re.match(r"^### \[(core|observer) §([0-9.]+)", line)
        if m:
            where = f"{m.group(1)} §{m.group(2)}"
            continue
        m = re.match(r"^\| \[([^\]]+)\]\([^)]+\)[^|]*\| (.*) \| [^|]* \|$", line)
        if m and role is not None and where is not None:
            out.append((role, where, m.group(1), m.group(2)))
    return out


class Reading:
    """The walk read against the texts and ``RULES.md``:

    - ``walked``: each ``RULES.md`` row of a claimed class or of the static
      rules, with the verdicts whose key it holds;
    - ``unwalked``: those rows with no verdict, which the next round walks;
    - ``beyond``: the verdicts that walk no row, rules the index does not
      list under their class;
    - ``gone``: the verdicts whose key no longer stands in their section of
      the texts, the one failure."""

    def __init__(self, spec: Path) -> None:
        texts = _texts(spec)
        self.rows = [r for r in _rules_md(spec) if r[0] in CLAIMED or r[0] == "static"]
        self.walked: list[tuple[tuple[str, str, str, str], list[tuple[str, ...]]]] = []
        hit: set[int] = set()
        for row in self.rows:
            shown = norm(row[3])
            found = [i for i, v in enumerate(WALK) if v[0] == row[0] and v[1] == row[1] and norm(v[2]) in shown]
            hit.update(found)
            self.walked.append((row, [WALK[i] for i in found]))
        self.unwalked = [row for row, vs in self.walked if not vs]
        self.beyond = [v for i, v in enumerate(WALK) if i not in hit]
        self.gone = []
        for v in WALK:
            text, _, number = v[1].partition(" §")
            section = texts.get(text, {}).get(number)
            if section is None or norm(v[2]) not in section:
                self.gone.append(v)


def structure() -> list[str]:
    """What :data:`WALK` must hold of itself: a class zk2py claims or the
    static rules, a section, one of the three statuses with what shows it,
    and one verdict per key in its class and section."""
    problems, seen = [], set()
    for v in WALK:
        if v[0] not in CLAIMED and v[0] != "static":
            problems.append(f"{v[:3]}: {v[0]} is no class zk2py claims")
        if not re.fullmatch(r"(core|observer) §\d+(\.\d+)?", v[1]):
            problems.append(f"{v[:3]}: no section")
        if v[3] not in (MET, NOT_MET, NA) or not v[4]:
            problems.append(f"{v[:3]}: status {v[3]!r} without what shows it")
        if v[:3] in seen:
            problems.append(f"{v[:3]}: two verdicts")
        seen.add(v[:3])
    return problems


def check(spec: Path) -> list[str]:
    """The walk's one failure: a verdict whose rule its text no longer
    states, each named. A ``RULES.md`` row with no verdict is reported
    (:class:`Reading`), never failed."""
    return [f"{v[0]} {v[1]}: {v[2]!r}: its text no longer states this rule" for v in Reading(spec).gone]


def counts() -> dict[str, dict[str, int]]:
    """Per claimed class and the static rules, the verdicts by status."""
    out: dict[str, dict[str, int]] = {}
    for cls in [*CLAIMED, "static"]:
        rows = [r for r in WALK if r[0] == cls]
        out[cls] = {s: sum(1 for r in rows if r[3] == s) for s in (MET, NOT_MET, NA)}
    return out


def report(spec: Path) -> list[str]:
    """What a run reports of the walk: the counts, the ``RULES.md`` rows
    with no verdict, named, and the verdicts beyond the index."""
    r = Reading(spec)
    lines = ["classes: " + "; ".join(f"{cls} " + ", ".join(f"{n} {s}" for s, n in c.items())
                                     for cls, c in counts().items())]
    lines.append(f"classes: {len(r.rows)} RULES.md rows of the claimed classes and the static rules, "
                 f"{len(r.rows) - len(r.unwalked)} walked, {len(r.unwalked)} with no verdict; "
                 f"{len(r.beyond)} verdicts beyond the index")
    lines += [f"classes: no verdict for {cls} {where} {rid}: {norm(sentence)[:80]!r}"
              for cls, where, rid, sentence in r.unwalked]
    return lines


def table() -> str:
    """The walk as a Markdown table."""
    lines = ["| Class | Where | Rule | Status | What shows it |", "|---|---|---|---|---|"]
    for cls, where, key, status, why in WALK:
        key_cell, why_cell = key.replace("|", "\\|"), why.replace("|", "\\|")
        lines.append(f"| {cls} | {where} | {key_cell}… | {status} | {why_cell} |")
    return "\n".join(lines)


if __name__ == "__main__":
    spec = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[3] / "spec"
    bad = structure() + check(spec)
    for b in bad:
        print(b, file=sys.stderr)
    print(table())
    for line in report(spec):
        print(line, file=sys.stderr)
    sys.exit(1 if bad else 0)
