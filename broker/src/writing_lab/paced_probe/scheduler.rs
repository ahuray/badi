//! One active request and one replaceable pending snapshot. All timestamps are
//! relative to the fixed typing epoch, never to request dispatch.

use std::future::Future;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{EventInput, EventRecord, ProbeCancellation, milliseconds};
use crate::writing_lab::{LabError, Request, ResultRecord};

pub(super) struct Schedule {
    pub events: Vec<EventRecord>,
    pub error: Option<&'static str>,
}

fn scheduled_ms(value: u64) -> f64 {
    f64::from(u32::try_from(value).expect("validated bounded event time"))
}

pub(super) fn records(events: &[EventInput]) -> Vec<EventRecord> {
    events
        .iter()
        .map(|event| EventRecord {
            dispatchable: !super::transient_joiner(&event.request),
            request_id: event.request.id.clone(),
            at_ms: event.at_ms,
            observed_at_ms: None,
            deadline_ms: event.at_ms + 550,
            started_at_ms: None,
            finished_at_ms: None,
            queue_ms: None,
            scheduling_lateness_ms: None,
            delivered_at_ms: None,
            disposition: "unavailable",
            reason: "not_observed",
            result: None,
        })
        .collect()
}

fn observe_due(
    rows: &mut [EventRecord],
    next: &mut usize,
    pending: &mut Option<usize>,
    elapsed: f64,
) {
    while *next < rows.len() && scheduled_ms(rows[*next].at_ms) <= elapsed {
        let index = *next;
        *next += 1;
        rows[index].observed_at_ms = Some(elapsed);
        rows[index].scheduling_lateness_ms = Some(elapsed - scheduled_ms(rows[index].at_ms));
        if let Some(replaced) = pending.take() {
            rows[replaced].disposition = "coalesced";
            rows[replaced].reason = "newer_pending_snapshot";
        }
        if rows[index].dispatchable {
            *pending = Some(index);
        } else {
            rows[index].disposition = "unavailable";
            rows[index].reason = "transient_joiner_no_request";
        }
    }
}

fn stop(mut rows: Vec<EventRecord>, error: &'static str) -> Schedule {
    for row in &mut rows {
        if row.disposition == "unavailable" && row.reason == "not_observed" {
            row.reason = error;
        }
    }
    Schedule {
        events: rows,
        error: Some(error),
    }
}

async fn next_event(epoch: Instant, next: Option<u64>) {
    if let Some(at) = next {
        tokio::time::sleep_until(epoch + std::time::Duration::from_millis(at)).await;
    } else {
        std::future::pending::<()>().await;
    }
}

fn classify(
    row: &mut EventRecord,
    superseded: bool,
    finished: f64,
    result: ResultRecord,
) -> Option<&'static str> {
    // Complete words do not certify backend idleness. A nonterminal HTTP
    // timeout requires owned shutdown, even if it returned useful prose.
    let failure =
        if finished > scheduled_ms(row.deadline_ms) || result.terminal_received != Some(true) {
            row.disposition = "deadline";
            row.reason = "active_deadline";
            Some("active_deadline")
        } else if superseded {
            row.disposition = "superseded";
            row.reason = "newer_observed_snapshot";
            None
        } else if result.outcome == "suggestion"
            && result.word_complete
            && result.shape_valid
            && result
                .text
                .as_ref()
                .is_some_and(|text| !text.trim().is_empty())
            && result.replace_before.is_none()
        {
            row.disposition = "eligible";
            row.reason = "current_terminal_suggestion";
            row.delivered_at_ms = Some(finished);
            None
        } else if result.outcome == "abstention" {
            row.disposition = "unavailable";
            row.reason = "model_abstention";
            None
        } else {
            row.disposition = "error";
            row.reason = "invalid_prediction_result";
            Some("invalid_prediction_result")
        };
    row.result = Some(result);
    failure
}

