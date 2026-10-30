"""The keys a node answers under its routing tables' read lock (observer §4.3, core
§11.1 and Appendix B; 0.48, any node's since 0.50, F-253): its declaration
records, ``@/<zid>/<whatami>/{subscriber,publisher,queryable,querier,token}/**``,
a router's, a peer's or a client's, its ``linkstate/*`` and its
``route/successor/src/<zid>/dst/<zid>``.

A tool MUST NOT ask a selector that intersects one of them unless its user
asks (observer §4.3: :func:`zk2py.live.admin_guard`), and a generator MUST NOT allow
a ``query`` on one (core §11.1: :func:`zk2py.acl.block_hazards`). The patterns
are plain strings, so both import them without zenoh; each reader
intersects them with zenoh 1.10.1's own key expressions.
"""

from __future__ import annotations

ADMIN_RECORDS: tuple[str, ...] = tuple(
    f"@/*/*/{kind}/**" for kind in ("subscriber", "publisher", "queryable", "querier", "token")) \
    + ("@/*/*/linkstate/*", "@/*/*/route/successor/**")
