#!/usr/bin/env python3
"""Fail when a formal spec's covered code has drifted from the commit it names.

A skill is formal when it lives in `.cursor/skills/*formal*/SKILL.md`, or
when its frontmatter sets `formal: true`. The frontmatter is a small YAML
subset: scalar `key: value` lines, `#` comments, and indented `- item` lists.

    sync_commit: <40-character commit the model was confirmed against>
    covered:
      - path/the/model/describes
    spec:
      - path/to/Model.tla

`covered` is required. `spec` defaults to every `*.tla` and `*.cfg` next to
the skill. `check.sh` in that directory is the TLC (or other) run. It is not
spec: changing the runner schedules a rerun and does not excuse a code edit.

The working tree's covered files must match that sync commit, including a
rename or a deletion. A commit in between may change a covered file only when
that same commit also changes a spec file. Moving `sync_commit` to the commit
you confirmed makes the trees match again; a later spec-only commit does not
excuse an earlier code change.

    python3 scripts/formal-sync-check.py
    python3 scripts/formal-sync-check.py --changed-since REV
    python3 scripts/formal-sync-check.py --changed-since ALL

The second form prints each affected skill's `check.sh`, one per line, when
REV...HEAD (or the working tree) touches a covered file, a spec file, or that
`check.sh`. ALL prints every formal `check.sh`.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

SHA = re.compile(r"[0-9a-fA-F]{40}")
MARKER = {"formal: true", "formal: yes"}


class SkillLoadError(Exception):
    pass


class GitError(Exception):
    pass


class Skill:
    def __init__(self, name, skill_md, sync_commit, covered, spec, check):
        self.name = name
        self.skill_md = skill_md
        self.sync_commit = sync_commit
        self.covered = tuple(covered)
        self.spec = tuple(spec)
        self.check = check


class Report:
    def __init__(self, failures, summary):
        self.failures = failures
        self.summary = summary


def run(root, args, check=True):
    result = subprocess.run(
        ["git", "-C", str(root), "-c", "core.quotepath=false", "-c", "diff.renames=true", *args],
        check=False,
        capture_output=True,
        text=True,
    )
    if check and result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or f"exit {result.returncode}"
        raise GitError(f"git {' '.join(args)} failed: {detail}")
    return result


def split_z(raw):
    if not raw:
        return []
    parts = raw.split("\0")
    if parts[-1] == "":
        parts.pop()
    return parts


def diff_entries(root, *args):
    result = run(root, ["diff", "-z", "-M", "--name-status", *args])
    parts = split_z(result.stdout)
    entries = []
    index = 0
    while index < len(parts):
        status = parts[index]
        index += 1
        if index >= len(parts):
            raise GitError(f"truncated git diff near {status!r}")
        path = parts[index]
        index += 1
        destination = None
        if status[:1] in {"R", "C"}:
            if index >= len(parts):
                raise GitError(f"truncated rename record for {path}")
            destination = parts[index]
            index += 1
        entries.append((status[:1], path, destination))
    return entries


def untracked(root):
    return split_z(run(root, ["ls-files", "--others", "--exclude-standard", "-z"]).stdout)


def paths_of(entries):
    found = set()
    for _status, path, destination in entries:
        found.add(path)
        if destination:
            found.add(destination)
    return found


def touches(entries, watched):
    watched = set(watched)
    return any(path in watched for path in paths_of(entries))


def covered_entries(entries, covered):
    covered = set(covered)
    return [
        entry
        for entry in entries
        if entry[1] in covered or (entry[2] is not None and entry[2] in covered)
    ]


def blob_at(root, revision, path):
    result = run(root, ["cat-file", "-t", f"{revision}:{path}"], check=False)
    return result.returncode == 0 and result.stdout.strip() == "blob"


def commit_exists(root, sha):
    result = run(root, ["cat-file", "-t", sha], check=False)
    return result.returncode == 0 and result.stdout.strip() == "commit"


def is_ancestor(root, ancestor, descendant):
    result = run(root, ["merge-base", "--is-ancestor", ancestor, descendant], check=False)
    if result.returncode == 0:
        return True
    if result.returncode == 1:
        return False
    detail = result.stderr.strip() or f"exit {result.returncode}"
    raise GitError(detail)


def frontmatter_block(text):
    if not text.startswith("---\n"):
        return None
    end = text.find("\n---\n", 4)
    if end == -1:
        return None
    return text[4:end]


def marker_in(block):
    if block is None:
        return False
    for raw in block.splitlines():
        if raw.strip().lower() in MARKER:
            return True
    return False


def unquote(value):
    if len(value) >= 2 and value[0] == value[-1] and value[0] in {"'", '"'}:
        return value[1:-1]
    return value


def parse_block(block, label):
    data = {}
    current = None
    for raw in block.splitlines():
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if stripped.startswith("- "):
            if current is None or not isinstance(data.get(current), list):
                raise SkillLoadError(f"{label}: list item {stripped!r} is not under a list key")
            item = stripped[2:].strip()
            if not item:
                raise SkillLoadError(f"{label}: empty list item under {current}")
            data[current].append(unquote(item))
            continue
        if raw[:1] in {" ", "\t"}:
            raise SkillLoadError(f"{label}: cannot parse frontmatter line {raw!r}")
        if ":" not in stripped:
            raise SkillLoadError(f"{label}: cannot parse frontmatter line {raw!r}")
        key, value = stripped.split(":", 1)
        key = key.strip()
        value = unquote(value.strip())
        if not key:
            raise SkillLoadError(f"{label}: cannot parse frontmatter line {raw!r}")
        if value == "":
            data[key] = []
            current = key
        else:
            data[key] = value
            current = None
    return data


def clean_path(name, kind, value):
    if not isinstance(value, str) or not value or value.startswith("/") or "\\" in value:
        raise SkillLoadError(f"{name}: {kind} path {value!r} must be a repository-relative path")
    path = Path(value)
    if path.is_absolute() or ".." in path.parts or "." in path.parts:
        raise SkillLoadError(f"{name}: {kind} path {value} escapes the repository")
    return path.as_posix()


def unique(items):
    seen = []
    for item in items:
        if item not in seen:
            seen.append(item)
    return seen


def default_spec(root, directory):
    found = []
    for pattern in ("*.tla", "*.cfg"):
        for file in sorted(directory.glob(pattern)):
            found.append(file.relative_to(root).as_posix())
    return found


def parse_skill(root, path, text):
    relative = path.relative_to(root).as_posix()
    block = frontmatter_block(text)
    if block is None:
        raise SkillLoadError(f"{relative}: missing frontmatter")
    data = parse_block(block, relative)
    name = data.get("name")
    if not isinstance(name, str) or not name:
        name = path.parent.name
    sync = data.get("sync_commit", "")
    if not isinstance(sync, str) or SHA.fullmatch(sync) is None:
        raise SkillLoadError(
            f"{name}: frontmatter sync_commit must be the 40-character commit the model was confirmed against"
        )
    covered = data.get("covered")
    if not isinstance(covered, list) or not covered:
        raise SkillLoadError(f"{name}: frontmatter needs a covered list of repository paths")
    covered = unique(clean_path(name, "covered", item) for item in covered)
    if "spec" not in data:
        spec = default_spec(root, path.parent)
    elif not isinstance(data["spec"], list):
        raise SkillLoadError(f"{name}: spec must be a list of repository paths")
    else:
        spec = unique(clean_path(name, "spec", item) for item in data["spec"])
    for item in spec:
        if not (root / item).is_file():
            raise SkillLoadError(f"{name}: spec file {item} is not in the working tree")
    check = (path.parent / "check.sh").relative_to(root).as_posix()
    if not (root / check).is_file():
        raise SkillLoadError(f"{name}: missing {check}")
    return Skill(name, relative, sync.lower(), covered, spec, check)


def load_skills(root):
    skills_dir = root / ".cursor" / "skills"
    skills = []
    errors = []
    if not skills_dir.is_dir():
        return skills, errors
    for path in sorted(skills_dir.glob("*/SKILL.md")):
        text = path.read_text(encoding="utf-8")
        if "formal" not in path.parent.name and not marker_in(frontmatter_block(text)):
            continue
        try:
            skills.append(parse_skill(root, path, text))
        except SkillLoadError as error:
            errors.append(str(error))
    return skills, errors


def net_entries(root, skill):
    entries = covered_entries(diff_entries(root, skill.sync_commit), skill.covered)
    seen = paths_of(entries)
    for path in untracked(root):
        if path in skill.covered and path not in seen:
            entries.append(("A", path, None))
            seen.add(path)
    return entries


def existence_gaps(root, skill, entries):
    seen = paths_of(entries)
    gaps = []
    for path in skill.covered:
        if path in seen:
            continue
        at_sync = blob_at(root, skill.sync_commit, path)
        at_work = (root / path).is_file()
        if at_sync and at_work:
            continue
        if not at_sync and not at_work:
            gaps.append(f"{path} (not a file at sync_commit or in the working tree)")
        elif not at_sync:
            gaps.append(f"{path} (added)")
        else:
            gaps.append(f"{path} (deleted)")
    return gaps


def describe(entry, covered):
    status, path, destination = entry
    covered = set(covered)
    if status in {"R", "C"} and destination:
        if path in covered:
            return f"{path} (renamed to {destination})"
        return f"{destination} (renamed from {path})"
    if status == "D":
        return f"{path} (deleted)"
    if status == "A":
        return f"{path} (added)"
    if status in {"M", "T"}:
        return f"{path} (modified)"
    return f"{path} (changed, {status})"


def descriptions(skill, entries, gaps):
    lines = []
    for line in [describe(entry, skill.covered) for entry in entries] + list(gaps):
        if line not in lines:
            lines.append(line)
    return lines


def first_parent(root, commit):
    result = run(root, ["rev-parse", "--verify", "--quiet", f"{commit}^"], check=False)
    if result.returncode == 0:
        return result.stdout.strip()
    return run(root, ["hash-object", "-t", "tree", "/dev/null"]).stdout.strip()


def worktree_entries(root):
    entries = diff_entries(root, "HEAD")
    entries.extend(("A", path, None) for path in untracked(root))
    return entries


def unexcused_reasons(root, skill):
    reasons = []
    excused = False
    listed = run(root, ["rev-list", "--reverse", f"{skill.sync_commit}..HEAD"]).stdout
    for commit in [line for line in listed.splitlines() if line]:
        entries = diff_entries(root, first_parent(root, commit), commit)
        if not touches(entries, skill.covered):
            continue
        if touches(entries, skill.spec):
            excused = True
            continue
        subject = run(root, ["log", "-1", "--format=%H %s", commit]).stdout.strip()
        reasons.append(subject)
    work = worktree_entries(root)
    if touches(work, skill.covered):
        if touches(work, skill.spec):
            excused = True
        else:
            reasons.append("working tree")
    if not reasons and not excused:
        reasons.append("covered files differ, and no commit updates a spec file")
    return reasons


def fix_line(skill):
    return (
        f"Fix: run {skill.check}, follow /maintain-formal-verification-skill, "
        f"then move the sync commit in {skill.skill_md} to the commit you confirmed."
    )


def format_failure(skill, changed, reasons):
    lines = [
        f"formal-sync-check: {skill.name} drifted from sync commit {skill.sync_commit}.",
        "Covered files that changed:",
    ]
    lines.extend(f"  {item}" for item in changed)
    if reasons:
        lines.append("This change did not update the sync commit or a spec file:")
        lines.extend(f"  {item}" for item in reasons)
    lines.append(fix_line(skill))
    return "\n".join(lines)


def format_pointer(skill, problem):
    return f"formal-sync-check: {skill.name} {problem}\n{fix_line(skill)}"


def check_skill(root, skill):
    if not commit_exists(root, skill.sync_commit):
        return format_pointer(
            skill,
            f"sync_commit {skill.sync_commit} is not a commit in this clone. Fetch the full history.",
        ), None
    if not is_ancestor(root, skill.sync_commit, "HEAD"):
        return format_pointer(
            skill,
            f"sync_commit {skill.sync_commit} is not an ancestor of HEAD.",
        ), None
    entries = net_entries(root, skill)
    gaps = existence_gaps(root, skill, entries)
    reasons = unexcused_reasons(root, skill) if entries else []
    if gaps or reasons:
        return format_failure(skill, descriptions(skill, entries, gaps), reasons), None
    if not entries:
        return None, None
    return None, (
        f"{skill.name}: covered files differ from {skill.sync_commit}, "
        "excused because each of those commits also updates a spec file."
    )


def evaluate(root):
    head = run(root, ["rev-parse", "--verify", "--quiet", "HEAD"], check=False)
    if head.returncode != 0:
        raise GitError("this repository has no commits")
    skills, errors = load_skills(root)
    failures = [f"formal-sync-check: {error}" for error in errors]
    notes = []
    for skill in skills:
        failure, note = check_skill(root, skill)
        if failure:
            failures.append(failure)
        elif note:
            notes.append(note)
    noun = "skill" if len(skills) == 1 else "skills"
    summary = f"formal-sync-check: ok, {len(skills)} formal {noun}"
    if notes and not failures:
        summary = summary + "\n" + "\n".join(notes)
    return Report(failures, summary)


def resolve_commit(root, revision):
    result = run(root, ["rev-parse", "--verify", "--quiet", f"{revision}^{{commit}}"], check=False)
    if result.returncode != 0:
        raise GitError(f"--changed-since {revision} is not a commit in this repository")
    return result.stdout.strip()


def checks_changed_since(root, since):
    skills, errors = load_skills(root)
    if errors:
        raise GitError("\n".join(f"formal-sync-check: {error}" for error in errors))
    if since == "ALL":
        return [skill.check for skill in skills]
    resolve_commit(root, since)
    changed = paths_of(diff_entries(root, f"{since}...HEAD"))
    changed.update(paths_of(worktree_entries(root)))
    selected = []
    for skill in skills:
        watched = set(skill.covered) | set(skill.spec) | {skill.check}
        if changed & watched:
            selected.append(skill.check)
    return selected


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--root", type=Path, help="repository root (default: this script's repo)")
    parser.add_argument(
        "--changed-since",
        metavar="REV",
        help="print check.sh paths whose covered files, spec, or check.sh differ since REV (three-dot). ALL prints every formal check.sh",
    )
    args = parser.parse_args(argv)
    root = (args.root if args.root is not None else Path(__file__).resolve().parents[1]).resolve()
    try:
        if args.changed_since:
            paths = checks_changed_since(root, args.changed_since)
        else:
            report = evaluate(root)
            if report.failures:
                sys.stderr.write("\n\n".join(report.failures) + "\n")
                return 1
            sys.stdout.write(report.summary + "\n")
            return 0
    except GitError as error:
        message = str(error)
        if not message.startswith("formal-sync-check:"):
            message = f"formal-sync-check: {message}"
        sys.stderr.write(message + "\n")
        return 1
    sys.stdout.write("".join(f"{path}\n" for path in paths))
    return 0


if __name__ == "__main__":
    sys.exit(main())
