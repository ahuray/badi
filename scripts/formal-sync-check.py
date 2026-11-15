#!/usr/bin/env python3
"""Fail when Rust code a formal model describes changed since it was confirmed.

A formal skill's SKILL.md frontmatter lists the functions its model covers
and a hash of their code:

    covered:
      - broker/src/engine/authority.rs set_paused set_runtime_pause
    covered_sha256: 0123456789abcdef

The hash is over each named function's text from `fn` to its closing brace,
with comments dropped and whitespace collapsed, so a reformat or a comment
does not fail the check and a squash or merge commit does not either. A
name that matches more than one `fn` in its file hashes every match.

    python3 scripts/formal-sync-check.py            check every formal skill
    python3 scripts/formal-sync-check.py --update   record the current hash,
                                                    after confirming the model
"""

from __future__ import annotations

import argparse
import functools
import hashlib
import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SKILLS = ".cursor/skills"
HASH_KEY = "covered_sha256"
HASH_LENGTH = 16
RAW_STRING = re.compile(r'b?r(#*)"')


class SyncError(Exception):
    pass


@dataclass
class Skill:
    path: Path
    covered: list[tuple[str, list[str]]]
    recorded: str | None


def frontmatter(text: str) -> list[str]:
    lines = text.splitlines()
    if not lines or lines[0] != "---":
        return []
    for index in range(1, len(lines)):
        if lines[index] == "---":
            return lines[1:index]
    return []


def read_skill(path: Path) -> Skill | None:
    lines = frontmatter(path.read_text(encoding="utf-8"))
    covered: list[tuple[str, list[str]]] = []
    recorded = None
    in_covered = False
    for line in lines:
        if line.startswith("covered:"):
            in_covered = True
            continue
        if in_covered and line.startswith("  - "):
            fields = line[4:].split()
            if len(fields) < 2:
                raise SyncError(f"{path}: `{line.strip()}` needs a file and at least one function name.")
            covered.append((fields[0], fields[1:]))
            continue
        in_covered = False
        if line.startswith(f"{HASH_KEY}:"):
            recorded = line.split(":", 1)[1].strip() or None
    if not covered:
        return None
    return Skill(path, covered, recorded)


@functools.lru_cache(maxsize=16)
def normalize_rust(source: str) -> tuple[str, tuple[bool, ...]]:
    """Drop comments and collapse whitespace outside literals.

    Returns the text and, per character, whether it is inside a string or
    character literal, so braces and `fn` there are not code.
    """
    out: list[str] = []
    literal: list[bool] = []

    def emit(text: str, is_literal: bool) -> None:
        out.extend(text)
        literal.extend([is_literal] * len(text))

    def space() -> None:
        if out and out[-1] != " ":
            emit(" ", False)

    index = 0
    length = len(source)
    while index < length:
        char = source[index]
        if source.startswith("//", index):
            end = source.find("\n", index)
            index = length if end == -1 else end
            space()
            continue
        if source.startswith("/*", index):
            depth = 0
            while index < length:
                if source.startswith("/*", index):
                    depth += 1
                    index += 2
                elif source.startswith("*/", index):
                    depth -= 1
                    index += 2
                    if depth == 0:
                        break
                else:
                    index += 1
            space()
            continue
        raw = RAW_STRING.match(source, index) if char in "br" else None
        if raw and (index == 0 or not (source[index - 1].isalnum() or source[index - 1] == "_")):
            close = '"' + raw.group(1)
            end = source.find(close, raw.end())
            end = length if end == -1 else end + len(close)
            emit(source[index:end], True)
            index = end
            continue
        if char == '"':
            end = index + 1
            while end < length and source[end] != '"':
                end += 2 if source[end] == "\\" else 1
            emit(source[index:end + 1], True)
            index = end + 1
            continue
        if char == "'":
            if index + 1 < length and source[index + 1] == "\\":
                end = index + 2
                while end < length and source[end] != "'":
                    end += 2 if source[end] == "\\" else 1
                emit(source[index:end + 1], True)
                index = end + 1
                continue
            if index + 2 < length and source[index + 2] == "'":
                emit(source[index:index + 3], True)
                index += 3
                continue
            emit(char, False)  # a lifetime
            index += 1
            continue
        if char.isspace():
            space()
            index += 1
            continue
        emit(char, False)
        index += 1
    return "".join(out), tuple(literal)


