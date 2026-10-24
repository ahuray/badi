# Prediction Lab

Prediction Lab compares continuations, word completion, and spelling on a draft you type. Expected answers are for scoring only. The Lab does not read the screen, edit another app, or change the installed broker.

## Sub-features

- `lab-open` serves the page on localhost and shows the three tabs.
- `lab-predict` runs the draft and shows the addition beside the expected line.
- `lab-device` reports the machine without launching a model.
- `lab-missing-model` fails closed when the pinned model files are absent.

## How to get to it (user POV)

- From the checkout, run `npm run writing:lab`. Open the printed `http://127.0.0.1:PORT` URL.
- On Prediction, enter text before the caret and one acceptable addition per line, then run.
- Word completion and Spelling are the other two tabs. Spelling needs a local dictionary manifest or it has nothing to check.

## Driving it with verify-badi

Preconditions:

- The Lab worker starts its own model process. Missing model files are an error, not a silent fixture.
- `--inspect-device` is read-only and does not download or launch a model.
- The phrase broker from Launch is not the Lab. Do not send a Lab draft to `verify-badi.sh drive`.

- **Device only.** Run `cargo build --release --locked -p badi-writing-lab`, then `target/release/badi-writing-lab --inspect-device`. Exit 0 and JSON that describes the machine, with no draft stored. This does not prove `lab-predict`.
- **Page.** Run `npm run writing:lab -- --port 36889`. Exit is the server staying up. A browser request to `http://127.0.0.1:36889` returns the page. Run one disposable draft and record the addition the page shows. Stop the server with Ctrl+C. If the worker exits because model files are missing, the journey fails for that reason. Do not point the page at the phrase broker and do not treat `npm run writing:lab:check` as the page.

## Gotchas

- A single Lab run is not a before-and-after comparison. Quality claims need the same suites, settings, and seed twice.
- Discovery search contacts the public model hub. A proof that only runs `--inspect-device` did not touch the hub; do not describe it as if it had.
- Drafts stay in the page and the server until you clear them or stop the process. Do not put a real private draft in a shared log.
- The server binds to `127.0.0.1` only. Another run needs its own port.
