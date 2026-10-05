#!/usr/bin/env python3
"""Cut lucide-static's icon font down to the icons zengui draws (#533).

The full font is ~900 KB for ~2,150 icons; zengui draws a few dozen. This
keeps exactly the names in zengui/assets/fonts/lucide-icons.txt and writes
zengui/assets/fonts/lucide.ttf, printing each kept name and codepoint so a
change to the list is a reviewable diff of `kit::Icon`.

Not run by CI — the output is committed, and `view::fonts`'s tests check it
against `kit::Icon`. Needs fontTools (pure Python; a wheel unzipped onto
PYTHONPATH is enough) and an unpacked lucide-static tarball:

    curl -sL https://registry.npmjs.org/lucide-static/-/lucide-static-1.52.0.tgz | tar xz
    PYTHONPATH=<fonttools> scripts/subset-icons.py package
"""

import json
import pathlib
import sys

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = pathlib.Path(__file__).resolve().parent.parent
ASSETS = ROOT / "zengui/assets/fonts"


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    package = pathlib.Path(sys.argv[1])
    codepoints = json.loads((package / "font/codepoints.json").read_text())
    names = [
        line.strip()
        for line in (ASSETS / "lucide-icons.txt").read_text().splitlines()
        if line.strip() and not line.startswith("#")
    ]
    missing = [n for n in names if n not in codepoints]
    if missing:
        sys.exit(f"not in lucide-static: {', '.join(missing)}")

    font = TTFont(package / "font/lucide.ttf")
    options = subset.Options()
    options.glyph_names = True
    options.name_IDs = ["*"]
    options.notdef_outline = True
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=[codepoints[n] for n in names])
    subsetter.subset(font)
    font.save(ASSETS / "lucide.ttf")
    for n in names:
        print(f"{n:28} U+{codepoints[n]:04X}")


if __name__ == "__main__":
    main()