def functions(source: str, name: str) -> list[str]:
    """Every `fn name` item in the source, normalized, in file order."""
    text, literal = normalize_rust(source)
    found = []
    for match in re.finditer(rf"\bfn {re.escape(name)}\b", text):
        start = match.start()
        if literal[start]:
            continue
        depth = 0
        index = match.end()
        body = None
        while index < len(text):
            if not literal[index]:
                char = text[index]
                if char in "([":
                    depth += 1
                elif char in ")]":
                    depth -= 1
                elif depth == 0 and char == ";":
                    found.append(text[start:index + 1])
                    break
                elif depth == 0 and char == "{":
                    body = index
                    break
            index += 1
        if body is None:
            continue
        braces = 0
        for index in range(body, len(text)):
            if literal[index]:
                continue
            if text[index] == "{":
                braces += 1
            elif text[index] == "}":
                braces -= 1
                if braces == 0:
                    found.append(text[start:index + 1])
                    break
        else:
            raise SyncError(f"fn {name} has no closing brace")
    return found


def covered_hash(skill: Skill, root: Path) -> str:
    digest = hashlib.sha256()
    for file, names in skill.covered:
        path = root / file
        if not path.is_file():
            raise SyncError(f"{skill.path.relative_to(root)} covers {file}, which does not exist.")
        source = path.read_text(encoding="utf-8")
        for name in names:
            items = functions(source, name)
            if not items:
                raise SyncError(
                    f"{skill.path.relative_to(root)} covers `fn {name}` in {file}, which has no such function. "
                    "Update the model and its covered list to the renamed or removed code."
                )
            for item in items:
                digest.update(f"{file}\0{name}\0{item}\n".encode())
    return digest.hexdigest()[:HASH_LENGTH]


def formal_skills(root: Path) -> list[Skill]:
    skills = []
    for path in sorted((root / SKILLS).glob("*/SKILL.md")):
        skill = read_skill(path)
        if skill is not None:
            skills.append(skill)
    return skills


def record(skill: Skill, value: str) -> None:
    text = skill.path.read_text(encoding="utf-8")
    lines = text.split("\n")
    end = lines.index("---", 1)
    for index in range(1, end):
        if lines[index].startswith(f"{HASH_KEY}:"):
            lines[index] = f"{HASH_KEY}: {value}"
            break
    else:
        lines.insert(end, f"{HASH_KEY}: {value}")
    skill.path.write_text("\n".join(lines), encoding="utf-8")


def run(root: Path, update: bool) -> int:
    problems = []
    skills = formal_skills(root)
    for skill in skills:
        name = skill.path.relative_to(root)
        try:
            current = covered_hash(skill, root)
        except SyncError as error:
            problems.append(str(error))
            continue
        if update:
            if current != skill.recorded:
                record(skill, current)
                print(f"{name}: {HASH_KEY} {skill.recorded} -> {current}")
            continue
        if current != skill.recorded:
            files = " ".join(sorted({file for file, _ in skill.covered}))
            problems.append(
                f"{name}: code its formal model covers changed ({HASH_KEY} {skill.recorded}, now {current}).\n"
                f"  Read the change to the covered functions (git log -p -- {files}),\n"
                f"  follow \"When the code changes\" in {name}, then run\n"
                "  python3 scripts/formal-sync-check.py --update"
            )
    if problems:
        print("\n\n".join(problems), file=sys.stderr)
        return 1
    if not update:
        print(f"Formal models match their covered code ({len(skills)} skill{'s' if len(skills) != 1 else ''}).")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--update", action="store_true", help="record the current hash in each formal skill")
    parser.add_argument("--root", type=Path, default=ROOT, help=argparse.SUPPRESS)
    args = parser.parse_args()
    return run(args.root.resolve(), args.update)


if __name__ == "__main__":
    sys.exit(main())