pub(super) async fn run<F, Operation>(
    events: &[EventInput],
    epoch: Instant,
    cancellation: &ProbeCancellation,
    mut predict: F,
) -> Schedule
where
    F: FnMut(Request, CancellationToken) -> Operation,
    Operation: Future<Output = Result<ResultRecord, LabError>>,
{
    let mut rows = records(events);
    let mut next = 0;
    let mut pending = None;
    let mut active = None;
    loop {
        let elapsed = milliseconds(epoch);
        observe_due(&mut rows, &mut next, &mut pending, elapsed);
        if cancellation.token.is_cancelled() {
            return stop(rows, cancellation.reason());
        }
        if active.is_none() {
            let now = milliseconds(epoch);
            observe_due(&mut rows, &mut next, &mut pending, now);
            let Some(index) = pending.take() else {
                if next == rows.len() {
                    return Schedule {
                        events: rows,
                        error: None,
                    };
                }
                tokio::select! {
                    biased;
                    () = cancellation.token.cancelled() => {},
                    () = next_event(epoch, Some(rows[next].at_ms)) => {},
                }
                continue;
            };
            if now >= scheduled_ms(rows[index].deadline_ms) {
                rows[index].disposition = "deadline";
                rows[index].reason = "queue_deadline";
                continue;
            }
            rows[index].started_at_ms = Some(now);
            rows[index].queue_ms = Some(now - rows[index].observed_at_ms.unwrap_or(now));
            let token = CancellationToken::new();
            active = Some((
                index,
                token.clone(),
                Box::pin(predict(events[index].request.clone(), token)),
            ));
        }
        let (index, token, operation) = active.as_mut().expect("active request");
        let index = *index;
        let deadline = epoch + std::time::Duration::from_millis(rows[index].deadline_ms);
        let result = tokio::select! {
            biased;
            () = cancellation.token.cancelled() => {
                token.cancel();
                return stop(rows, cancellation.reason());
            },
            () = next_event(epoch, rows.get(next).map(|row| row.at_ms)) => continue,
            () = tokio::time::sleep_until(deadline) => None,
            result = operation => Some(result),
        };
        // A ready inner future can win a timeout after its deadline. It can
        // also run across a typing event. Re-read the clock and revisions before
        // considering either runtime reuse or delivery.
        let finished = milliseconds(epoch);
        observe_due(&mut rows, &mut next, &mut pending, finished);
        if cancellation.token.is_cancelled() {
            token.cancel();
            return stop(rows, cancellation.reason());
        }
        rows[index].finished_at_ms = Some(finished);
        let Some(result) = result else {
            token.cancel();
            rows[index].disposition = "deadline";
            rows[index].reason = "active_deadline";
            return stop(rows, "active_deadline");
        };
        let Ok(result) = result else {
            rows[index].disposition = "error";
            rows[index].reason = "runtime_request_failed";
            return stop(rows, "runtime_request_failed");
        };
        if let Some(reason) = classify(&mut rows[index], index + 1 < next, finished, result) {
            token.cancel();
            return stop(rows, reason);
        }
        active = None;
    }
}

