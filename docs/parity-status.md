# Parity status, 12 October 2026

This is a snapshot of the Cotypist-parity work, for review. It is not a
second backlog. Open work stays in [future plans](<../future plans.md>).
What a person can do today stays in the [README](../README.md). The product
contract stays in [VISION](../VISION.md).

`main` at this snapshot is `7d80b37`. The desktop on this machine was
installed from the tree of `37b2bf2` (the type-through wait). The later
note in `7d80b37` is documentation only. Predictions were left on.

The parity plan is not finished. Nothing below is a held-out quality score,
and a tested app version is not a claim about Linux in general.

## What shipped

| Phase | Result | Merge |
| --- | --- | --- |
| 0. Green main | Formatting and the healing-prompt test match CI | [#33](https://github.com/ahuray/badi/pull/33) `8b7476d` |
| 1. Review defects from #32 | Preview stays inside a real field edge. English healing honors its switch and does not abstain on one unknown word plus a space. Lab keystrokes group by config and trace and count whole words. Healed Lab mode matches production echo. Dead Lab labels are gone. Docs say `llama-<pid>.sock`. The model directory may be missing (`ReadOnlyPaths=-`). Memory caps fit the 4B tier. Persian stays at 8 tokens | [#34](https://github.com/ahuray/badi/pull/34) `230b1fa` |
| 2. Mid-sentence | Off by default. A word after the caret asks for nothing before any model call. Punctuation can still finish the word being typed. Production rejoin was not changed | [#35](https://github.com/ahuray/badi/pull/35) `f39f5c0` |
| 3. Quality harness | Lab frame `everyday-english-2026-10-12.v1`, plus framed and field-context modes. No model or prompt was promoted | [#36](https://github.com/ahuray/badi/pull/36) `88454bb` |
| 4–5. Confidence and instructions | Lab-only. Thresholds en −2.0, de −0.7, fa −2.0 are not in production `writing.rs`. No production instructions | not shipped |
| 6. Speed | First grey word is shown while the rest generates. A known field starts the model on the key and holds display for the 120 ms pause. See the speed table | [#40](https://github.com/ahuray/badi/pull/40)–[#54](https://github.com/ahuray/badi/pull/54) |
| 7. Typo replacement | Exact replacement only where an editor can replace text. IME-parity apps stay append-only. Result stays `dispatched-unverified` | [#37](https://github.com/ahuray/badi/pull/37) `f78b4cb` |
| 8. Host font and emoji | Grey text uses the host font when accessibility exposes it. Fcitx already has emoji; Badi does not add a `:shortcode` expander | [#38](https://github.com/ahuray/badi/pull/38) `1d6cd9a` |
| 9–11. Learning, context, packaging | Not built. See decisions below | — |
| 12. Release qualification | Not started | — |

## Speed, on this machine

Warm first grey word, five disposable prefixes, four repeats, predictions
on, build `5faafcf`, 12 October 2026. This is one machine and these
prefixes, not a universal p95.

| App | Shown | Range | Middle | 19th of 20 |
| --- | --- | --- | --- | --- |
| Omawrite 0.5.0 | 20/20 | 209–250 ms | 220–223 ms | 244 ms |
| Chromium textarea | 20/20 | 227–371 ms | 246–249 ms | 314 ms |

Closed after measurement, not shipped:

- Six threads were not faster. The cap stays 4.
- Q4_0 did not replace the pinned Q4_K_M ([#57](https://github.com/ahuray/badi/pull/57)).
- N-gram speculation and a Qwen3-0.6B draft did not bring the first word forward ([#58](https://github.com/ahuray/badi/pull/58)).
- Naming the line after the caret (`following_text`, Lab only) did not beat today's rejoin. On 41 English development cases it missed 550 ms on 34 of 41 ([#59](https://github.com/ahuray/badi/pull/59)).

## Other behavior now on main

- The settings panel polls only while its window is open ([#64](https://github.com/ahuray/badi/pull/64)).
- The grey preview follows a scroll of the armed field or a scroll-pane ancestor when the text offset is unchanged ([#61](https://github.com/ahuray/badi/pull/61)–[#63](https://github.com/ahuray/badi/pull/63)).
- A toolkit password mark is refused. Labels, placeholders, and HTML autocomplete hints are not treated as sensitive ([#60](https://github.com/ahuray/badi/pull/60)).
- Bash's Readline timer runs only while a grey preview is held ([#65](https://github.com/ahuray/badi/pull/65)).
- The headless Obsidian lane signals only the app process, so it does not trip Electron's crash helper ([#56](https://github.com/ahuray/badi/pull/56)).
- Omawrite 0.5.0 spelling undo is two native undos (`teh ` then `the `). Obsidian replaces the suffix in one undo step.
- LibreOffice Writer 26.8 showed a preview after `Thank you`. Calc showed none within 3 seconds. Paragraph text was not read back.

## Type-through, still open

On a disposable Brave Origin page, `Thank you` showed grey `for the opportunity`. The addon counted the following space as type-through, and the field became `Thank you `. The broker still made another model call. After the wait in [#66](https://github.com/ahuray/badi/pull/66) was installed, the same trial still made a second model call, and that wait did not run: the field already contained the space when the request went out ([#67](https://github.com/ahuray/badi/pull/67)).

A fresh Zen profile exposed only the window frame, so its page was not typed in.

## Decisions already recorded

- Context beyond the field may later be the app name, the window title, or that window's accessibility text. Not one design yet. Anything except app-name-only needs a VISION amendment. Not built.
- Learning, if built, is opt-in and local, from accepted suggestions or a chosen writing style, never raw typing. It still needs a held-out comparison before promotion. Not built.
- Clipboard awareness is listed. It is not an opt-in grant. The clipboard is not scraped.
- Distribution is AUR, then Omarchy. That is publication. Not published.
- A quality change is promoted only with no per-language harm increase.
- The held-out set must be writing you provide or approve: at least 100 English, 40 German, and 40 Persian prefixes. It has not been supplied. No model, prompt, confidence threshold, or production instruction was promoted ahead of it.

## Still in the way of the definition of done

- Held-out usefulness and harm, per language, on that frozen set.
- A wider speed sample than the five prefixes above.
- A live matrix with zero wrong-field or stale edits. Telegram and Xournal++ spelling undo are not verified. Telegram's host font is not checked.
- Instructions, a strength setting, and learning.
- One install command on a fresh Omarchy, with a clean uninstall.

Dependabot pull requests [#30](https://github.com/ahuray/badi/pull/30) and [#31](https://github.com/ahuray/badi/pull/31) target `develop`. They were not merged.
