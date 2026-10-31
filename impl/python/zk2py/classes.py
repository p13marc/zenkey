"""The conformance classes zk2py claims (core §12, 0.66), and its walk of
``spec/RULES.md`` (#609, round 89).

``python -m zk2py.classes`` prints the walk as a Markdown table, one row per
``RULES.md`` entry of a claimed class, with what shows it.

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
``static``, and :func:`counts` counts them in each class.

**Statuses.** ``met``: zk2py does what the entry says, everywhere it plays
the role, and the cited module or family shows it. ``not met``: a claimed
rule zk2py does not keep, a gap to implement or a finding. ``n/a``: the
entry binds a part of the role zk2py does not play (an archive, the
constrained level, a contract's CI), said why.

``RULES.md`` is the index, not the texts (its header says so). The rules it
leaves out that bind a claimed class are :data:`BEYOND`, walked too, each
with the finding that names it.
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

#: One row per ``RULES.md`` entry: (class, rule column, the sentence's
#: start after any run-in label, status, what shows it). A rule listed under
#: two roles has a row under each.
WALK: list[tuple[str, str, str, str, str]] = [
    # -- owner -------------------------------------------------------------------------
    ("owner", "§1.4", "A value with several textual forms SHOULD be", NOT_MET,
     "zk2py's owner slugs the value it is handed (slug.slug); nothing in it knows a value's textual form, "
     "and it canonicalizes no IP address or ULID"),
    ("owner", "§1.5", "An owner MUST mint a new instance id wheneve", MET,
     "owner.Owner: 64 random bits at every start, and new_epoch (presence_scenarios §3)"),
    ("owner", "§1.5", "The owner SHOULD do so in a new session, so ", NOT_MET,
     "a restart opens a new session; owner.Owner.new_epoch keeps its session, on purpose "
     "(telemetry.v1 scenarios §2, 'the process unchanged'), so its stamps' id stays"),
    ("owner", "§1.5", "An owner's counter that crosses a face where", MET,
     "telemetry_scenarios' owner: a counter's Point carries its epoch, the instance id (telemetry family)"),
    ("owner", "§2.3", "An owner MUST expose every required one, or ", MET,
     "owner.Owner._plan, §8.2 step 2 (presence_scenarios §2)"),
    ("owner", "§2.4", "An owner MUST publish with it.", MET,
     "owner.qos_of, every publisher and each event's (round 89: reliability, through zenoh's unstable "
     "API, and an event's congestion, priority and express; readings)"),
    ("owner", "§2.7", "An owner MUST NOT hold more live members of ", MET,
     "owner.Owner._check_budget (budget_scenarios §4–§6)"),
    ("owner", "§2.7", "An owner MUST NOT publish more: `<n>` + 1 oc", MET,
     "owner.Owner.occur (budget_scenarios §7)"),
    ("owner", "R3", "**Owner:** the descriptor (§3.3) MUST list e", MET,
     "owner.Owner._descriptor, a self.system provider resolved (owner.resolve_providers; live_interop, "
     "bindings.md §3, §5)"),
    ("owner", "§3.2", "An owner whose configuration binds a require", MET,
     "owner.Owner._plan refuses before anything is declared (presence_scenarios §2)"),
    ("owner", "§3.3", "The owner's descriptor MUST NOT raise it.", MET,
     "owner.Owner._plan refuses a stated bound outside 1 to the contract's (round 89; readings)"),
    ("owner", "§3.3", "One member is used: an owner SHOULD state it", MET,
     "owner.Owner._descriptor's meta.zid, unless a test turns it off (state_zid)"),
    ("owner", "§3.3", "An owner SHOULD write `meta.zid` as zenoh wr", MET,
     "owner.Owner._descriptor writes str(session.zid())"),
    ("owner", "§3.3", "An owner's descriptor SHOULD stay within 1 K", NOT_MET,
     "zk2py neither checks nor warns: an owner of enough contracts writes a longer descriptor"),
    ("owner", "§3.3", "At the constrained level (§12), it MUST fit ", NA,
     "zk2py claims no class at the constrained level"),
    ("owner", "§3.3", "The owner MUST put the descriptor on its ins", MET,
     "owner.Owner._put_descriptor and its queryable (presence_scenarios §2)"),
    ("owner", "S1", "**Owner:** every state mutation, put **and**", MET,
     "owner.Owner.set_state and delete_state, minted (live_interop state.md §1)"),
    ("owner", "S2", "**Owner:** a GET reply MUST carry the timest", MET,
     "owner.Owner.answer_state (live_interop state.md §1; state_scenarios §10)"),
    ("owner", "S2", "It MUST answer a selector over them with eve", MET,
     "owner.Owner.state_answer, reply_del within the window (state_scenarios §10)"),
    ("owner", "S3", "**Owner:** for a key it deleted within the t", MET,
     "owner.Owner.delete_state and its tombstone window (state_scenarios §10)"),
    ("owner", "S7", "A session that serves state MUST enable Zeno", MET,
     "owner.Owner.open_session: timestamping on"),
    ("owner", "S7", "An owner MUST NOT stamp at or below the last", MET,
     "owner.Owner.mint (the tick) and catch_up from a record (state_scenarios, live_interop state.md §1)"),
    ("owner", "S7", "With neither source, the owner MUST start a ", MET,
     "every owner.Owner start is a new instance on a new session, its zid zenoh's random one"),
    ("owner", "S7", "An owner SHOULD NOT stamp beyond its router'", MET,
     "owner.Owner's heartbeat and clock guard (freshness_scenarios §2, health_scenarios §4)"),
    ("owner", "S7", "The owner SHOULD detect drift against a refe", MET,
     "owner.Owner._beat, a router-stamped heartbeat key (freshness_scenarios §2)"),
    ("owner", "S7", "When a detection shows it beyond the delta, ", MET,
     "owner.ClockAhead stops every state write, re-puts included, and Owner.events reports it "
     "(freshness_scenarios §2, health_scenarios §4)"),
    ("owner", "§4.4", "**An archive's backend MUST NOT accept a put", NA, "zk2py runs no archive"),
    ("owner", "§4.4", "When an owner becomes reachable again, the a", NA, "zk2py runs no archive"),
    ("owner", "§4.4", "An archive SHOULD repeat an alignment that l", NA, "zk2py runs no archive"),
    ("owner", "§4.4", "The archive MAY drop such a key after a depl", NA, "zk2py runs no archive"),
    ("owner", "§4.4", "An archive MUST hold its `archive.v1` interf", NA,
     "zk2py runs no archive; its owner refuses archive.v1 in a tokenless set (owner.Owner._plan, D011)"),
    ("owner", "O1", "**Owner:** an operation's queryable MUST be ", MET,
     "owner.Owner._declare_resources, complete=True (live_interop operations.md §1)"),
    ("owner", "O2", "**Owner:** a call whose key expression is no", MET,
     "owner.Owner._op_handler, before any handler (live_interop operations.md §2)"),
    ("owner", "O3", "**Owner:** a reply MUST go on the operation'", MET,
     "owner.OpCall.reply on the named member's key (live_interop operations.md §2, §3)"),
    ("owner", "O3", "Every call that reaches an owner's queryable", MET,
     "owner.OpCall.finish: internal, never silence"),
    ("owner", "O3", "The active instance of a service MUST answer", MET,
     "owner.Owner._unavailable_handler, with its cause"),
    ("owner", "O6", "With a declared `summary`, each replier MUST", MET,
     "owner.OpCall.summary: a summary owed at finish, a second refused and no value after it "
     "(round 89; readings)"),
    ("owner", "§5.1", "An instance whose exposed resources of an in", MET,
     "owner.replica_only: no unavailable queryable on an exclusive operation (round 89; readings)"),
    ("owner", "§5.1", "Replicas SHOULD expose the same replicated o", MET,
     "an owner.Owner exposes every resource it is not told to list or withhold, so replicas "
     "configured alike expose the same"),
    ("owner", "§5.1", "It names the member each reply answers for, ", MET,
     "owner.OpCall.name refuses a member the call does not select (live_interop operations.md §2)"),
    ("owner", "§5.2", "**The `Encoding` of the owner's reply** MUST", MET,
     "owner._refuse, the envelope's encoding (envelope.envelope_encoding)"),
    ("owner", "§5.2", "An operation that declares no `error` type h", MET,
     "owner.OpCall.refuse refuses a detail for it, nothing sent (round 89; readings)"),
    ("owner", "§5.2", "**A detail that does not fit** its envelope,", MET,
     "envelope.encode raises before anything is sent, and the call ends internal (errors family)"),
    ("owner", "§6", "An owner MUST NOT write under another servic", MET,
     "owner.Owner builds every key under its own prefix"),
    ("owner", "§6", "An owner SHOULD keep a re-mint's overlap bel", MET,
     "owner.Owner.new_epoch undeclares the old at once (presence_scenarios §3)"),
    ("owner", "§7.2", "An owner MUST set Zenoh's predefined `Encodi", MET,
     "owner._value_encoding and every publisher's encoding"),
    ("owner", "§7.2", "It MUST NOT add a schema suffix to it, excep", MET,
     "owner's encodings carry no suffix; envelope.PROTOBUF_ENCODING alone does"),
    ("owner", "§7.4", "A generated `@stream` publisher SHOULD accep", NA, "zk2py generates no publisher"),
    ("owner", "§8.1", "Every instance MUST hold one, including pure", MET,
     "owner.Owner._declare_instance (presence_scenarios §1)"),
    ("owner", "§8.1", "An owner MUST hold one per interface **of wh", MET,
     "owner.Owner, the tokenless set and a standby's none (presence_scenarios §1, §5; redundancy_scenarios §1)"),
    ("owner", "§8.1", "The owner MUST hold one member token per mem", MET,
     "owner.Owner.hold_member and release_member (presence_scenarios §3)"),
    ("owner", "§8.1", "An owner that starts a new instance id while", MET,
     "owner.Owner.new_epoch, make before break (presence_scenarios §3)"),
    ("owner", "§8.2", "An owner MUST bring itself up in this order,", MET,
     "owner.Owner._bring_up (presence_scenarios §1)"),
    ("owner", "§8.2", "A state's value is not a declaration, so an ", MET,
     "owner.Owner._put_held before the tokens (presence_scenarios §1)"),
    ("owner", "§8.4", "Every holder MUST declare a `complete` query", MET,
     "owner.Owner, a complete queryable per bundle (live_interop's retrieval)"),
    ("owner", "§12", "An owner's descriptor MUST fit one fragment ", NA,
     "zk2py claims no class at the constrained level"),
    # -- consumer ----------------------------------------------------------------------
    ("consumer", "R6", "**Consumer:** MUST discard a sample, or a GE", MET,
     "owner.Owner.subscribe_role, live.get_state, seed.LateJoiner (live_interop bindings.md §4)"),
    ("consumer", "R7", "**Consumer:** a binding MUST NOT require pre", MET,
     "owner.Owner.subscribe_role subscribes at once, without presence"),
    ("consumer", "§3.2", "A consumer's runtime MUST resolve a provider", MET,
     "owner.resolve_providers at start (live_interop bindings.md §5)"),
    ("consumer", "S4", "**Consumer:** a state GET MUST be addressed ", MET,
     "live.get_state and seed.LateJoiner: All and Latest (state_scenarios §3)"),
    ("consumer", "S5", "**Consumer:** last-known state MUST be read ", NA,
     "zk2py's consumers read no last-known state"),
    ("consumer", "S6", "The archive's answer is last-known, never cu", NA,
     "zk2py's consumers read no archive"),
    ("consumer", "§4.2", "A late joiner MUST declare its subscription ", MET,
     "seed.LateJoiner (state_scenarios §10)"),
    ("consumer", "§4.2", "A late joiner MUST merge the GET's replies a", MET,
     "seed.kept and seed.compare (seed family)"),
    ("consumer", "S7", "A consumer MUST order an owner's values by t", NOT_MET,
     "seed.LateJoiner orders by Latest's order, time then id, as core §4.2 says, never by time within "
     "one id (a finding); desired.TargetRole keeps desired.v1 §2.4's own order"),
    ("consumer", "S7", "It MUST accept the first value under a new i", NOT_MET,
     "seed.LateJoiner keeps an older epoch's later stamp over a new id's first value (a finding)"),
    ("consumer", "§8.1", "Every instance MUST hold one, including pure", MET,
     "every zk2py service is an owner.Owner, which holds one (presence_scenarios §1)"),
    ("consumer", "§11.1", "**History** (§2.5): a consumer's advanced su", NA,
     "zk2py's consumers declare no advanced subscriber (budget_scenarios §3's, a measurement, declares "
     "no detection token)"),
    # -- caller ------------------------------------------------------------------------
    ("caller", "O1", "**Caller:** a concrete call MUST use target ", MET,
     "live.call (live_interop operations.md §1)"),
    ("caller", "O2", "**Caller:** a call to a fan-out operation MU", MET,
     "live.call(fanout=True) (live_interop operations.md §2)"),
    ("caller", "O4", "**Caller:** MUST NOT retry an operation that", MET, "live.call never retries"),
    ("caller", "O5", "**Caller and tool:** MUST NOT treat an empty", MET,
     "live.CallResult.silent, live.o3_verdict (observer_scenarios, operations.md §5)"),
    ("caller", "O6", "**Caller:** MUST use consolidation `None`, s", MET, "live.call"),
    ("caller", "§5.1", "A caller MUST NOT depend on a refusal of wha", MET,
     "live.call sends what it is handed, and reads a refusal as an answer, never as a check"),
    ("caller", "§8.4", "A caller or tool MUST retrieve a bundle as f", MET,
     "live.retrieve_bundle (live_interop's retrieval, retrieval.md §2, §3)"),
    ("caller", "§8.4", "A caller MUST set consolidation `None` on bo", MET, "live._answers"),
    ("caller", "§8.4", "A caller MUST NOT depend on the encoding: th", MET,
     "live.retrieve_bundle verifies the hash, never the encoding"),
    ("caller", "§3.1", "A caller or tool's liveliness GET on a sessi", MET,
     "live.list_presence, a callback handler (live_interop presence.md §4)"),
    ("caller", "§3.1", "Every liveliness subscriber the caller or to", MET,
     "live.declare_liveliness_subscriber, callbacks only"),
    # -- observer ----------------------------------------------------------------------
    ("observer", "O5", "**Caller and tool:** MUST NOT treat an empty", MET,
     "judgement, live.o3_verdict (observer_scenarios, operations.md §5)"),
    ("observer", "§5.2", "It MUST refuse:", MET, "envelope.decode (errors family)"),
    ("observer", "§8.4", "A caller or tool MUST retrieve a bundle as f", MET,
     "live.retrieve_bundle (live_interop's retrieval, retrieval.md §2, §3)"),
    ("observer", "§11.1", "A generator MUST refuse an input that binds ", MET,
     "acl.Principal binds by one name: no input can bind one by both"),
    ("observer", "§11.1", "Under `deny`, a generator MUST NOT compile a", MET,
     "acl, admin_records (security_scenarios §4)"),
    ("observer", "§11.2", "A generator MUST therefore add to a provider", MET,
     "acl.compile_grants (acl_interop security.md §2)"),
    ("observer", "§11.2", "A generator MUST grant the consumer selector", MET,
     "acl.compile_grants (acl_interop security.md §2)"),
    ("observer", "§11.2", "A generator MUST deny, in each principal's p", MET,
     "acl.compile_grants (acl_interop security.md §2)"),
    ("observer", "§3.1", "A caller or tool's liveliness GET on a sessi", MET, "live.list_presence"),
    ("observer", "§3.1", "Every liveliness subscriber the caller or to", MET,
     "live.declare_liveliness_subscriber"),
    ("observer", "§3.2", "A tool that decides a fault from the shape o", MET,
     "live.presence_faults, two reads a grace apart (observer_scenarios, operations.md §8)"),
    ("observer", "§3.3", "A tool SHOULD treat a liveliness GET that en", MET,
     "live.list_presence (presence_scenarios §6)"),
    ("observer", "§3.3", "A reader MUST read as possibly incomplete a ", MET,
     "live.list_presence's at_interests_timeout (presence_scenarios §10)"),
    ("observer", "§3.3", "A reader MUST read as not standing a round t", MET, "live.settle (presence_scenarios §10)"),
    ("observer", "§3.3", "A reader whose reads may wait longer SHOULD ", MET,
     "live.open_client, a 120 s interests timeout"),
    ("observer", "§3.3", "A reader that cannot read its session's inte", MET,
     "live.open_client opens its readers' sessions and records the value"),
    ("observer", "§3.4", "Where it cannot rule out a refusal, it SHOUL", MET,
     "live.Presence.reading: 'absent, or refused by access control'"),
    ("observer", "§3.5", "A reader MUST NOT conclude absence from it, ", MET,
     "live.list_presence, presence.PresenceView (presence_scenarios §7)"),
    ("observer", "§3.5", "A reader SHOULD NOT make such a read.", MET,
     "live.list_presence: not made (presence_scenarios §7)"),
    ("observer", "§3.6", "A reader MUST NOT conclude absence from a wi", MET,
     "presence.PresenceView (presence_scenarios §8)"),
    ("observer", "§3.6", "A reader that follows presence live SHOULD r", MET,
     "presence.PresenceFollower (presence_scenarios §8)"),
    ("observer", "§3.6", "The reader SHOULD report what that read esta", MET,
     "presence.PresenceFollower (presence_scenarios §8)"),
    ("observer", "§3.7", "A reader SHOULD make every liveliness read i", MET,
     "live.list_presence refuses a session that held a subscriber (presence_scenarios §9)"),
    ("observer", "§3.7", "The reader SHOULD hold its liveliness subscr", MET,
     "presence.PresenceFollower's watch session (presence_scenarios §9)"),
    ("observer", "§3.7", "A reader that seeds on one session and follo", MET,
     "live.Barrier (presence_scenarios §9, §10)"),
    ("observer", "§3.7", "**The subscribers of a reader's watch sessio", MET,
     "presence.PresenceFollower, no history (presence_scenarios §10)"),
    ("observer", "§3.7", "A reader that follows presence in a namespac", MET,
     "live.wire_selector, live.Stripped (presence_scenarios §10)"),
    ("observer", "§3.7", "A reader that keeps a namespaced watch sessi", MET,
     "presence.PresenceFollower handed a namespaced watch: live.flight_timeout and a 120 s interests timeout"),
    ("observer", "§3.7", "A reader that follows presence live SHOULD m", MET,
     "presence.PresenceFollower's transport events (presence_scenarios §10)"),
    ("observer", "§3.7", "So a reader that holds back what a move make", MET,
     "arbitration_live, the move begun at a withdrawal with no link (arbitration_scenarios §7)"),
    ("observer", "§3.7", "A reader SHOULD ask the round trip after an ", MET,
     "arbitration_live.open_margin_s; presence.PresenceFollower's attempt 0.1 s after a move "
     "(arbitration_scenarios §7)"),
    ("observer", "§3.7", "After a move, a reader SHOULD wait for the r", MET,
     "presence.PresenceFollower's move_wait, until it ends (presence_scenarios §10)"),
    ("observer", "§3.7", "It SHOULD run its interests timeout above th", MET, "live.open_client, 120 s"),
    ("observer", "§3.7", "A reader SHOULD NOT ask a round trip while o", MET,
     "live.Barrier, one in flight (presence_scenarios §10)"),
    ("observer", "§3.7", "It SHOULD NOT ask a read while one it asked ", MET,
     "live.Flight (presence_scenarios §10)"),
    ("observer", "§3.7", "It SHOULD NOT make a read that it would foll", MET,
     "presence.PresenceFollower._watch_cut_off (presence_scenarios §10)"),
    ("observer", "§3.9", "A reader that concludes absence of a token a", MET,
     "live.open_client: every reader is a client of a router (redundancy_scenarios §5)"),
    ("observer", "§4.3", "A tool MUST NOT ask a selector that intersec", MET,
     "live.admin_guard (security_scenarios §4, state_scenarios §12)"),
    ("observer", "§4.4", "A tool MUST compare two zids by value, never", MET,
     "live._zid_value (timing family, lifespans.json)"),
    ("observer", "§6.1", "A tool's grace period SHOULD exceed the long", MET,
     "live.SHAPE_GRACE_S, 1 s, above an owner's overlap below one second"),
    ("observer", "§7.1", "Where nothing crosses, liveness is *unobserv", MET,
     "timing_tool and telemetry_scenarios across a face: owners unseen, never down (link_scenarios §6)"),
    ("observer", "§7.1", "A tool that is not a service MUST refuse a `", MET,
     "owner.resolve_providers with no system raises"),
    ("observer", "§7.3", "A generic tool MUST decode `protobuf` (proto", NOT_MET,
     "echo_tool shows each sample's key, kind, size and encoding, and decodes none (types.md §2 not run)"),
    ("observer", "§7.3", "A tool MUST render a kind it cannot decode a", NOT_MET,
     "echo_tool reads no contract, so it shows no declared type"),
    ("observer", "§7.3", "It MUST NOT show garbage, and MUST NOT drop ", MET,
     "echo_tool shows no payload bytes and drops no sample (observer_scenarios)"),
    ("observer", "§7.4", "A tool SHOULD check the limit at start: the ", MET,
     "live.memlock_said, at the start of timing_tool, arbitration_tool and echo_tool (round 89)"),
    # -- static, in every class --------------------------------------------------------
    ("static", "§1.1", "An implementation MUST refuse to build one, ", MET, "keys family"),
    ("static", "§1.1", "Every key it accepts MUST build back to the ", MET, "keys family"),
    ("static", "§1.3", "Two consequences follow, and an implementati", MET,
     "templates family, observer_scenarios (grammar.md §1)"),
    ("static", "§1.4", "Both MUST be implemented exactly as follows.", MET, "slugs family"),
    ("static", "§1.4", "The decoder MUST accept exactly the canonica", MET, "slugs and templates families"),
    ("static", "§1.6", "Application code SHOULD NOT spell a deployme", MET,
     "keys are base-relative; a namespace is the session's (live.open_client)"),
    ("static", "§2.2", "**Within one interface,** two templates unde", MET, "contracts family (E021)"),
    ("static", "§2.2", "A template with parameters MUST declare `car", MET, "contracts family (E013)"),
    ("static", "§2.2", "A template without parameters MUST NOT decla", MET, "contracts family (E014)"),
    ("static", "§2.2", "A contract MAY write the number instead: the", MET,
     "contracts family (ok-cardinality-unbounded, ok-cardinality-no-ceiling)"),
    ("static", "§2.4", "**A contract's state and events** SHOULD kee", MET,
     "every state and event of zk2py's own contracts (interop/) keeps reliable and block; no lint has a code for it"),
    ("static", "§2.4", "A contract's author SHOULD set it only on me", MET,
     "no contract zk2py wrote sets express (arbitration/steer.v1 does, copied from its scenarios)"),
    ("static", "§2.5", "A contract MUST NOT declare `history` on an ", MET, "contracts family (E019)"),
    ("static", "E026", "The seconds, `<n>` times the unit's, MUST fi", MET, "contracts family (E026)"),
    ("static", "E028", "Like every canonical integer, they MUST also", MET, "contracts family (E028)"),
    ("static", "§3.1", "`resources` names resources of the required ", MET, "contracts family (E030)"),
    ("static", "§3.3", "Its schema is [`descriptor.schema.json`](des", MET, "descriptors family"),
    ("static", "W102", "A request type SHOULD NOT repeat a template ", MET, "contracts family (W102)"),
    ("static", "E024", "The name MUST be unique across the listed fi", MET, "contracts family (E024)"),
    ("static", "§9.1", "A contract MUST NOT need later TOML.", MET, "contracts family (the five e000-toml11 cases)"),
    ("static", "§9.2", "An implementation MUST report, for each fixt", MET, "contracts and sets families"),
    ("static", "§9.3", "A defaulted contract and the same contract w", MET, "contracts family"),
    ("static", "E024", "Its `name` is the file's stem, the last path", MET, "contracts family (E024)"),
    ("static", "E024", "The id MUST be unique among the listed files", MET, "contracts family (e024-identical)"),
    ("static", "E024", "Otherwise it resolves as a path relative to ", MET,
     "contracts family (E032: e032-ref-outside)"),
    ("static", "§9.4", "The pointer MUST resolve.", MET, "contracts family (E032)"),
    ("static", "§9.5", "An implementation MUST produce, for every va", MET, "contracts family"),
    ("static", "§9.6", "To verify bundle bytes, an implementation MU", MET, "bundles family"),
    ("static", "§9.7", "**The history check** MUST verify:", MET, "history family"),
    ("static", "§9.8", "**Breaking:** a contract's CI MUST NOT publi", NA,
     "zk2py is no contract's CI and publishes no revision; its compat family classifies"),
    ("static", "§9.8", "**Review:** a human MUST accept it before pu", NA,
     "zk2py publishes no revision; a human is no implementation"),
    ("static", "§9.8", "The classifier MUST classify every case of `", MET, "compat family"),
    ("static", "§10", "A contract that uses one MUST list the profi", MET, "contracts family (E020, W105)"),
]

#: Rules of the texts that bind a claimed class and that ``RULES.md`` does
#: not list under it: (class, where, the rule, status, what shows it, the
#: finding).
BEYOND: list[tuple[str, str, str, str, str, str]] = [
    ("observer", "core §4.2", "A late joiner MUST declare its subscription first, then make S4's GET "
     "(a late joiner is \"a consumer or a tool\")", MET, "seed.LateJoiner, desired_tool's reads", "F-361"),
    ("observer", "core §4.2", "A late joiner MUST merge the GET's replies and the subscription's samples per key "
     "by the owner's stamp", MET, "seed.kept (seed family)", "F-361"),
    ("observer", "core §8.4", "Every holder MUST declare a complete queryable on it (any participant MAY hold "
     "bundles)", NA, "no zk2py tool holds bundles", "F-361"),
    ("caller", "core §8.1", "Every instance MUST hold one [an instance token]: a caller that is a service", MET,
     "owner.Owner", "F-361"),
    ("observer", "observer §1", "The judgement shape: the two Unestablished kinds apart in every medium, not "
     "asked never answered no, not this profile's never folded, an empty scope unobservable", MET,
     "judgement (budget, freshness, telemetry, desired, timing, arbitration and redundancy families)", "F-362"),
    ("observer", "observer §2", "Silence is never a verdict: unattributable silence is Unestablished", MET,
     "judgement, live.o3_verdict (observer_scenarios)", "F-362"),
    ("observer", "observer §5", "A window's readings and conclusions (lossless, the watch after a window)", MET,
     "budget (budget family, budget_scenarios)", "F-362"),
    ("observer", "observer §7.1", "The data-flow graph is read from descriptors, never inferred", MET,
     "live.get_descriptor (live_interop bindings.md §3)", "F-362"),
    ("observer", "observer §8", "What a tool tells a person: a foreign key a fact, a wildcard scope's blind "
     "spots, who stamped a timestamp", MET, "echo_tool, timing_tool, live.blind_spots (observer_scenarios)",
     "F-362"),
    ("static", "core §2.2", "A rest parameter MAY appear only as the last segment (E012)", MET,
     "contracts family (E012)", "F-363"),
]


def _rules_md(spec: Path) -> list[tuple[str, str, str]]:
    """``RULES.md``'s entries as (class, rule column, sentence), in its
    order, the rule-id table left out."""
    out, role = [], None
    for line in (spec / "RULES.md").read_text(encoding="utf-8").splitlines():
        m = re.match(r"^## (.*)", line)
        if m:
            role = HEADINGS.get(m.group(1))
            continue
        m = re.match(r"^\| \[([^\]]+)\]\([^)]+\)(?:, \*reader\*)? \| (.*) \| .* \|$", line)
        if m and role is not None:
            out.append((role, m.group(1), m.group(2)))
    return out


def _start(sentence: str) -> str:
    """A sentence's start after a run-in label that ``RULES.md`` cut with
    "…", the key :data:`WALK` holds."""
    return sentence.split(" … ")[-1]


def check(spec: Path) -> list[str]:
    """What keeps :data:`WALK` and ``RULES.md`` in step: every entry of a
    claimed class or of the static rules walked once, in order, and no row
    that walks nothing. Empty when they agree."""
    entries = [e for e in _rules_md(spec) if e[0] in CLAIMED or e[0] == "static"]
    problems = []
    if len(entries) != len(WALK):
        problems.append(f"RULES.md has {len(entries)} entries of the claimed classes and the static rules; "
                        f"the walk {len(WALK)}")
    for (role, rid, sentence), row in zip(entries, WALK):
        if (role, rid) != row[:2] or not _start(sentence).startswith(row[2]):
            problems.append(f"RULES.md's {role} {rid} {_start(sentence)[:44]!r} walked as {row[:3]}")
            break
    for row in WALK:
        if row[3] not in (MET, NOT_MET, NA) or not row[4]:
            problems.append(f"{row[:3]}: status {row[3]!r} without what shows it")
    return problems


def counts() -> dict[str, dict[str, int]]:
    """Per claimed class, its own entries' statuses, and the static rules'
    beside them (in every class)."""
    out: dict[str, dict[str, int]] = {}
    for cls in [*CLAIMED, "static"]:
        rows = [r for r in WALK if r[0] == cls]
        out[cls] = {s: sum(1 for r in rows if r[3] == s) for s in (MET, NOT_MET, NA)}
    return out


def table() -> str:
    """The walk as a Markdown table."""
    lines = ["| Class | Rule | Sentence | Status | What shows it |", "|---|---|---|---|---|"]
    for cls, rid, start, status, why in WALK:
        lines.append(f"| {cls} | {rid} | {start.replace('|', '\\|')}… | {status} | {why.replace('|', '\\|')} |")
    return "\n".join(lines)


if __name__ == "__main__":
    spec = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[3] / "spec"
    bad = check(spec)
    for b in bad:
        print(b, file=sys.stderr)
    print(table())
    for cls, c in counts().items():
        print(f"{cls}: " + ", ".join(f"{v} {k}" for k, v in c.items()), file=sys.stderr)
    sys.exit(1 if bad else 0)
