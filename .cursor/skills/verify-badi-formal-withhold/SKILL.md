---
name: verify-badi-formal-withhold
description: Checks Badi's pause, snooze and screen-share withhold ordering with TLA+ (TLC) and the broker withhold_ tests. Use when a change touches runtime pause, settings pause, desktop hold, screen-share grace, snooze, ambiguous_session, or whether a suggestion may be admitted.
# The functions Withhold.tla describes (the Coverage table below).
# scripts/formal-sync-check.py hashes their code; see "When the code changes".
covered:
  - broker/src/engine/authority.rs set_paused set_runtime_pause settings_allow_data policy_status
  - broker/src/engine/settings.rs replace_settings begin_control_plane_mutation install_settings_authority install_settings
  - broker/src/engine/mod.rs close_all_sessions runtime_paused effective_paused session_with_data_access
  - broker/src/engine/desktop.rs report_desktop_state finish_desktop_transition forget_desktop_state end_share_grace desktop_hold screen_share
  - broker/src/engine/session.rs open_session close_session update_context revoke_context_authority
  - broker/src/engine/generation.rs request_suggestion admit_generation
  - broker/src/engine/snooze.rs snooze_focused focused_subject end_snoozes expire_snoozes withheld_reason
  - broker/src/protocol.rs holds
  - broker/src/settings/policy.rs allows_context_read allows_display allows_suggestion
covered_sha256: 4dc83ca67a0a75c2
---

# Verify withhold ordering

A settings pause, a runtime pause, a desktop hold and a snooze each withdraw suggestions. Ending one does not lift the others, and a snooze records nothing when the broker is already paused or the focused fields belong to different apps or sites.

The harm: grey text, which can carry words from the window, appears on a shared screen, or in an app or site the user paused or snoozed.

This is the ordering to check. The suggestion and commit lifecycle, and the alternatives cycle, are separate machines with their own tests. Do not treat a pass here as a pass for those.

## Run

From the repository root:

```sh
.cursor/skills/verify-badi-formal-withhold/check.sh
```

Exit 0 means TLC found no counterexample and every `withhold_` test in `broker/src/engine/tests/withhold.rs` passed. The script prints a TLC summary and a final line `TLC exit N; cargo test exit N`. Any other exit is a failure. A TLC "No error has been found" line is not success on its own.

The checker is TLA+ 1.7.4, TLC 2.19 of 08 August 2024, jar sha256 `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`. The script downloads that jar into the user cache and refuses a different hash. It needs `java` 17 or newer (checked on OpenJDK 17 LTS and 21 LTS; Debian 13 and Ubuntu 24.04 package `openjdk-21-jre-headless`) and `curl`. It does not install Rust.

The model is [Withhold.tla](Withhold.tla) with [Withhold.cfg](Withhold.cfg). TLC is the tool because the question is whether these withholds can happen in a bad order. The broker test calls the same functions on a running broker, so a broken guard fails even when the model is unchanged.

## Read a result

