# `redundancy.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.6 — 2026-10-30: editorial: inline history taken out of the rules and long sentences split, no rule changed (#935)

The rules carried their own history, as the core's did before core 0.60
(#935, epic #921 Track B). A rule named the version that made it and the
finding that caused it, as in "(0.2, F-204)" or "(core §8.1, 0.37, 0.38)".
This changelog and the core's already hold that history. So the text now
states each rule and cites its evidence, and nothing else. This is #935's
fifth part, beside `timing.v1` and `arbitration.v1`.

**Editorial: inline history taken out of the rules, no rule changed.**
- **This text's versions and findings,** "(0.2)" to "(0.4, F-221)". Most
  opened a run-in label, which now stands alone:
  - §2.5's "Its read goes on a session of its own", "Where its sessions are"
    and "What it hears while a read runs";
  - §2.7's "A poll, where no watch session can be held", and §2.8's "When a
    departure is seen";
  - §2.9's "A heal read that ends nothing" and "Its own re-mint is no
    appearance";
  - §2.15's "One split, one finding", "A standby's exposure, probed" and
    "Its wildcards".
  The others sat after a sentence. In §0, §2.4 and §2.6: the read made
  off the claim's session, the refused write, and the state from an
  earlier incumbency. In §2.9 and §2.11: the step-down's writes, the
  `external` split, the lock regained and two locks. In §2.15: the tool's
  sessions, a contract not read, the offline contract, `hostid.v1`'s
  standby and "Never a call that would run".
- **A history told as a tag.** §2.7's "(0.2; a MUST at 0.1)" after the
  watch's SHOULD is gone: the rule is the SHOULD.
- **The core's versions.** "(core §3.3, 0.39)", "(core §8.1, 0.37, 0.38)",
  "(core §8.1, 0.40)", "(core §6, 0.39; §2.15)" and the like cite the
  section alone, in §0's terms, §1, §2.3, §2.4, §2.5, §2.7, §2.8, §2.9,
  §2.14 and §2.15. §0's "Read" loses two "(0.37, 0.38)", a "(0.38)" and a
  "(0.37)", and keeps every condition of its completeness. Its "Watch
  session" loses a "(0.37)". "(core §8.1, 0.38: a profile's premise)" keeps
  its reason, as "(core §8.1: a profile's premise)", in §2.5 step 3 and
  §2.15. §1's and §4's "core Appendix D (core 0.39)" are "core Appendix D",
  and §2.14's "(core 0.39)" after `archive.v1` is gone.
- **The core's labels, for its versions.** §2.6's "(core 0.39, "serving
  later")" and §2.9's "(core 0.39, "stopping")" now name the item they
  meant: "(core §8.2, "Serving later, and stopping")".
- **`hostid.v1`'s versions.** §2.15's "(its text 0.6, its reads of the
  address's `@zk/**` since 0.7)" is gone: `hostid.v1` §2.12 is cited.
- **ZenSight's issues.** §2.5's "(ZenSight #1104)" and "(ZenSight
  #1105)", and §2.10's "(#1104)", now point to Appendix A, which names
  both issues and what each fixed. Appendix A keeps its own.
- **The scenarios.** The opening names the reference's tests without "#841,
  RB" and "#841, RC". The conventions lose "(0.2, F-204)", "(core §8.1,
  0.37)", "(0.2, F-202)", "(0.38)", "(0.2, F-201)", "(#841, RB)", "(0.4,
  F-220)", "(0.3, F-216)" and "(0.4, F-221)". §1's Expected 1 and 4, §3's
  step 1 and Expected 1, §5's setup and Expected 1 to 3, §7's Expected 4,
  §10's steps 3 and 5 and Expected 4, §11's Expected 1 and 2, and §12's
  setup, steps 8 and 9 and Expected 6 to 9 each lose a tag. §4's Expected
  2's "(ZenSight #1104)" is "(v1.md Appendix A)". No heading held a tag.

**The long sentences, split.** The nine over 50 words in this text:
- **§2.5, "Where its sessions are",** is its MUST, then the complete read's
  reach, then the peer that takes over beside a live incumbent.
- **§2.5, "What it hears while a read runs",** is the events kept, then how
  each is decided, as a list of its three. The MUST that follows is a
  paragraph of its own, its scope unchanged.
- **§2.7, the poll,** is the read every period, its MUST, then the
  appearance and the withdrawal.
- **§2.15:**
  - "Completeness" is the read not made, its answer, and the reason the tool
    reports;
  - premise 3 is a list of its three answers, the offline contract a
    paragraph after it;
  - the first question's fourth step is a list of its five answers, in their
    order;
  - `hostid.v1` §2.12's standby is the answer, then what the instance is;
  - "A standby's exposure, probed" is its MAY, then why a reply is a
    queryable, then that the finding is the tool's.
- **Appendix A's record** is two sentences, the second opening "Architecture
  r3.2's".

The scenarios' six:
- **The conventions' crash** is the two ways, then each way's meaning.
- **§1's Expected 1** is its claim, then the GET at the instance token, then
  the one at the interface token.
- **§3's Expected 1:** the GET at the instance token, then each instance's
  first read.
- **§5's setup:** the peers and why, then their retry period.
- **§7's Expected 4:** what is dated before the withdrawal, then what
  follows it.
- **§10's Expected 3:** V2's takeover, then G's reads.

**Nothing moved to an appendix.** No rule held a measurement. §2.8's lease,
"10 s by default", and the reference's "0.1 s after the first" carry no tag,
and stay.

**How "no rule changed" was checked**, as for core 0.60 and 0.61, with the
tool of #935's third part.
- **The method.** Every sentence holding MUST, MUST NOT, SHOULD, SHOULD NOT,
  MAY, REQUIRED or RECOMMENDED was extracted before and after. Each was
  keyed by section, and the two sets diffed. The scenarios' Expected lines
  were diffed too, keyword or not.
- **This text.** Both sets hold 40. 30 are unchanged and four lost a tag
  only. The other six keep their keyword, condition and scope:
  - §2.5's sessions MUST, §2.7's poll MUST and §2.15's probe MAY, each split
    from the words after it;
  - §2.6's and §2.9's MUSTs, whose core tag is now the core's label;
  - §2.7's SHOULD, which lost "a MUST at 0.1": the tool counts that
    history's MUST as a keyword.
- **The scenarios.** 68 keyword sentences and Expected lines before, and 73
  after. 49 are unchanged and 14 lost a tag only. Five were split, and their
  five new sentences are the rest of the old ones. §4's Expected 2 points to
  Appendix A for ZenSight's issue.
- **The rest.** Every other sentence and table row was diffed too, and each
  rewrite read beside its old text. None moves a condition, an object or a
  scope. No fixture moved.

**The figures.**
- **This text:** tags from 58 to none, sentences over 50 words from 9 to
  none, and a mean from 21.45 words to 19.73.
- **`scenarios.md`:** tags from 29 to none, sentences over 50 words from 6
  to none, and a mean from 18.1 to 16.93.

`spec/.prose-baseline.json` is lowered to them.

**Deliberately not changed.**
- **No rule**, and no section number, label or anchor.
- **Appendix A**, which is history: its ZenSight issues and its "Core 0.37"
  stay.
- **Written against core 0.40**, still.

## 0.5 — 2026-10-25: the offline half of §12 step 8 (#841, RD)

zk2py round 63 read 0.4 cold at PR #867's head: every check passed in
all four pairings, F-220 to F-223 resolved, the two tools agreeing
everywhere. It filed F-226, minor, accepted. Core stays 0.40.

**Changed.**
- **§12 gains step 9** (F-226). §2.15's "Never a call that would run"
  guards a probe by every revision the address's descriptors name **or**
  the tool holds offline, and cites `[Sc: §12]`. Step 8 runs only the
  first half: `ledger-appendall` running at the address. A tool that
  guarded by the named revision alone passed every step. Step 9 runs the
  second: `ledger.v1` at the address, `ledger-appendall` held offline,
  named by no descriptor. T makes no `append$*` probe, names
  `appendall` and the revision held offline, and reads the standbys'
  exposure unobservable. The reference runs it
  (`redundancy_s12_a_probe_guarded_by_a_revision_held_offline`), and
  zk2py already ran the shape. The MUST's evidence, `[Sc: §12]`, now
  covers both halves.

**Deliberately not changed.** The rule itself, its wording (0.4), and
every fixture.

## 0.4 — 2026-10-25: round 60's answers, and the review of the tools (#841, RD)

zk2py round 60 (PR #863) read 0.3 cold and ran §12 against the reference
tools in four pairings: 2959 of 2959 static checks, every live check,
the two tools agreeing on every shape. It filed F-220 to F-223, all
accepted on #841; the review of RC (PR #862) asked two more of the text's
rules. Core stays 0.40 and `hostid.v1` 0.7.

**Changed: round 60's answers.**
- **The owner example's `--wait-go` starts at `go`** (scenarios'
  conventions; F-220). It read standard input only once its service was
  up, so `go` never arrived. It reads from its own start; a line before
  `go` is refused. `[Sc: §3]`
- **A probe's wildcards, and a call that would run** (§2.15; F-221). A
  template-less operation's wildcard is its name followed by `$*`, which
  reaches every operation whose name begins with it: `append$*` reaches
  `appendall`. A wildcard that intersects the key of an operation whose
  fan-out is allowed, in any revision of the interface the tool holds
  that the address's descriptors name or that it holds offline, MUST NOT
  be asked, since the call would run it (#862's review asked for every
  revision: a minor can add such an operation). It is now a MUST of a
  tool that probes, where 0.3 said only that an allowed operation is
  never probed itself. What a probe can still reach, an operation of a
  revision the tool does not hold, is said. `[Sc: §12]` (step 8, on
  `ledger-appendall`, new in the conventions)
- **A skipped case sends nothing** (F-222): a tool's, not the text's.
  `zenctl check conform --skip` called an idempotent operation with every
  case skipped; it now calls one only when `operation` is asked, probes a
  fan-out only when `fanout-refused` is, and a standby only when
  `redundancy-standby` is.
- **"Before the token" within a round trip** (scenarios §1 and §3
  Expected 1; F-223). A takeover whose step 3, the unmarked descriptor,
  lands within the round trip of S's GET at the instance token is read
  unmarked just before S hears the interface token, the answer and the
  token riding two priorities, as F-202 found of a put. The exception now
  reads: unless it took over before that GET was answered, its interface
  token at S within a round trip of the answer. `profile_redundancy.rs`'s
  §1 and §3 check exactly that. `[Sc: §1, §3]`

**Changed: the review of the tools.**
- **`gen`'s members of a governed interface** follow the election's own
  watch (`Redundancy::subscribe`), never a poll of the role: each change
  declares, while the interface serves, each member token the instance
  does not hold, and a declaration that failed for another reason than
  its standing is made again. A member held is never declared twice: the
  runtime cycles a member declared again (core §8.1), a new epoch. The
  runtime gains `Service::holds_member` to ask.
- **zengui's reading on selection** waits for the selection to settle
  (300 ms): a burst of selections asks one reading, the last's.

**The fixtures.** Unchanged.

**Decided here, where the record was silent.**
- **The revisions a probe is guarded by** are the one under test, which
  every descriptor at the address names (the reference's `check conform`
  tests the one revision a service claims), and every revision of the
  interface held offline: `--contracts` is where an operator puts a minor
  the bus does not yet run.
- **A probe that is not made is said**, each with its reason, in the
  case's answer; a case whose only probes were not made is unobservable,
  never a pass.
- **`run_redundancy` seeds nothing**: its store comes seeded by its
  caller (zenctl's `zk2::store`, zengui's window store), one path.
- **The settle is 300 ms**, a selection's, from the selection; "read
  again" asks at once.

**Deliberately not changed.**
- **The doctor takes no `--contracts`**, so over a test's vacancy whose
  standbys serve no bundle, `redundancy-vacancy` is unobservable, where
  `zenctl redundancy --contracts` answers. Real standbys serve their
  bundle (§2.4); round 60 noted it, and it stays.
- **Every fixture**, and §5's answers.

## 0.3 — 2026-10-25: what the tools showed, and round 58's answers (#841, RC)

The reference tools are built (#841, RC): `zenkey-fleet`'s
`bus|model|judge|report::redundancy` (§5's four questions from two reads,
every answer `zenkey_model::redundancy`'s), `zenctl redundancy`, the
doctor's `split-brain`, `token-missing` and `hostid-duplicate` reading the
profile and its new `redundancy-vacancy`, `check conform`'s
`redundancy-declaration` and `redundancy-standby`, `gen` and `serve` as
candidates, zengui's nodes card and Inspector, and `scenarios.md` §12 as
`zenctl/tests/live_zk2.rs`'s `redundancy_s12_…`. zk2py round 58 (PR #860)
ran 0.2 live against the runtime in every pairing, all passing, and filed
F-214 to F-216, all accepted on #841. Core stays 0.40 and `hostid.v1` 0.7.

**Changed: round 58's answers.**
- **The owner example checks before it writes** (scenarios §11; F-214).
  Its §11 refusal was the standby-write error, since it put its raw state
  before the runtime read the declaration. It now writes nothing of a
  contract listing `redundancy.v1` before its start, so the refusal is
  `not started: … invalid: value`, as §11 Expected 2 now says.
  `[Sc: §11]`
- **A poll compares with the last's** (§2.7; F-215), in zk2py's wording:
  a read not complete removes no holder from the last's; a holder it
  shows that is new is an appearance, and only a complete read can show
  one gone. 0.2's "a complete read with no other holder, after one with
  some" left the last's undefined after a read not complete. It is a
  MUST, stated by a new fixture. `[F: polls.json]`
- **§3's start, measured** (scenarios §3; F-216): at S, by each instance
  token's arrival; a run whose two tokens arrive more than 10 ms apart is
  made again and counted apart. The owner example gains `--wait-go`, and
  `redundancy zid` and its poll's log from the round's stdin gaps.
  `[Sc: §3]`

**Changed: what building the tools showed.**
- **An invalid declaration on the bus** (§2.15 premise 3). An instance
  running under one breaks §2.2's refusal, and the model's governance has
  no token for it: it reads unobservable, `no_contract`, the reasons named
  beside it. `[F: conclusions.json]` (the token) `[F: declarations.json]`
  (the reasons)
- **Governance from a contract held offline** (§2.15 premise 3). In a
  vacancy whose standbys are gone, and in X's shapes, no instance serves
  the bundle, so retrieval answers nothing (core §8.4) and the answer is
  `no_contract`; a tool holding the contract offline reads it. §12's T
  holds `ledger.v1` so. `[Sc: §12]`
- **One split, one finding** (§2.15). The doctor judged core §6's
  split-brain by exposure, and a governed split whose second holder was
  marked standby (core 0.39: still a holder, exposing nothing) fell
  between the two rules. On a governed interface the first question
  decides, whatever the marks, and the finding is reported once.
  `[Sc: §12]`
- **A standby's exposure, probed** (§2.15, a MAY). A GET over a
  fan-out-forbidden operation's wildcard reaches every operation
  queryable before any handler runs (core O2), so its replies, attributed
  by the replier's zid, find a queryable a marked instance holds; a state
  queryable holding nothing answers nothing, so silence proves less.
  Measured: across a router, zenoh 1.10.1 attributes each reply to the
  answering session. `[Sc: §12]` (step 8, new)
- **A recognised standby, named** (§2.15, `hostid.v1` §2.12). §12 step 6
  said the tool speaks "beside that finding"; a recognised standby is not
  counted, so there is no finding, and the tool names it beside its
  answer. `[Sc: §12]`

**The fixtures.** `polls.json` is new: 7 cases, each a poll's reads in
order -> each read's appearances and withdrawals, F-215's two sequences
among them. Its expectations were written by hand before
`zenkey_model::redundancy::poll` ran them, and the runtime's poll now
goes through that function. No other fixture changed.

**Decided here, where the record was silent.**
- **`zenctl redundancy`'s exit contract.** 1 is a finding: a split, a
  twin (`unmarked`, `marked_holding`, `unlisted`, a split's holders), or
  a lasting vacancy, §5's "Finding on" for each, since a vacancy in both
  reads is the election stuck or no instance holding the lock, which an
  operator acts on as on a split. 0: every governed interface asked has
  exactly one incumbent, and every instance is the incumbent, a standby
  or no candidate. 2 otherwise, `vacant` (a takeover due) included, and
  an interface not governed asked by name.
- **The doctor's `redundancy-vacancy`**, a warning: §2.15 asks for a
  lasting vacancy's finding, and no core check had one. A twin needs no
  check of its own: `split-brain` holds a split, `token-missing` a marked
  holder or an unlisted one, and an unmarked listing with no token is
  `token-missing`'s exposure no token covers. A vacancy in the second
  read only is a takeover due, no finding. An interface whose governance
  cannot be read is asked only when its shape would be the finding.
- **`hostid-duplicate` and a standby question unobservable.** `hostid.v1`
  0.6's unobservable case, applied: an instance listing only governed
  interfaces, or ones whose governance cannot be read, for which the
  question is unobservable, leaves the check unjudged unless the finding
  is established anyway.
- **`check conform`'s cases**, each a rule one run checks:
  `redundancy-declaration` (§2.2, §2.14: a running instance of an
  invalid or tokenless declaration) and `redundancy-standby` (§2.4: the
  probe above, and a marked holder of an interface or member token in
  both reads; a pass needs an operation probed). Left out: an operation
  run on the incumbent alone (the probe covers a standby's queryable),
  a standby writing nothing (no reader tells its writes from the
  incumbent's by key; `state-stamp` reads the stamp id), the election's
  own order (§2.5, §2.6, §2.9: a candidate's sequence, the scenarios'
  §1 to §7), and publishers (no reader sees one).
- **`gen` and `serve` elect.** A mock owner of a governed contract is a
  candidate on a reader and a watch of its own; a governed state's first
  value goes at its takeover, a write refused while it stands by is
  counted apart, never a failure, and its members are declared at each
  takeover. No `--standby` flag: no test needed one. The address guard is
  unchanged: a candidate that takes over writes synthetic data at the
  address, so a mock beside a live instance still needs `--i-know`.
- **zengui** asks the reading on an address's selection, beside its
  health, never polled; each role is a chip off the fleet's judge, and
  the descriptor's mark is shown as the descriptor's word, apart from it.

**Deliberately not changed.**
- **`elections.json`, `heals.json`, `declarations.json`,
  `descriptors.json`, `lifecycle.json`, `conclusions.json`.**
- **Core and `hostid.v1`.** Their texts already say what the tools read:
  core 0.39's marked holder, 0.40's complete read, hostid 0.7's reads.
- **No probe of a state queryable as proof.** The text names it weaker;
  making it a MUST would make every tool's silence a pass.

## 0.2 — 2026-10-25: what the runtime showed, and round 56's answers (#841, RB)

The reference runtime is built (#841, RB): `zenkey::redundancy`, through
`ServiceBuilder::redundancy`, driven by `zenkey_model::redundancy`'s
`Elector`, the owner example's `--standby`, and `scenarios.md` §1 to §11
as `zenkey/tests/profile_redundancy.rs`, one test per section. The second
implementation read 0.1 and core 0.39 cold (zk2py round 56, PR #855) and
filed F-199 to F-209, all accepted on #841. This text writes those
answers, with what building the runtime showed the text did not say. Core
goes to 0.40 (two zenoh facts, measured in Rust first) and `hostid.v1` to
0.7 (F-208).

**Changed: round 56's answers.**
- **What is heard while a read runs** (§2.5, §2.9; F-199). An appearance
  or a withdrawal heard while a read is asked is kept, as one heard while
  an act runs already was, and decided at that read's decision, in the
  state it leaves. A heal's read not complete that shows fewer than two
  holders ends nothing: it is made again after the retry interval, and only
  a complete read showing one holder ends a heal. `[F: lifecycle.json]`
- **`external` never heals, shown** (§2.9, §2.11; F-200). §10's step 4,
  two lock files and a sequential start, now expects what §2.11 says: the
  later instance reads the earlier's token and stands by on it. A new step
  5 starts both behind two routers whose link is cut: both serve, and once
  it is restored neither steps down within 10 s, and G's two reads read a
  split. §2.11 gains "Two locks". `[Sc: §10]`
- **A crash seen at once, or at the lease** (§2.8; F-201). A crash whose
  host's kernel closes its links is seen at once; a lost host, or a
  partition, at the lease. §1 and §10 step 3 make the links silent before
  the kill, with a short lease, so that "no read before the withdrawal" is
  tested at all, and run again without the silence. Core Appendix B
  already said both. `[Sc: §1, §10]`
- **"Before the token" observed by a GET** (scenarios; F-202). zenoh
  carries a token's declaration and a put on two priorities, so a
  subscriber's receipt order is not the owner's. The scenarios observe the
  order as core `presence.md` §1 does, by a GET of the descriptor the
  moment S hears each token, and never judge receipt order: the
  Conventions, §1 Expected 1, 2 and 4, §3, and §7 Expected 4, whose
  step-down order is observed by the stamps of what B put.
- **§3's third outcome** (F-203). One serves and the other stands by: A,
  unless B took over before A's read, A then standing by on B, as §2.5
  step 2 says. `[Sc: §3]`
- **The read on a session of its own** (§2.5 step 1, §0; F-204). The read
  goes on a session that holds neither a liveliness subscriber nor the
  claim, after one round trip on the claim's session: a session answers
  its own tokens itself, even through a router that refuses the read
  (core 0.40 §8.1, measured), so a read on the claim's session never
  shows the refusal step 3's own-claim check is for. A candidate runs
  three sessions: the claim's, the watch and the reader, which may be
  shared with the application's other reads. `[Sc: §5]`
- **§5's reconnect** (F-205). The Setup holds A's connect retry period at
  1 s at most, and Expected 2's bound adds its reconnect. `[Sc: §5]`
- **Where a candidate's sessions are** (§2.5 step 1, §0, §2.15; F-206).
  A complete read covers only what its routers' clients and its linked
  peers hold (core 0.40 §8.1, measured: a router gives a peer no other
  peer's token, and `peers_failover_brokering` has no effect in zenoh
  1.10.1). So a candidate's sessions, and a tool's, MUST be clients of a
  router, or peers linked to every other candidate's. §5's peers listen
  and gossip. `[Sc: §5]`
- **A withdrawal kept through a step-down under `external`** (§2.11;
  F-207) campaigns only with the lock, else nothing; the lock's return
  campaigns in any case. `[F: lifecycle.json]`
- **`hostid.v1` §2.12's reads** (F-208) are of the address's `@zk/**`,
  instance and interface tokens together: its text 0.7.
- **Its own re-mint is no appearance** (§2.9; F-209). The machine gains
  the event `{"reminted": id}`: the new id is its own from then on, and
  every former id is its own, never another holder or candidate. A
  runtime tells its election before declaring the new id's tokens, in
  every state. `[F: lifecycle.json]`

**Changed: what building the runtime showed.**
- **A standby's writes refused, not dropped** (§2.4). A runtime SHOULD
  refuse its application's write of the interface's keys while it stands
  by, saying so: the reference's writers return `Error::Standby`, nothing
  published, so the application is never left guessing its role.
  `[Sc: §7]`
- **A takeover after an earlier incumbency** (§2.6). An instance taking
  over again SHOULD hold nothing of the interface's state that it does not
  put again: the other incumbent may have written since. The reference
  forgets, at each step-down, the values its owner GET answers and its
  refreshers keep. `[Sc: §7]`
- **A poll where no watch session can be held** (§2.7). 0.1's MUST of a
  liveliness subscriber is now "MUST follow the address, SHOULD by a
  subscriber": the runtime's brief asked for a poll beside the watch
  session a caller hands in, and ZenSight's guard polls. The poll's rule,
  its misses (what comes and goes within one period) and its cost (a
  presence read per period) are stated; the reference polls every 500 ms.
  The scenarios run the subscriber, and §1 still checks that B never
  polled.
- **Writes stop before a step-down's step 1** (§2.9), so that nothing of
  the interface is put once the token is withdrawn, which §7 Expected 4
  observes. §2.9's four steps keep their numbers.

**The fixtures.** `lifecycle.json` gains 13 cases (43 in all): nine for
F-199 (an appearance kept while campaigning, the read taking over or
standing by on its holder; a withdrawal kept while campaigning, the read
standing by on the incumbent or taking over; a heal's first read not
made, and its second read not complete, each made again; only a complete
read ending a heal; an appearance kept through a heal read that ends it; a
withdrawal kept through the heal read that steps down), one for F-207, and
three for F-209 (an incumbent's own new id, a heal read showing both of its
ids, a standby's former claim). Its description states the `reminted` event, the `retry` step,
and the rule for what is kept. Each new expectation was written by hand
from the answers before `zenkey_model::redundancy` ran it; none of the 30
earlier cases changed, nor any other fixture.

**Decided here, where the record was silent.**
- **One call elects every governed interface** the service implements:
  `ServiceBuilder::redundancy(RedundancyStartup)`. Governance is a
  contract's (§2.2), and the sessions, timers and lock are the process's.
  Each interface has its own election (§2.3). A governed interface whose
  service takes part in no election is refused at start, as an invalid
  declaration is: an instance of one MUST elect (§2.5), and serving it
  outside the election would break that silently. So is the election
  configured where nothing is governed.
- **A governed interface's resources are born standing by.** Every
  publisher, state queryable, operation and `unavailable` queryable and
  member token the application declares for it, before the start or
  after, is declared only while the instance serves: at the takeover's
  step 1, and undeclared at the step-down's step 2. A raw zenoh publisher
  or queryable on a governed interface is refused, since the runtime could
  not undeclare it.
- **The application's takeover** is a hook between §2.6's step 1 and
  its descriptor's put, where it puts what the new incumbent holds, so
  that a GET at the token finds it. Its role is a watch it can await
  (`unclaimed`, `standby`, `serving`, with why it stands by).
- **The lock** is a trait the runtime asks at the start, every lock
  period (1 s by default), and when the application says it changed; the
  reference gives a `flock` and a lock the application sets. An interface
  elected by `external` with no lock to ask is refused at start.
- **The round trips** (core §8.1, #851): the claim's is a liveliness GET
  of the instance's own key, a population of one; the watch's, of the
  address's `@zk/**`, whose few tokens cost nothing that matters.
- **An act that fails.** zenoh refuses a declaration, a put or an
  undeclaration only when its session is closing or broken. A takeover
  whose step fails is undone at once, every step it made reversed in
  §2.9's order (writes stopped, any token withdrawn, the resources
  undeclared and their state forgotten, the descriptor put marked); a
  step-down makes every step whatever one before it gave. Either way the
  instance stands by and elects no more, and its standing says why
  (`Standing::fault`). The text says nothing of a failed act, and the
  election's machine has no event for one: a retry through it would ask
  the same session to declare again, and would flap, so the reference
  stops and leaves the restart to the application.
- **No write gets around the fence.** Every publication of a governed
  interface's keys goes through its gate: a writer's put, a stamped put or
  delete (which takes the gate's admission as a proof), a state member's
  refresh, an event. A profile's owner the runtime implements whose writes
  go outside the gate or before any election (a desired.v1 author, a
  desired.v1 target's report, alarms.v1's owner on a revision that lists
  `redundancy.v1`) is refused on a governed interface at this version;
  `health.v1` and `config.v1` are the standard contracts, never governed.
- **One read at a time.** The election's machine never asks a read while
  one it asked is unanswered, so a runtime never aborts one, and no phase
  waiting on a read is stranded: `zenkey_model::redundancy`'s
  `a_read_asked_is_never_asked_again_before_its_answer` explores every
  state it reaches, under both elections.
- **A test's hooks**: an instance's id chosen (`ServiceBuilder::set_instance_id`),
  and a heal's reads held (`Redundancy::hold_heal`, for §9 and §12).
- **The owner example's surface**: `--standby` and its options, and its
  `redundancy` commands answered by one JSON line each, documented in
  its `--help`.

**Deliberately not changed.**
- **No consumer in the runtime.** §2.10's follower is the application's;
  §9's K is the test's. A helper may come with the tools (#841, RC).
- **`elections.json`, `heals.json`, `declarations.json`,
  `descriptors.json`, `conclusions.json`**: the decisions over one read and
  two, the declaration, the mark and §5's answers are unchanged.
- **§12**, a tool's, is RC's.
- **No fence and no epoch** (§2.16).

## 0.1 — 2026-10-24: the first text, draft (#841, RA)

`redundancy.v1` is the profile the core has left exclusivity to since its
0.1: O1's at most once holds only while one instance serves (core §5.1,
spike S6), and "election and fencing belong to `redundancy.v1`" (core §6).
Two places waited on it: `hostid.v1` §2.12 reads a standby on a minted
system as a finding "until `redundancy.v1` says how a tool recognises a
standby", and the reference doctor's `token-missing` read a standby's
listing as an exposed interface with no token. ZenSight runs a claim
protocol for its two singletons (`zensight-common/src/service_guard.rs`,
fixed in ZenSight #1104 and #1105), and `zs.catalog.v1` and
`zs.desired.v1` list the profile in `uses`. Core Appendix D's interim row,
`redundancy | election`, was all the core said; no contract wrote the
key. This text is written against core 0.39, which marks a standby in the
descriptor and publishes the vocabulary.

**Decided on #841, before the text** (the maintainer's defaults, not
reopened here).
- **What the profile contributes:** point 4, through `uses`: an interface
  whose contract uses `redundancy.v1` is **governed**; point 2, a
  published vocabulary with one key, `election`, on the interface's
  `[defaults]`: `claim`, the default, this profile's protocol, and
  `external`, an outside lock electing, the profile saying only how a
  standby shows and what a tool concludes. No standard contract or schema.
- **The claim set** is the address's live instance tokens whose
  descriptors mark the governed interface as standing by or serving. A
  pure consumer on the address is not a claim.
- **The incumbent** is the instance holding the governed interface's
  token.
- **Election (`claim`):** claim before reading, the instance's token and
  its descriptor, the interface marked standing by, before it reads
  anything; a live incumbent wins, and the instance stands by; with no
  incumbent, a complete read of the claim set (core 0.37), one not
  complete meaning stand by; the lowest instance id wins, the winner
  exposing the interface's resources, putting its descriptor as serving
  and declaring the interface token last (§8.2's order), a loser standing
  by.
- **A standby** keeps its instance token and its descriptor; exposes none
  of the interface's resources and declares none of its queryables (O3),
  so a call finds no standby; follows the incumbent's interface token by a
  liveliness subscriber on a session that holds no presence read (#830),
  not by polling.
- **A vacancy** is the incumbent's interface token withdrawn, then absent
  in a complete read (core 0.37). It makes every standby campaign again. A
  crashed incumbent is seen at zenoh's lease, 10 s by default.
- **Healing a split:** two incumbents seen in two reads a grace apart
  (§6's rule) make the one with the higher instance id step down: it
  withdraws its token, unexposes, and stands by. The rule is
  deterministic, so a healed partition ends with one incumbent without a
  vote.
- **Consumers** SHOULD accept state and stream samples only from the
  incumbent, the holder of the interface token, and during a split follow
  the lowest instance id, the one the heal keeps: the profile's fencing,
  by presence, not an epoch in the payload, which stops S7's flapping
  between two writers. Callers keep core O1, so a split still runs a call
  on each side, and that is stated, not hidden.
- **Actuators** keep an out-of-bus lock, described as `election =
  "external"` and not replaced.
- **The core first:** the descriptor's per-interface standby mark (§3.3);
  a standby exposing none of an interface's resources, required ones
  included (§8.2 step 2); a standby holding no interface token (§8.1); the
  standby defined, and the split-brain check reading the mark (§6); the
  tokenless set never naming a governed interface, as for `archive.v1`.
- **Not at 1.0:** quorum or consensus; fencing epochs in payloads; leases
  other than zenoh's liveliness; exclusivity across routers stronger than
  "eventual single writer" (ZenSight's own claim).

**Stated.**
- **Who implements it** (§2.1); **the declaration**, governed and the
  election, with a closed list of reasons a declaration is invalid
  (§2.2); **the claim set and the incumbent**, and how ids compare (§2.3);
  **the standby** (§2.4); **the election under `claim`** (§2.5); **taking
  over** (§2.6); **following the address** (§2.7); **a vacancy** (§2.8);
  **healing a split** (§2.9); **consumers, callers, and the stamps they
  read** (§2.10); **`election = "external"`** (§2.11); **population**
  (§2.12); **constrained faces** (§2.13); **the tokenless set** (§2.14);
  **what a tool concludes** (§2.15); **not at 1.0** (§2.16).
- **No contract** (§3), and **the vocabulary** (§4), `election`, the
  interim row's key unchanged.
- **What a tool may conclude** (§5): is the address served by exactly one
  incumbent; is there a vacancy; does any instance stand by, and how
  many; is this instance a standby, or a twin. The changes against v1's
  ownership protocol (Appendix A), and the adopters' mappings
  (Appendix B).
- **The core's part** is 0.39 ([`../../CHANGELOG.md`](../../CHANGELOG.md)):
  the mark, D012, a standby's tokens and exposure, serving later and
  stopping in §8.2's order, the tokenless refusal, the split-brain check
  reading the mark, Appendix D's published row, and W105's fixture
  `contracts/w105-redundancy-epoch`.
- **Evidence:** `conformance/declarations.json` (26 cases),
  `conformance/elections.json` (29), `conformance/heals.json` (16),
  `conformance/descriptors.json` (20), `conformance/lifecycle.json` (30),
  `conformance/conclusions.json` (39), 160 in all. Each expectation was
  computed by an independent transcription of this text before
  `zenkey_model::redundancy` ran it. Twelve scenario sections, eleven the
  runtime's and one a tool's, over two test contracts, `ledger.v1`
  (`claim`) and `valve.v1` (`external`), kept under
  `zenkey/tests/contracts/`. `zenkey_model::redundancy` runs every
  fixture.
- **`hostid.v1` text 0.6** points its §2.12 here: a recognised standby,
  which §2.15's fourth question answers `standby` of every interface it
  lists, all governed, is not counted (its CHANGELOG).
- **The examples' contracts need no new revision.** `zs.catalog.v1` and
  `zs.desired.v1` list `redundancy.v1` in `uses` and write no election,
  so each reads governed, `claim`, as the reference checks; their header
  comments, outside the fingerprint, were rewritten to this text.
  `modem.v3`, `tc.netif.v1` and `tc.netem.v1` do not list the profile,
  and read not this profile's (Appendix B).

**Decided here, where the record was silent.**
- **The election is read from the resources.** A contract writes
  `redundancy.election` on `[defaults]`, and the canonical form carries
  merged annotations per resource (core §9.5), so a reader of a bundle
  finds it on each resource, never on the interface. Every resource
  resolves to one election, or the declaration is invalid, a resource
  carrying none read as `claim`, and an interface with no resource is
  `claim`; a `[defaults.<kind>]` election
  that reaches some resources only is `disagrees`. The key on a
  requirement means nothing, and another `redundancy.<key>` is W105's
  and makes nothing invalid.
- **An invalid declaration** has four reasons, checked alone: `not_used`
  (core E020's, for a reader of annotations from elsewhere), `value`,
  `disagrees`, `tokenless`. A runtime refuses to start such an instance,
  as core §8.2 step 2 refuses, and a tool reports it.
- **Governance is a revision's.** Each instance is governed by the
  revision its descriptor names; adding or dropping `uses`, or changing
  the election, is a review (core §9.8). A tool reads an address whose
  candidates' revisions disagree as unobservable, `revisions_disagree`,
  never as one or the other.
- **A read is one liveliness read of the address's `@zk/**`,** instance
  and interface tokens together, with the descriptors of the instances
  whose ids are lower than the reader's own: a higher id cannot beat it,
  so its descriptor is not needed.
- **Its own claim must be in the read** (accepted by the maintainer on
  2026-10-24). ZenSight's check ("an answer that
  does not contain our own claim is an answer we did not really get")
  carried over: a refused read is complete and empty (core §8.1, 0.8), so
  completeness alone cannot tell it from a lone candidate's.
- **Which reads are complete, by core 0.37 and 0.38.** A read cut off
  while it ran (its session's links gone when it ended) is possibly
  incomplete, and a read not made is read as one (0.38's premise rule), so
  either makes a campaigning instance stand by and retry; the tokens a read
  not complete returned were seen, so an incumbent it shows still wins,
  and the holders it shows still count toward a split. A tool reads a read
  not made as one not complete, and reports its reason beside the answer.
- **"Claim before reading" across sessions.** The read goes on a session
  that has never held a liveliness subscriber (core 0.37, 0.38), which
  may not be the one holding the claim; then the instance makes one round trip on the claim's
  session first, since zenoh orders no two sessions' messages. The claim
  narrows the simultaneous-start race; the heal ends what it leaves.
- **The retry interval.** A standby that saw no incumbent (the read not
  complete, its own claim missing, a lower descriptor unread, or a lower
  candidate) campaigns again after an interval of its own, SHOULD be a few
  seconds, as ZenSight's 2 s poll was. One that saw an incumbent waits on
  its watch alone. A lower candidate is retried too, since it may never
  take over.
- **The watch covers the address's whole `@zk/**`,** held by every
  candidate from before its first read: the standby to hear its incumbent
  go, the incumbent to hear a second one come, which is the heal's
  trigger.
- **How the heal is seen.** An incumbent that hears another holder appear
  reads at once and again a grace later; its own token counted whether a
  read shows it or not; a read's completeness ignored, since a read cannot
  invent a holder; the second read's lowest holder deciding, so three
  incumbents leave one. A crashed lower holder may be stepped down to; the
  vacancy that follows is campaigned for.
- **The order of a takeover and a step-down** is core §8.2's and its
  reverse (core 0.39): resources, the descriptor put without the mark,
  then the token; then the tokens, the resources, the descriptor put
  marked. A takeover's validation (step 2) happens at the start, as a
  standby, so a takeover cannot fail it. A standby, and an instance
  stepping down, put and delete none of the interface's keys: a delete
  would read as the incumbent's.
- **A standby still holds the bundle.** Its contract queryable (core
  §8.4) is no resource of the interface, and a tool retrieves the contract
  from any holder.
- **A standby's `unavailable` and `cardinality`** state what it will
  expose once it serves (core 0.39): one member flips at a takeover, and a
  tool sees a standby's readiness before it.
- **Stamps the consumer reads.** A candidate MUST state `meta.zid` (a
  SHOULD in core §3.3) and enable its HLC whatever kinds it serves, so
  every sample names its writer. A following consumer discards a sample it
  cannot attribute, accepts nothing new while it sees no incumbent, and
  pays for a split's seed: `Latest` keeps the later stamp per key,
  whichever incumbent wrote it, so the key reads nothing until its
  incumbent puts again, which the text states.
- **`external`** (accepted by the maintainer on 2026-10-24): an instance
  without the lock refuses to start or stands by; with it, it takes over once a read shows no other holder, complete
  and holding its own claim, without the lowest-id step, since the lock
  decided, and waits for the vacancy rather than take over beside a
  crashed holder's token, which outlives the lock by up to a lease; it
  never steps down by ids, and steps down when it loses the lock; its
  anti-twin probe is an optional early refusal. A split under `external`
  is a tool's finding the profile does not heal.
- **Every interface at a governed address SHOULD be governed** (confirmed
  by the maintainer on 2026-10-24, for #841's record). One that is not is
  exposed by every instance at once, so an address with governed
  interfaces implements governed interfaces only, and a process's own
  `health.v1` and `alarms.v1`, whose contracts no adopter can make use the
  profile, go on an address of its own, as ZenSight's correlator does.
- **Several governed interfaces at one address** are elected apart.
- **Constrained faces:** presence does not cross, so a deployment SHOULD
  keep an address's candidates on one side; a tool across a face answers
  `across_face`.
- **A tool's questions** read two reads a grace apart (core §6, and §8.1's
  two-read rule for every presence shape), answer in five states, `not
  this profile's` among them, and count by token: a holder marked standby
  is an incumbent (`marked_holding` when it lasts), a candidate listing
  the interface unmarked with no token is `unmarked`, a holder not listing
  it `unlisted`. "Exactly one" and "standby" need a complete read; a split
  does not. The standby count is exact only from a complete read with
  every descriptor read, and a lower bound otherwise. `hostid.v1` §2.12's
  second process is the instance the fourth question answers `standby`.
- **The mark's reading** (`descriptors.json`): `malformed` first and
  alone; then `standby_tokenless` (core D012), `governed_tokenless`, and
  `not_governed` for a mark on an interface the profile does not govern.
- **A sixth fixture,** `lifecycle.json`, pins the state machine the
  runtime drives: events, steps, and the role after them, so that a second
  runtime reaches the same steps from the same events.
- **Uses no other profile.** Nothing here needs `freshness.v1`'s horizon:
  liveness is presence.

**Deliberately not stated.**
- **No quorum, epoch, claim id or lease of the profile's own,** and no
  exclusivity stronger than eventual single writer (§2.16): the
  maintainer's "not at 1.0".
- **No state handover, preferred incumbent, priority or switchover
  operation.** A clean stop of the incumbent is the switchover.
- **No reporting by a standby at its address.** Its `health.v1` would be
  a second writer of the incumbent's status; a process's own reporting
  goes on its own address.
- **No tool here.** `hostid.v1`'s text moves now (its 0.6); the doctor's
  `hostid-duplicate` reading it is #841's RC.
- **No timing numbers as rules.** The retry interval, the grace and the
  scenarios' 2 s are settings; the scenarios fix them for their runs.
- **No runtime and no tool here.** `zenkey`'s `ServiceBuilder::redundancy()`
  and the owner example's `--standby` are #841's RB, and run §1 to §11;
  zenctl's questions, the doctor reading the mark and zengui's standby are
  its RC, and run §12.
