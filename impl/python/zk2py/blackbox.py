"""Where zk2py's live runners find the reference's black boxes: the owner
example, the ``consume`` example and ``zenctl``, built from main elsewhere and
run, never read (#609).

One constant for every runner (review-bot on round 102, #1069): the
directory is ``$ZK2PY_BLACKBOX``, and :data:`DEFAULT` where it is unset.
"""

from __future__ import annotations

import os
from pathlib import Path

#: The black boxes' directory on this machine, where ``ZK2PY_BLACKBOX`` is
#: unset.
DEFAULT = "/srv/dev/scratch/zenkey/bin/current"
#: The environment variable that names another directory.
ENV = "ZK2PY_BLACKBOX"


def directory() -> Path:
    """The black boxes' directory: ``$ZK2PY_BLACKBOX``, or :data:`DEFAULT`."""
    return Path(os.environ.get(ENV, DEFAULT))


#: The directory as the environment named it when zk2py was imported.
BLACKBOX = directory()
