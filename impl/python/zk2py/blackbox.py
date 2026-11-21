"""Where zk2py's live runners find the reference's black boxes: the owner
example, the ``consume`` example and ``zenctl``, built from main elsewhere and
run, never read (#609).

One constant for every runner (review-bot on round 102, #1069): the
directory is ``$ZK2PY_BLACKBOX``, and :data:`DEFAULT` where it is unset,
read once, when zk2py is imported. There is no second, live reading of the
environment (review-bot on round 103, #1076): a runner started under
another ``ZK2PY_BLACKBOX`` is a new process.
"""

from __future__ import annotations

import os
import tempfile
from pathlib import Path

#: The black boxes' directory on this machine, where ``ZK2PY_BLACKBOX`` is
#: unset.
DEFAULT = "/srv/dev/scratch/zenkey/bin/current"
#: The environment variable that names another directory.
ENV = "ZK2PY_BLACKBOX"


#: The black boxes' directory: ``$ZK2PY_BLACKBOX`` as the environment named it
#: when zk2py was imported, or :data:`DEFAULT`.
BLACKBOX = Path(os.environ.get(ENV, DEFAULT))

#: The environment variable that names where a live runner keeps its working
#: files (a router's sockets, its ``usrpwd`` dictionary, a capture). Unset,
#: they go under the platform's temporary directory.
SCRATCH_ENV = "ZK2PY_SCRATCH"


def workdir(prefix: str) -> tempfile.TemporaryDirectory:
    """A working directory of a live runner's own, removed when its ``with``
    block ends, with everything in it: a run leaves no ``users.txt`` behind
    (review-bot on round 109, #1121). It is made under ``$ZK2PY_SCRATCH``,
    which is created where missing, or under the platform's temporary
    directory where that is unset: no path of one machine is assumed."""
    base = os.environ.get(SCRATCH_ENV) or None
    if base is not None:
        os.makedirs(base, exist_ok=True)
    return tempfile.TemporaryDirectory(prefix=prefix, dir=base)
