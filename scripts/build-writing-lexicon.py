#!/usr/bin/env python3
"""Build or verify Badi's pinned, conservative English completion lexicon offline."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "broker/data/writing-lexicon"
COMMIT = "8cfea406b505e4d7df52d5a19bce525df98c54ab"
UPSTREAM = f"https://raw.githubusercontent.com/wooorm/dictionaries/{COMMIT}/dictionaries/en"
HUNSPELL_VERSION = "1.7.3"
SOURCES = {
    "source/index.dic": ("index.dic", "f0b1a234bd178bdd01875b2a392a9647f888b8fe879f79c52aae62c2759b3647"),
    "source/index.aff": ("index.aff", "8ae1f19d4840d957728ad90555d5a8dff6cc5c046279c95ff0c00fc0a0136c7b"),
    "LICENSE": ("license", "2a7e8d8ae9e8facc84818546ae2a8d83aec5e9c80a675ff789acd1c338b53b3d"),
}
ALPHABETIC = re.compile(r"[a-z]+\Z")
EXCLUDED_DIRECTIVES = {"NOSUGGEST", "FORBIDDENWORD", "ONLYINCOMPOUND", "NEEDAFFIX", "PSEUDOROOT"}
ALLOWED_DIRECTIVES = EXCLUDED_DIRECTIVES | {
    "SET", "TRY", "ICONV", "COMPOUNDMIN", "COMPOUNDRULE", "WORDCHARS", "PFX", "SFX", "REP",
}


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def source_metadata(directory):
    metadata = {}
    for path, (upstream_name, expected) in SOURCES.items():
        data = (directory / path).read_bytes()
        if sha256(data) != expected:
            raise ValueError(f"Pinned source checksum mismatch: {path}")
        metadata[path] = {"url": f"{UPSTREAM}/{upstream_name}", "sha256": expected, "bytes": len(data)}
    return metadata


def excluded_flags(affix):
    """unmunch expands affixes but does not implement Hunspell suggestion flags."""
    flags = set()
    for line in affix.splitlines():
        fields = line.split()
        if not fields or fields[0].startswith("#"):
            continue
        directive = fields[0]
        if directive not in ALLOWED_DIRECTIVES:
            raise ValueError(f"Unreviewed affix directive: {directive}")
        if directive == "SET" and fields != ["SET", "UTF-8"]:
            raise ValueError("Only the pinned UTF-8 dictionary format is supported")
        if directive in EXCLUDED_DIRECTIVES:
            if len(fields) != 2 or len(fields[1]) != 1 or not fields[1].isascii():
                raise ValueError("Only single-byte exclusion flags are supported")
            flags.add(fields[1])
        if directive in {"PFX", "SFX"} and len(fields) > 4 and "/" in fields[3]:
            raise ValueError("Affix continuation flags need a separately reviewed generator")
    return flags


def partition_entries(dictionary, flags):
    lines = dictionary.splitlines()
    if not lines or not lines[0].isdigit() or int(lines[0]) != len(lines) - 1:
        raise ValueError("Dictionary entry count does not match its header")
    allowed, excluded = [], []
    rejected_format = 0
    for line in lines[1:]:
        if any(character.isspace() for character in line) or "\\" in line or line.count("/") > 1:
            raise ValueError("Unreviewed escaped or morphological dictionary entry")
        word, _, entry_flags = line.partition("/")
        if not word or not line.isascii():
            raise ValueError("Only the pinned ASCII stems and single-byte flags are supported")
        if flags.intersection(entry_flags):
            excluded.append(line)
        elif ALPHABETIC.fullmatch(word):
            allowed.append(line)
        else:
            rejected_format += 1
    return allowed, excluded, rejected_format


def expand(entries, affix_path, unmunch):
    if not entries:
        return set()
    with tempfile.TemporaryDirectory(prefix="badi-lexicon-") as temporary:
        dictionary = Path(temporary) / "words.dic"
        dictionary.write_text(f"{len(entries)}\n" + "\n".join(entries) + "\n", encoding="utf-8")
        # unmunch emits verbose affix diagnostics on stderr even on success.
        result = subprocess.run([str(unmunch), str(dictionary), str(affix_path)],
                                capture_output=True, check=True, timeout=30)
    expanded = result.stdout.decode("utf-8").splitlines()
    return {word for word in expanded if ALPHABETIC.fullmatch(word)}


def generator_tool(unmunch_name, hunspell_name):
    unmunch, hunspell = shutil.which(unmunch_name), shutil.which(hunspell_name)
    if not unmunch or not hunspell:
        raise ValueError("Regeneration requires Hunspell 1.7.3 (hunspell and unmunch); --verify needs only Python")
    version = subprocess.run([hunspell, "-v"], capture_output=True, text=True, check=True, timeout=5)
    if not re.search(rf"\bHunspell {re.escape(HUNSPELL_VERSION)}\)", version.stdout):
        raise ValueError(f"Regeneration requires Hunspell {HUNSPELL_VERSION}")
    return Path(unmunch)


def generate(directory, unmunch):
    sources = source_metadata(directory)
    flags = excluded_flags((directory / "source/index.aff").read_text(encoding="utf-8"))
    allowed, excluded, rejected_format = partition_entries(
        (directory / "source/index.dic").read_text(encoding="utf-8"), flags)
    # Subtract the complete excluded families as well, so another dictionary
    # entry or affix combination cannot reintroduce a flagged surface form.
    candidates = expand(allowed, directory / "source/index.aff", unmunch)
    blocked = expand(excluded, directory / "source/index.aff", unmunch)
    words = sorted(candidates - blocked)
    data = ("\n".join(words) + "\n").encode("utf-8")
    manifest = {
        "schema": "badi.writing-lexicon.v1",
        "language": "en-US",
        "upstream_repository": "https://github.com/wooorm/dictionaries",
        "upstream_commit": COMMIT,
        "upstream_dictionary": "SCOWL en_US size 60, 2020.12.07 (mk-list --accents=strip en_US 60)",
        "sources": sources,
        "generation": {
            "script": "scripts/build-writing-lexicon.py",
            "script_sha256": sha256(Path(__file__).read_bytes()),
            "tool": "Hunspell unmunch",
            "version": HUNSPELL_VERSION,
            "tool_source": f"https://github.com/hunspell/hunspell/blob/v{HUNSPELL_VERSION}/src/tools/unmunch.cxx",
            "rules": "Lowercase ASCII alphabetic stems only; expand original affixes; exclude special-flag families; retain only [a-z]+; sort unique UTF-8 lines with LF terminators. No added words or frequency ranking.",
            "excluded_flags": sorted(flags),
            "eligible_stems": len(allowed),
            "excluded_flagged_stems": len(excluded),
            "excluded_other_stems": rejected_format,
            "excluded_expanded_words": len(blocked),
            "overlapping_words_removed": len(candidates & blocked),
        },
        "output": {"path": "en.txt", "sha256": sha256(data), "bytes": len(data), "words": len(words)},
        "license": "LICENSE retains the full original SCOWL, Ispell, WordNet and other upstream notices. This word list is a modified derivative; see README.md.",
    }
    return data, manifest


def verify(directory):
    sources = source_metadata(directory)
    manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
    if (manifest.get("schema") != "badi.writing-lexicon.v1" or manifest.get("sources") != sources
            or manifest.get("upstream_commit") != COMMIT):
        raise ValueError("Manifest provenance does not match the pinned sources")
    generation = manifest.get("generation", {})
    if (generation.get("script_sha256") != sha256(Path(__file__).read_bytes())
            or generation.get("version") != HUNSPELL_VERSION):
        raise ValueError("Manifest generator provenance does not match the current script")
    data = (directory / "en.txt").read_bytes()
    words = data.decode("utf-8").splitlines()
    if not words or data != ("\n".join(sorted(set(words))) + "\n").encode("utf-8"):
        raise ValueError("Word list must contain sorted unique UTF-8 lines with LF terminators")
    if not all(ALPHABETIC.fullmatch(word) for word in words):
        raise ValueError("Word list contains a non-lowercase-alphabetic entry")
    expected = {"path": "en.txt", "sha256": sha256(data), "bytes": len(data), "words": len(words)}
    if manifest.get("output") != expected:
        raise ValueError("Generated word list does not match its manifest")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="Regenerate offline and require byte-identical assets")
    mode.add_argument("--verify", action="store_true", help="Verify bundled hashes and format without build tools")
    parser.add_argument("--unmunch", default="unmunch", help="Hunspell 1.7.3 expansion executable")
    parser.add_argument("--hunspell", default="hunspell", help="Companion executable used to check the Hunspell version")
    args = parser.parse_args()
    if args.verify:
        manifest = verify(DATA)
    else:
        unmunch = generator_tool(args.unmunch, args.hunspell)
        data, manifest = generate(DATA, unmunch)
        encoded_manifest = (json.dumps(manifest, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
        if args.check:
            verify(DATA)
            if data != (DATA / "en.txt").read_bytes() or encoded_manifest != (DATA / "manifest.json").read_bytes():
                raise ValueError("Generated assets differ; regenerate and review the lexicon change")
        else:
            (DATA / "en.txt").write_bytes(data)
            (DATA / "manifest.json").write_bytes(encoded_manifest)
    print(f"English lexicon: {manifest['output']['words']} words, sha256={manifest['output']['sha256']}")


if __name__ == "__main__":
    main()
