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
from pathlib import Path

#: The black boxes' directory on this machine, where ``ZK2PY_BLACKBOX`` is
#: unset.
DEFAULT = "/srv/dev/scratch/zenkey/bin/current"
#: The environment variable that names another directory.
ENV = "ZK2PY_BLACKBOX"


#: The black boxes' directory: ``$ZK2PY_BLACKBOX`` as the environment named it
#: when zk2py was imported, or :data:`DEFAULT`.
BLACKBOX = Path(os.environ.get(ENV, DEFAULT))
