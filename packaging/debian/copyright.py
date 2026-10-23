#!/usr/bin/env python3
"""Write the package's /usr/share/doc/badi/copyright to standard output.

    copyright.py LICENSE THIRD-PARTY-NOTICES.txt SCOWL-LICENSE

Debian keeps every licence of a package in that one file: Badi's MIT
licence, then THIRD-PARTY-NOTICES.txt, then the English lexicon's SCOWL
notices. Debian ships the Apache License 2.0 in /usr/share/common-licenses
and asks packages to refer to it there rather than repeat it, so each
Apache-2.0 text in the notices becomes that reference. Copyright lines the
text carried, and the LLVM exception, are kept.
"""

import re
import sys
from pathlib import Path

APACHE_TERMS = "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION"
LLVM_EXCEPTION = "LLVM Exceptions to the Apache 2.0 License"
TEMPLATE = re.compile(r"Copyright \[yyyy\] \[name of copyright owner\]")


def without_apache_texts(notices):
    """The notices with each full Apache-2.0 text replaced by Debian's copy."""
    sections = re.split(r"(?m)^(?===> )", notices)
    kept = [sections[0]]
    for section in sections[1:]:
        header, _, text = section.partition("\n")
        if APACHE_TERMS in text and re.search(r"Apache License\s+Version 2\.0,", text):
            owners = [line.strip() for line in text.splitlines()
                      if line.strip().startswith("Copyright") and not TEMPLATE.search(line)]
            text = "\nApache License 2.0: /usr/share/common-licenses/Apache-2.0\n"
            if owners:
                text += "\n" + "\n".join(owners) + "\n"
            exception = section.find(LLVM_EXCEPTION)
            if exception >= 0:
                start = section.rfind("\n", 0, exception) + 1
                text += "\n" + section[start:].rstrip("\n") + "\n"
            text += "\n"
        kept.append(header + "\n" + text)
    return "".join(kept)


def main(arguments):
    if len(arguments) != 3:
        raise SystemExit(__doc__.strip().splitlines()[2].strip())
    licence, notices, scowl = (Path(argument).read_text(encoding="utf-8") for argument in arguments)
    sys.stdout.write("Badi\nSource: https://github.com/ahuray/badi\n\n" + licence.rstrip("\n") + "\n\n\n"
                     + without_apache_texts(notices).rstrip("\n") + "\n\n\n"
                     "The English writing lexicon (SCOWL), compiled into badi-broker\n"
                     "=============================================================\n\n" + scowl)


if __name__ == "__main__":
    main(sys.argv[1:])