#[cfg(test)]
mod tests {
    use super::super::Control;
    use super::super::tests::{input, result};
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    #[tokio::test(start_paused = true)]
    async fn due_event_precedes_simultaneous_completion_and_no_future_input_is_sent() {
        let input = input();
        let epoch = Instant::now();
        let mut launched = Vec::new();
        let report = run(
            &input.events,
            epoch,
            &ProbeCancellation::default(),
            |request, _| {
                let index = input
                    .events
                    .iter()
                    .position(|event| event.request.id == request.id)
                    .unwrap();
                let now = milliseconds(epoch);
                assert!(now >= scheduled_ms(input.events[index].at_ms));
                launched.push((index, now));
                async move {
                    tokio::time::sleep(Duration::from_millis(if index == 0 { 100 } else { 50 }))
                        .await;
                    Ok(result(&request))
                }
            },
        )
        .await;
        assert_eq!(report.error, None);
        assert_eq!(report.events[0].disposition, "superseded");
        assert_eq!(report.events[0].delivered_at_ms, None);
        assert_eq!(launched, [(0, 0.0), (1, 100.0), (2, 250.0), (3, 500.0)]);
        assert!(
            report
                .events
                .iter()
                .skip(1)
                .all(|row| row.disposition == "eligible")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn latest_pending_replaces_older_snapshots_and_keeps_original_deadline() {
        let input = input();
        let epoch = Instant::now();
        let mut launched = 0;
        let report = run(
            &input.events,
            epoch,
            &ProbeCancellation::default(),
            |request, _| {
                launched += 1;
                let delay = if launched == 1 { 540 } else { 520 };
                async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    Ok(result(&request))
                }
            },
        )
        .await;
        assert_eq!(launched, 2);
        assert_eq!(report.error, Some("active_deadline"));
        assert_eq!(report.events[1].disposition, "coalesced");
        assert_eq!(report.events[2].disposition, "coalesced");
        assert_eq!(report.events[3].queue_ms, Some(40.0));
        assert_eq!(report.events[3].deadline_ms, 1050);
        assert_eq!(report.events[3].finished_at_ms, Some(1050.0));
        assert_eq!(report.events[3].disposition, "deadline");
    }

    #[tokio::test(start_paused = true)]
    async fn scheduling_lateness_is_preserved_and_overdue_snapshots_coalesce_before_dispatch() {
        let input = input();
        let epoch = Instant::now();
        tokio::time::advance(Duration::from_millis(300)).await;
        let mut ids = Vec::new();
        let report = run(
            &input.events,
            epoch,
            &ProbeCancellation::default(),
            |request, _| {
                ids.push(request.id.clone());
                async move { Ok(result(&request)) }
            },
        )
        .await;
        assert_eq!(
            ids,
            [
                input.events[2].request.id.clone(),
                input.events[3].request.id.clone()
            ]
        );
        assert_eq!(report.events[2].scheduling_lateness_ms, Some(50.0));
        assert_eq!(report.events[2].queue_ms, Some(0.0));
        assert_eq!(report.events[0].disposition, "coalesced");
        assert_eq!(report.events[1].disposition, "coalesced");
    }

    #[tokio::test(start_paused = true)]
    async fn nonterminal_complete_words_require_shutdown_without_runtime_reuse() {
        let input = input();
        let mut count = 0;
        let report = run(
            &input.events,
            Instant::now(),
            &ProbeCancellation::default(),
            |request, _| {
                count += 1;
                async move {
                    let mut result = result(&request);
                    result.terminal_received = Some(false);
                    Ok(result)
                }
            },
        )
        .await;
        assert_eq!(count, 1);
        assert_eq!(report.error, Some("active_deadline"));
        assert_eq!(report.events[0].disposition, "deadline");
        assert!(report.events[0].result.as_ref().unwrap().word_complete);
        assert!(
            report
                .events
                .iter()
                .all(|row| row.delivered_at_ms.is_none())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn misleading_suggestions_never_become_eligible() {
        for defect in 0..5 {
            let input = input();
            let report = run(
                &input.events,
                Instant::now(),
                &ProbeCancellation::default(),
                |request, _| async move {
                    let mut result = result(&request);
                    match defect {
                        0 => result.word_complete = false,
                        1 => result.shape_valid = false,
                        2 => result.text = None,
                        3 => result.replace_before = Some("draft".to_owned()),
                        _ => result.outcome = "error",
                    }
                    Ok(result)
                },
            )
            .await;
            assert_eq!(report.error, Some("invalid_prediction_result"));
            assert!(
                report
                    .events
                    .iter()
                    .all(|row| row.delivered_at_ms.is_none())
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn preflight_and_inference_share_original_budget_and_cancel_active_operation() {
        let input = input();
        let active_token = Rc::new(RefCell::new(None));
        let record = Rc::clone(&active_token);
        let mut count = 0;
        let report = run(
            &input.events,
            Instant::now(),
            &ProbeCancellation::default(),
            |request, token| {
                count += 1;
                *record.borrow_mut() = Some(token);
                async move {
                    tokio::time::sleep(Duration::from_millis(400)).await; // preflight
                    tokio::time::sleep(Duration::from_millis(200)).await; // generation
                    Ok(result(&request))
                }
            },
        )
        .await;
        assert_eq!(count, 1);
        assert_eq!(report.events[0].finished_at_ms, Some(550.0));
        assert!(active_token.borrow().as_ref().unwrap().is_cancelled());
        assert_eq!(report.error, Some("active_deadline"));
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_wins_simultaneous_completion_and_prevents_next_launch() {
        for control in [Control::Cancel, Control::Clear, Control::ContextChange] {
            let input = input();
            let cancellation = ProbeCancellation::default();
            let trigger = cancellation.clone();
            let mut count = 0;
            let report = run(
                &input.events,
                Instant::now(),
                &cancellation,
                |request, _| {
                    count += 1;
                    let trigger = trigger.clone();
                    async move {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        trigger.cancel(control);
                        Ok(result(&request))
                    }
                },
            )
            .await;
            assert_eq!(count, 1);
            assert_eq!(report.error, Some(cancellation.reason()));
            assert_eq!(report.events.len(), 4);
            assert!(
                report
                    .events
                    .iter()
                    .all(|row| row.delivered_at_ms.is_none())
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn successful_future_returned_after_clock_advance_is_never_delivered() {
        let input = input();
        let mut count = 0;
        let report = run(
            &input.events,
            Instant::now(),
            &ProbeCancellation::default(),
            |request, _| {
                count += 1;
                async move {
                    tokio::time::advance(Duration::from_millis(551)).await;
                    Ok(result(&request))
                }
            },
        )
        .await;
        assert_eq!(count, 1);
        assert_eq!(report.error, Some("active_deadline"));
        assert!(
            report
                .events
                .iter()
                .all(|row| row.delivered_at_ms.is_none())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn transient_joiner_supersedes_active_and_pending_without_inference_then_recovers() {
        let mut input = input();
        for (event, before) in input
            .events
            .iter_mut()
            .zip(["میر", "میری", "میری‌", "میری‌ر"])
        {
            event.request.before = before.to_owned();
            event.request.language = "fa".to_owned();
        }
        input.validate().unwrap();
        let mut sent = Vec::new();
        let report = run(
            &input.events,
            Instant::now(),
            &ProbeCancellation::default(),
            |request, _| {
                sent.push(request.before.clone());
                async move {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    Ok(result(&request))
                }
            },
        )
        .await;
        assert_eq!(report.error, None);
        assert_eq!(sent, ["میر", "میری‌ر"]);
        assert_eq!(report.events[0].disposition, "superseded");
        assert_eq!(report.events[1].disposition, "coalesced");
        let transient = &report.events[2];
        assert_eq!(transient.disposition, "unavailable");
        assert_eq!(transient.reason, "transient_joiner_no_request");
        assert_eq!(transient.observed_at_ms, Some(250.0));
        assert!(
            transient.started_at_ms.is_none()
                && transient.result.is_none()
                && transient.delivered_at_ms.is_none()
        );
        assert_eq!(report.events[3].disposition, "eligible");
    }
}
