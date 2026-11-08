"""``scenarios/state.md`` §15, "A ``reply_del`` where the owner answers by
snapshot" (core 0.102 §4.2; #609, round 107), live, with zk2py's owner and
readers. ``python -m zk2py.stray_live``.

**Step 1, a stray ``reply_del``.** zk2py's owner holds §14's 32 plans
(``interop/state/zk2py_fleet.v1.toml``, :class:`zk2py.snapshot_live.Fleet`).
A session of the test's own plays the owner, as §14's steps 3 and 5 do: the
owner's own answer to ``plans/**``, its pages taken by a raw GET, and beside
it a ``reply_del`` on ``plans/v03`` under the owner's run, at a seq above the
snapshot's asof, which no owner sends. Read by T, a tool's late joiner, by a
consumer's, and by S4's GET, each given the owner's revision
(:class:`zk2py.seed_snapshot.ByDescriptor`); and, the control, by S4's GET
without it, which "cannot tell a stray ``reply_del`` from an owner's, and
merges it as a deletion".

**Step 2, an owner on an earlier revision.** zk2py's owner of
``zk2py_stray.v1`` 1.0, whose ``status`` (``freshness.ttl_s`` 60, a contract
that uses ``freshness.v1``) is a per-key resource; T holds 1.0 and 1.1, whose
``status`` is a ``freshness.v2`` collection. The owner deletes ``status``,
and within W T GETs ``state/**``: by the descriptor's revision the
``reply_del`` is the owner's, merged as a deletion; by 1.1 alone it would be
a stray one.

**Step 3, the readers** (review-bot on #1100, 2; round 108): a member read
of ``plans/v03``, played as step 1's, the owner's member snapshot and the
stray ``reply_del`` beside it, read by :func:`zk2py.freshness.get_reading`,
the reader the alarms, health and freshness tools share the GET of, given
the revisions it holds: the member stays live, the ``reply_del`` refused,
``form``; without them it reads the member deleted, as the text says a
reader that cannot read the owner's revision does.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[3]
STRAY = REPO / "impl" / "python" / "interop" / "state" / "zk2py_stray.v1.toml"
STRAY_1 = REPO / "impl" / "python" / "interop" / "state" / "zk2py_stray.v1-1.toml"

Result = tuple[str, bool, str]


def step1() -> list[Result]:
    import zenoh

    from . import live
    from . import seed as sd
    from . import seed_snapshot as ss
    from . import snapshot_live as sl
    from . import zk2c
    from .live_interop import _r1

    out: list[Result] = []
    r, ep, _ = _r1(timestamping=False)
    tool, helper = live.open_client(ep), live.open_client(ep)
    f = sl.Fleet("zk2py", ep)
    player = None
    try:
        members = f.setup()
        time.sleep(0.5)
        coll = f"{f.base}/plans"
        answer = sl.raw_get(tool, f"{coll}/**")
        pages = [ss.read_page_of(k, b) for k, b in answer]
        head = pages[0]
        stray_key = f"{coll}/v03"
        stamp = zk2c.InBand(head["run_width"], head["run"], head["asof"] + 1)
        player = sl.Player(helper, f"{coll}/**")
        owner_off = sl._quiet(f)  # noqa: SLF001 - the runner's own helper

        def play() -> None:
            player.taken.wait(5.0)
            with player.lock:
                qs, player.queries = player.queries, []
            for q in qs:
                for k, b in answer:
                    q.reply(k, b, encoding=zk2c.ENCODING)
                q.reply_del(stray_key, attachment=zk2c.deletion(stamp))
                q.drop()

        import threading

        def read(make: Any) -> Any:
            th = threading.Thread(target=play, daemon=True)
            player.taken.clear()
            th.start()
            got = make()
            th.join(10)
            return got

        try:
            time.sleep(0.3)

            def joiner(consumer: bool) -> sd.LateJoiner:
                lj = sd.LateJoiner(tool, f"{coll}/*", consumer=consumer, timeout=5.0, get_selector=f"{coll}/**",
                                   collections=ss.ByDescriptor([f.contract]))
                lj.start()
                lj.wait()
                return lj

            t = read(lambda: joiner(False))
            c = read(lambda: joiner(True))
            g = read(lambda: live.get_state(tool, f"{coll}/**", timeout=5.0, revision=ss.ByDescriptor([f.contract])))
            g0 = read(lambda: live.get_state(tool, f"{coll}/**", timeout=5.0))
        finally:
            owner_off()
        for name, lj in (("T, a tool's late joiner", t), ("a consumer's late joiner", c)):
            held = sorted(k[len(coll) + 1:] for k, e in lj.view().items() if not e.deleted)
            done = [h.done for h in lj.trackers[-1].heard] if lj.trackers else []
            out.append((f"§15 step 1, {name}: the snapshot complete and the 32 plans, plans/v03 among them; the "
                        "reply_del counted undecodable, refused as form, and no deletion of plans/v03 merged "
                        "(core 0.102 §4.2)",
                        done == [True] and held == sorted(members) and (stray_key, "reply", "form") in lj.undecodable
                        and not lj.view()[stray_key].deleted,
                        f"complete {done}, {len(held)} plans, undecodable {lj.undecodable}"))
        live_ = {x.key: x for x in g.replies}
        stray = [x for x in g.replies if x.key == stray_key and x.refusal == "form"]
        out.append(("§15 step 1, S4's GET given the owner's revision: plans/v03 live from the snapshot, and the "
                    "reply_del a refused reply, form, never merged",
                    len(stray) == 1 and stray_key in live_ and any(x.key == stray_key and not x.deleted
                                                                   and x.refusal is None for x in g.replies)
                    and (stray_key, "refused", None) in g.arrivals,
                    f"{[(x.key[-3:], x.deleted, x.refusal) for x in g.replies if x.key == stray_key]}"))
        out.append(("§15 step 1, the control: S4's GET without the owner's revision merges the reply_del as a "
                    "deletion, as the text says a reader without it does (core §4.2)",
                    any(x.key == stray_key and x.deleted and x.refusal is None for x in g0.replies),
                    f"{[(x.key[-3:], x.deleted, x.refusal) for x in g0.replies if x.key == stray_key]}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("§15 step 1: the run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        if player is not None:
            player.close()
        f.close()
        tool.close()
        helper.close()
        r.close()
    _ = zenoh
    return out


def step2() -> list[Result]:
    from . import live
    from . import seed as sd
    from . import seed_snapshot as ss
    from .contract import load_contract
    from .live_interop import _r1
    from .owner import Owner

    out: list[Result] = []
    r, ep, _ = _r1(timestamping=False)
    tool = live.open_client(ep)
    old, new = load_contract(STRAY), load_contract(STRAY_1)
    o = Owner("lab", "stray", [old], connect=ep)
    o.start()
    key = "zk2/lab/stray/zk2py_stray.v1/state/status"
    try:
        o.set_state(key, b"up")
        time.sleep(0.5)
        o.delete_state(key)
        time.sleep(0.3)
        both = ss.ByDescriptor([old, new])
        lj = sd.LateJoiner(tool, "zk2/lab/stray/zk2py_stray.v1/state/*", consumer=False, timeout=3.0,
                           get_selector="zk2/lab/stray/zk2py_stray.v1/state/**", collections=both)
        lj.start()
        lj.wait()
        g = live.get_state(tool, "zk2/lab/stray/zk2py_stray.v1/state/**", timeout=3.0, revision=both)
        by_new = ss.per_key_in(new, key)
        st = lj.view().get(key)
        out.append(("§15 step 2: by the descriptor's revision (1.0, freshness.v1) status is a per-key resource; its "
                    "owner answers it with a reply_del within W, and T merges it as a deletion; read by 1.1, which T "
                    "holds too, it would be a stray reply_del (core §4.2)",
                    both.per_key(key, st.inband if st else None) is True and by_new is False
                    and st is not None and st.deleted and not lj.undecodable
                    and any(x.key == key and x.deleted and x.refusal is None for x in g.replies),
                    f"per key by 1.0 {both.per_key(key, st.inband if st else None)}, by 1.1 {by_new}; T holds "
                    f"{st and ('deleted' if st.deleted else 'live')}; undecodable {lj.undecodable}; GET "
                    f"{[(x.key[-6:], x.deleted, x.refusal) for x in g.replies]}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("§15 step 2: the run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        o.close()
        tool.close()
        r.close()
    return out


def step3() -> list[Result]:
    import threading

    from . import freshness as fr
    from . import live
    from . import seed_snapshot as ss
    from . import snapshot_live as sl
    from . import zk2c
    from .live_interop import _r1

    out: list[Result] = []
    r, ep, _ = _r1(timestamping=False)
    tool, helper = live.open_client(ep), live.open_client(ep)
    f = sl.Fleet("zk2py", ep)
    player = None
    try:
        f.setup()
        time.sleep(0.5)
        key = f"{f.base}/plans/v03"
        answer = sl.raw_get(tool, key)
        head = ss.read_page_of(answer[0][0], answer[0][1])
        stamp = zk2c.InBand(head["run_width"], head["run"], head["asof"] + 1)
        player = sl.Player(helper, f"{f.base}/plans/**")
        owner_off = sl._quiet(f)  # noqa: SLF001 - the runner's own helper

        def play() -> None:
            player.taken.wait(5.0)
            with player.lock:
                qs, player.queries = player.queries, []
            for q in qs:
                for k, b in answer:
                    q.reply(k, b, encoding=zk2c.ENCODING)
                q.reply_del(key, attachment=zk2c.deletion(stamp))
                q.drop()

        def read(contracts: Any) -> Any:
            th = threading.Thread(target=play, daemon=True)
            player.taken.clear()
            th.start()
            got = fr.get_reading(tool, key, timeout=5.0, contracts=contracts)
            th.join(10)
            return got

        try:
            time.sleep(0.3)
            given = read([f.contract])
            control = read(None)
        finally:
            owner_off()
        out.append(("§15, the readers (freshness.get_reading, given the revisions the reader holds): plans/v03 read "
                    "live from the owner's member snapshot; the stray reply_del never merged as a deletion "
                    "(core 0.102 §4.2; review-bot on #1100, 2)",
                    given.reply == "put" and given.snapshot,
                    f"{given.reply}, snapshot {given.snapshot}"))
        out.append(("§15, the readers, the control: without the owner's revision the reply_del is merged, the "
                    "member read deleted, as the text says a reader that cannot read the owner's revision does",
                    control.reply == "delete", f"{control.reply}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("§15 step 3: the run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        if player is not None:
            player.close()
        f.close()
        tool.close()
        helper.close()
        r.close()
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.stray_live", description=__doc__.split("\n")[0])
    ap.parse_args(argv)
    failed = 0
    for what, ok, detail in step1() + step2() + step3():
        print(f"{'PASS' if ok else 'FAIL'} {what}" + (f": {detail}" if detail else ""), flush=True)
        failed += not ok
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
