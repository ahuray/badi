#!/usr/bin/env python3
"""Lab-only: read the focused window's visible text with OCR, the way a
future window-context source could, and measure what it costs.

The focused window alone is captured (Hyprland geometry, `grim`), recognised
in memory with Tesseract and dropped; nothing is written unless `--output`
names a file. By default only timings and sizes are printed, never text.
`--expect FILE` scores the recognition against the text known to be shown
(character error rate), for disposable trial windows.

This is not installed and no product path calls it. Reading the screen
needs its own decision (VISION's context firewall allows only same-window
accessibility text today).
"""

import argparse
import json
import subprocess
import sys
import time


def focused_window():
    window = json.loads(subprocess.run(["hyprctl", "activewindow", "-j"], capture_output=True,
                                       text=True, check=True, timeout=3).stdout)
    if not window or "at" not in window:
        raise RuntimeError("no focused window")
    return window


def capture(window, scale):
    (x, y), (width, height) = window["at"], window["size"]
    command = ["grim", "-g", f"{x},{y} {width}x{height}", "-t", "png"]
    if scale:
        command += ["-s", str(scale)]
    return subprocess.run([*command, "-"], capture_output=True, check=True, timeout=5).stdout


def recognise(png, languages):
    # Page segmentation 4: one column of text of variable sizes, which fits
    # terminals, chats and documents better than full-page layout analysis.
    return subprocess.run(["tesseract", "stdin", "stdout", "-l", languages, "--psm", "4"],
                          input=png, capture_output=True, check=True, timeout=20).stdout.decode()


def tail(text, limit):
    """The last `limit` characters of non-empty lines: the text nearest a
    terminal's prompt or a chat's composer, which sit at the bottom."""
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    kept = " ".join(lines)
    return kept[-limit:]


def error_rate(recognised, expected):
    """Character error rate: Levenshtein distance over the expected length,
    with whitespace collapsed on both sides."""
    a, b = " ".join(recognised.split()), " ".join(expected.split())
    previous = list(range(len(b) + 1))
    for i, left in enumerate(a, 1):
        current = [i]
        for j, right in enumerate(b, 1):
            current.append(min(previous[j] + 1, current[j - 1] + 1, previous[j - 1] + (left != right)))
        previous = current
    return previous[-1] / max(1, len(b))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--languages", default="eng", help="Tesseract languages, e.g. eng+deu")
    parser.add_argument("--limit", type=int, default=800, help="characters of window text kept (default 800)")
    parser.add_argument("--expect", help="file with the text known to be shown, to score recognition")
    parser.add_argument("--output", help="write the kept text here (disposable trials only)")
    parser.add_argument("--delay", type=float, default=0, help="seconds to wait before capturing")
    parser.add_argument("--scale", type=float, help="capture scale (default: the output's own, e.g. 2 on HiDPI)")
    args = parser.parse_args()
    time.sleep(args.delay)
    started = time.monotonic()
    window = focused_window()
    png = capture(window, args.scale)
    captured = time.monotonic()
    text = recognise(png, args.languages)
    recognised = time.monotonic()
    kept = tail(text, args.limit)
    report = {
        "window_class": window.get("class"),
        "size": window["size"],
        "capture_ms": round((captured - started) * 1000),
        "ocr_ms": round((recognised - captured) * 1000),
        "png_bytes": len(png),
        "recognised_chars": len(text),
        "kept_chars": len(kept),
    }
    if args.expect:
        with open(args.expect, encoding="utf-8") as expected:
            report["character_error_rate"] = round(error_rate(" ".join(text.split()), expected.read()), 3)
    if args.output:
        with open(args.output, "w", encoding="utf-8") as output:
            output.write(kept)
    json.dump(report, sys.stdout)
    print()


if __name__ == "__main__":
    main()