A counterexample is a numbered list of states. Each state after the first names the action that produced it. Read it from the first state. For each step, write down the action, the variables it changed (names below) and the function in [Coverage](#coverage) that takes that step. Then decide which of the cases below the trace is.

State names in a trace:

| TLC name | In the broker |
| --- | --- |
| `user_paused` | `BrokerState.paused`, written by `set_runtime_pause` |
| `settings_paused` | `SettingsV2.paused` |
| `screen_mode` | `SettingsV2.screen_share` (`hold` or `suggest`) |
| `helper[h]` | that connection's last `DesktopStatePayload`; `absent` means no report |
| `share_grace` | `BrokerState.share_grace` is `Some` |
| `snooze[id]` | that app or site is still in `BrokerState.snoozes` |
| `phase[s]` | `closed` (no session), `open` (no context), `focused` (`context.field.focused`) |
| `shown[s]` | `admit_generation` admitted a suggestion that `revoke_context_authority` has not withdrawn |
| `sa` / `mail`, `sb` / `docs` | the two sites |
| `inactive` | `DesktopSession::Inactive` or `Sleeping` (both `holds`) |

A counterexample is a product bug when every step is an action in the table below and the named function can take it. Say the steps in those function names, and the harm. Leave the model as it is. Do not change product code unless the user asked. Model upkeep ([When the code changes](#when-the-code-changes)) is its own change, separate from a product fix.

A counterexample is a modeling error when a step cannot happen in that function: the code's guard would refuse it, the code does not take that step, or the step is outside the bounds below. Fix the model so it describes the code, then rerun. Do not drop a conjunct from `SuggestionsStayWithheld`, `OneWithholdDoesNotLiftAnother` or `SnoozeStartsOnlyForOneClearSubject` to get a pass.

TLC passing while a `withhold_` test fails means the running broker broke the promise the model still describes. That is a product finding, reported the same way.

## When the code changes

Confirm the model still describes the functions in [Coverage](#coverage). A green `check.sh` is not that confirmation.

1. Read the diff of every function in Coverage since the confirmed commit: `git diff <confirmed>..HEAD -- <files>`.
2. For each change, ask whether it adds or drops a withhold, changes a guard on opening a session, reading context or admitting a suggestion, or lets one withhold end another. A change that does none of these (a refactor, a log line, proposal shape) needs no model edit.
3. If the model no longer matches, edit [Withhold.tla](Withhold.tla), [Withhold.cfg](Withhold.cfg), the Coverage and Bounds sections and the `withhold_` tests so they describe the code. Do not edit product code here, and do not weaken a property to pass.
4. Run `check.sh` and update the state count in [Bounds](#bounds).
5. Move the confirmed commit in Coverage to the commit you read, and run `python3 scripts/formal-sync-check.py --update` to record the hash of the code you confirmed.

`npm run formal:check` (part of `npm run check`) fails when the code of a function in the frontmatter `covered` list no longer has the recorded `covered_sha256`. The hash ignores comments and whitespace, and it does not depend on commit ids, so a squash or merge commit does not trip it. It also fails when a covered function is renamed or removed. Keep the list and the Coverage table naming the same functions. The CI job "Formal model / TLC" runs `check.sh --tlc-only` on every change; the Rust job already runs the `withhold_` tests.

## Coverage

Confirmed against commit `bb9aec21299e6d148c6c6d1464daacbaa3399ef6`. The model describes these functions as of that commit. The `withhold_` tests call them.

| TLC action or operator | Code |
| --- | --- |
| `SetUserPause` | `Broker::set_paused`, `BrokerState::set_runtime_pause` in `broker/src/engine/authority.rs` |
| `ReplaceSettings` | `Broker::replace_settings`, `begin_control_plane_mutation`, `install_settings_authority`, `BrokerState::install_settings` in `broker/src/engine/settings.rs`; `BrokerState::close_all_sessions` in `broker/src/engine/mod.rs` |
| `Report` | `Broker::report_desktop_state`, `finish_desktop_transition` in `broker/src/engine/desktop.rs` |
| `Forget` | `Broker::forget_desktop_state` |
| `EndShareGrace` | `Broker::end_share_grace` |
| `Open` | `Broker::open_session` in `broker/src/engine/session.rs` |
| `Close` | `Broker::close_session` |
| `Focus` | `Broker::update_context` |
| `Admit` | `Broker::request_suggestion`, `admit_generation` in `broker/src/engine/generation.rs` |
| `Snooze` | `Broker::snooze_focused`, `BrokerState::focused_subject` in `broker/src/engine/snooze.rs` |
| `EndSnoozes` | `Broker::end_snoozes` |
| `Expire` | `Broker::expire_snoozes` |
| `Revoked` | `SessionState::revoke_context_authority` in `broker/src/engine/session.rs` |
| `RuntimePaused`, `EffectivePaused` | `BrokerState::runtime_paused`, `BrokerState::effective_paused` in `broker/src/engine/mod.rs` |
| `DesktopHoldOf`, `ScreenShareOf` | `BrokerState::desktop_hold`, `BrokerState::screen_share` in `broker/src/engine/desktop.rs`; `DesktopSession::holds` in `broker/src/protocol.rs` |
| `DataAccess` | `BrokerState::session_with_data_access` in `broker/src/engine/mod.rs`; `BrokerState::withheld_reason` in `broker/src/engine/snooze.rs`; `settings_allow_data` in `broker/src/engine/authority.rs`; `PolicyResolution::allows_context_read`, `allows_display`, `allows_suggestion` in `broker/src/settings/policy.rs` |
| `SuggestionsAllowed` | `policy_status` in `broker/src/engine/authority.rs` |

`withheld_reason` also returns an editor hold. That input is outside this model; see [Bounds](#bounds).

## Bounds

TLC checks every interleaving inside these bounds, and nothing outside them:

- Two sessions, fixed to two sites (`mail`, `docs`). Two fields of one app are not in the model. `snoozing_needs_one_focused_app_or_site_and_no_pause` in `broker/src/engine/tests/snooze.rs` covers two fields of one app.
- Two helper connections. Each report is one of active, shared, inactive, or inactive-and-shared. Sleeping and inactive both `holds` and are one state here; `desktop_status` still tells them apart, and this check does not.
- Screen-share grace is one phase, not the 10 second `SCREEN_SHARE_GRACE`. `a_share_grace_ends_on_its_own_and_never_outlasts_its_bound` in `broker/src/engine/tests/desktop.rs` covers the timer. The `withhold_` grace test uses a 100 ms grace on the real broker and checks the ordering, not the 10 second constant.
- A snooze is on or off per site. `Expire` drops one site and leaves the other. It does not model which deadline is earlier. `an_earlier_snoozes_wake_up_leaves_a_later_snooze_alone` covers that.
- The control plane is healthy and not mid-mutation. `replace_settings` is one step. The real gap between `begin_control_plane_mutation` and `install_settings_authority` is fail-closed: sessions are already closed and `runtime_paused` is true, so it cannot admit a suggestion.
- The target is allowed by settings whenever it is not paused, held or snoozed. A blocked site, an unknown identity, manual activation, a sensitive field and editor focus are other denials.
- No commit grant, no suggestion text, no battery scheduling, no type-through.

Checked states at the confirmed commit: 3080 distinct, search depth 13. TLC's state-graph outdegree can show a minimum of 0. That is an action that leaves the state unchanged, such as a repeated helper report. Deadlock checking is on; a state with no enabled action fails the run.

## What a pass does not cover

Commit authorization, `commits_applied`, sessions returning to 0, and the alternatives cycle. Editor focus withholding the native path. The 10 second grace constant and two snooze deadlines racing the clock. Anything the session helper does before `report_desktop_state`. A pass does not mean grey text was shown on a real shared screen; that needs a live desktop trial.
