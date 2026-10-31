# Prediction Lab

Prediction Lab compares continuations and word completion on a draft you type. Expected answers are for scoring only. The Lab does not read the screen, edit another app, or change the installed broker. The page has two tabs: Prediction and Word completion. There is no Spelling tab; production spelling is scored inside a Prediction run that uses current Badi logic.

## Sub-features

- `lab-open` serves the page on localhost and shows Prediction and Word completion.
- `lab-predict` runs the draft on Prediction and shows the addition beside the expected line. It needs the pinned model files.
- `lab-complete` finishes one word from text you supply, without loading the language model.
- `lab-device` reports the machine without launching a model.
- `lab-missing-model` fails a prediction when the pinned model files are absent. It does not fall back to the phrase fixture.

## How to get to it (user POV)

- From the checkout, run `npm run writing:lab`. Open the printed `http://127.0.0.1:PORT` URL.
- On Prediction, enter text before the caret and one acceptable addition per line, then run.
- Word completion is the other tab. It searches the text you supply and does not load a language model.

## Driving it with verify-badi

Preconditions:

- The Lab worker starts its own model process. Missing model files are an error, not a silent fixture.
- `--inspect-device` is read-only and does not download or launch a model.
- The phrase broker from Launch is not the Lab. Do not send a Lab draft to `verify-badi.sh drive`.

- **Device only.** Run `cargo build --release --locked -p badi-writing-lab`, then `target/release/badi-writing-lab --inspect-device`. Exit 0 and JSON that describes the machine, with no draft stored. This does not prove `lab-predict`.
- **Page.** Run `npm run writing:lab -- --port 36889`. Exit is the server staying up. A request to `http://127.0.0.1:36889` returns the page, and its tabs are Prediction and Word completion. Word completion of the page's English example (`Please send the updated sched`, context `The schedule is ready.`) returns the suffix `ule` and does not need model files. A Prediction run with no pinned model files answers on the open stream with `Model startup failed: runtime_start_failed`. That stream can still be HTTP 200. It is not a phrase-fixture suggestion and it is not `lab-predict`. Stop the server with Ctrl+C. Do not point the page at the phrase broker and do not treat `npm run writing:lab:check` as the page.

## Gotchas

- A single Lab run is not a before-and-after comparison. Quality claims need the same suites, settings, and seed twice.
- Discovery search contacts the public model hub. A proof that only runs `--inspect-device` did not touch the hub; do not describe it as if it had.
- Drafts stay in the page and the server until you clear them or stop the process. Do not put a real private draft in a shared log.
- The server binds to `127.0.0.1` only. Another run needs its own port.
