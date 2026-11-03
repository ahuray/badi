# Suggestion length

Suggestion length chooses how many words a continuation may show. Normal, the default, shows up to four. Short shows one or two. The popout's Suggestion length row sets the same choice. In All settings that row sits under Writing; the compact popout shows the row with no section header.

## Sub-features

- `length-normal` shows up to four words.
- `length-short` shows one or two words.
- `length-ignored` leaves the phrase fixture, a spelling correction, and a vocabulary completion at their own length.

## How to get to it (user POV)

- Run `badi length short`. The next local-model suggestion is one or two words.
- Run `badi length normal`. Suggestions may be up to four words again.
- In the Omarchy popout, set Suggestion length to Short or Normal.

## Driving it with verify-badi

Preconditions:

- Headless: doctor has exited 0 for this run. No session is open.
- The phrase fixture ignores `suggestion_length`. `thank you` still completes as ` for your time` after short is set. That is the fixture rule, not a failed short suggestion.
- A shorter grey suggestion needs the local model on a desktop. This VM cannot show it.

- **Set short and read it back.** Run `.cursor/skills/verify-badi/verify-badi.sh drive length`. Settings gain `suggestion_length` `short`. A following Bash-bridge suggestion is still ` for your time`. The drive then restores normal, and settings omit `suggestion_length`. Helper exit code is 0.
- **Desktop model.** With the installed local model, run `badi length short`, type disposable text, and capture the grey text of one or two words. `badi length normal` allows up to four again. A screenshot is the proof. Do not cite the phrase fixture as that proof.

## Gotchas

- Settings changes retire live sessions. A type-through carry does not survive a length change.
- Spelling replacement is still taken whole.
- Up to eight words was tried and removed. There is no `long` setting to drive.
