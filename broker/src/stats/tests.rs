use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use jiff::civil::{Date, date};
use tempfile::{TempDir, tempdir};

use super::{
    MAX_STATS_BYTES, MAX_SUBJECTS_PER_DAY, MAX_SUMMARY_SUBJECTS, STATS_FILE_NAME,
    STATS_RETENTION_DAYS, StatsCounts, StatsError, StatsEvent, StatsStore, StatsSummary,
};
use crate::protocol::MAX_SAFE_COUNTER;
use crate::settings::{
    BrowserAdapter, LinuxAdapter, PermissionDecision, RetentionPermission, SettingsV2,
    StableIdentity, SubjectPermissions, SubjectRule, WebScheme, ensure_private_directory,
    read_private_limited,
};

const TODAY: Date = date(2026, 10, 20);

fn app(id: &str) -> StableIdentity {
    StableIdentity::LinuxApp {
        adapter: LinuxAdapter::Fcitx,
        app_id: id.to_owned(),
    }
}

fn site(host: &str) -> StableIdentity {
    StableIdentity::browser_origin(BrowserAdapter::Chromium, WebScheme::Https, host, None)
        .expect("identity")
}

fn rule(identity: StableIdentity, decision: PermissionDecision) -> SubjectRule {
    SubjectRule {
        identity,
        permissions: SubjectPermissions {
            suggest: decision,
            display: decision,
            context_read: decision,
            learn: PermissionDecision::Block,
            retention: RetentionPermission::None,
        },
    }
}

/// Settings that let every listed subject suggest.
fn allowing(identities: &[StableIdentity]) -> SettingsV2 {
    let mut subjects: Vec<SubjectRule> = identities
        .iter()
        .map(|identity| rule(identity.clone(), PermissionDecision::Allow))
        .collect();
    subjects.sort_by(|left, right| left.identity.cmp(&right.identity));
    SettingsV2 {
        paused: false,
        subjects,
        ..SettingsV2::deny_by_default()
    }
}

fn store() -> (TempDir, PathBuf, StatsStore) {
    let directory = tempdir().expect("temporary directory");
    // The private data directory: atomic writes create it with mode 0700.
    let path = directory.path().join("badi").join(STATS_FILE_NAME);
    let store = StatsStore::open(path.clone()).expect("empty store");
    (directory, path, store)
}

fn days_ago(days: i64) -> Date {
    TODAY
        .checked_sub(jiff::Span::new().days(days))
        .expect("date in range")
}

fn counts(shown: u64, accepted: u64, words: u64) -> StatsCounts {
    StatsCounts {
        shown,
        accepted,
        words,
    }
}

#[test]
fn counts_per_day_and_subject_survive_a_reopen() {
    let (_directory, path, mut store) = store();
    let zen = site("example.com");
    let ghostty = app("com.mitchellh.ghostty");
    let settings = allowing(&[zen.clone(), ghostty.clone()]);
    for _ in 0..3 {
        assert!(store.record(&settings, TODAY, &zen, StatsEvent::Shown));
    }
    assert!(store.record(&settings, TODAY, &zen, StatsEvent::Accepted { words: 2 }));
    assert!(store.record(&settings, TODAY, &zen, StatsEvent::Accepted { words: 0 }));
    assert!(store.record(&settings, days_ago(1), &ghostty, StatsEvent::Shown));
    assert!(store.record(
        &settings,
        days_ago(1),
        &ghostty,
        StatsEvent::Accepted { words: 4 }
    ));
    let before = store.summary(TODAY);
    assert!(store.persist().expect("persist"));
    assert!(
        !store.persist().expect("nothing new"),
        "clean store writes nothing"
    );

    let reopened = StatsStore::open(path.clone()).expect("reopen");
    let after = reopened.summary(TODAY);
    assert_eq!(after, before);
    assert_eq!(after.today, "2026-10-20");
    assert_eq!(after.retention_days, 90);
    assert_eq!(after.totals.today, counts(3, 2, 2));
    assert_eq!(after.totals.last_7_days, counts(4, 3, 6));
    assert_eq!(after.totals.retained, counts(4, 3, 6));
    let days: Vec<(&str, u64)> = after
        .days
        .iter()
        .map(|day| (day.day.as_str(), day.words))
        .collect();
    assert_eq!(days, [("2026-10-19", 4), ("2026-10-20", 2)]);
    assert_eq!(after.subjects.len(), 2);
    assert_eq!(after.subjects[0].identity, ghostty, "most words first");
    assert_eq!(after.subjects[0].today, StatsCounts::default());
    assert_eq!(after.subjects[0].last_7_days, counts(1, 1, 4));
    assert_eq!(after.subjects[1].identity, zen);
    assert_eq!(after.subjects[1].today, counts(3, 2, 2));
}

#[test]
fn the_file_is_private_compact_and_holds_counts_only() {
    let (_directory, path, mut store) = store();
    let zen = site("example.com");
    let settings = allowing(std::slice::from_ref(&zen));
    store.record(&settings, TODAY, &zen, StatsEvent::Accepted { words: 3 });
    store.persist().expect("persist");
    let mode = fs::metadata(&path).expect("file").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let bytes = read_private_limited(&path, MAX_STATS_BYTES)
        .expect("read")
        .expect("present");
    let text = String::from_utf8(bytes).expect("UTF-8");
    assert_eq!(
        text,
        concat!(
            r#"{"schema":"badi.stats.v1","days":[{"day":"2026-10-20","subjects":["#,
            r#"{"identity":{"kind":"browser_origin","adapter":"chromium","scheme":"https","host":"example.com","port":443},"#,
            r#""shown":0,"accepted":1,"words":3}]}]}"#,
            "\n"
        )
    );
}

#[test]
fn pause_blocked_and_unlisted_targets_record_nothing() {
    let (_directory, path, mut store) = store();
    let allowed = app("omawrite");
    let blocked = app("discord");
    let unlisted = site("unlisted.example");
    let mut settings = allowing(std::slice::from_ref(&allowed));
    settings
        .subjects
        .push(rule(blocked.clone(), PermissionDecision::Block));
    settings
        .subjects
        .sort_by(|left, right| left.identity.cmp(&right.identity));
    settings.validate().expect("valid settings");

    assert!(!store.record(&settings, TODAY, &blocked, StatsEvent::Shown));
    assert!(!store.record(&settings, TODAY, &unlisted, StatsEvent::Shown));
    let paused = SettingsV2 {
        paused: true,
        ..settings.clone()
    };
    assert!(!store.record(&paused, TODAY, &allowed, StatsEvent::Accepted { words: 1 }));
    assert_eq!(store.summary(TODAY).totals.retained, StatsCounts::default());
    assert!(!store.persist().expect("nothing to write"));
    assert!(!path.exists(), "nothing counted, nothing written");

    // A list mode lets an unlisted site suggest, so it counts there; an
    // explicit block still wins.
    let blocklist = SettingsV2 {
        all_web_origins: true,
        all_linux_apps: true,
        ..settings
    };
    assert!(store.record(&blocklist, TODAY, &unlisted, StatsEvent::Shown));
    assert!(!store.record(&blocklist, TODAY, &blocked, StatsEvent::Shown));
    assert_eq!(store.summary(TODAY).totals.today, counts(1, 0, 0));
}

#[test]
fn days_older_than_the_retention_window_are_deleted() {
    let (_directory, path, mut store) = store();
    let zen = site("example.com");
    let settings = allowing(std::slice::from_ref(&zen));
    let oldest_kept = days_ago(i64::from(STATS_RETENTION_DAYS) - 1);
    // Each day counts on that day, as the clock moves on.
    for day in [days_ago(200), oldest_kept, TODAY] {
        store.record(&settings, day, &zen, StatsEvent::Shown);
    }
    let summary = store.summary(TODAY);
    let kept: Vec<&str> = summary.days.iter().map(|day| day.day.as_str()).collect();
    assert_eq!(kept, ["2026-07-23", "2026-10-20"]);
    assert_eq!(summary.totals.retained.shown, 2);

    store.persist().expect("persist");
    let mut store = StatsStore::open(path.clone()).expect("reopen");
    let a_week_later = TODAY.checked_add(jiff::Span::new().days(7)).expect("date");
    assert!(store.reconcile(a_week_later));
    store.persist().expect("persist");
    let summary = StatsStore::open(path.clone())
        .expect("reopen")
        .summary(a_week_later);
    let kept: Vec<&str> = summary.days.iter().map(|day| day.day.as_str()).collect();
    assert_eq!(kept, ["2026-10-20"]);

    let far_later = TODAY
        .checked_add(jiff::Span::new().days(400))
        .expect("date");
    assert!(store.reconcile(far_later));
    assert!(store.persist().expect("persist"));
    assert!(!path.exists(), "an empty store removes its file");
}

#[test]
fn moving_west_across_a_time_zone_keeps_the_day_just_counted() {
    let (_directory, _path, mut store) = store();
    let zen = site("example.com");
    let settings = allowing(std::slice::from_ref(&zen));
    let tomorrow = TODAY.tomorrow().expect("tomorrow");
    store.record(&settings, tomorrow, &zen, StatsEvent::Shown);
    // The local date steps back a day: tomorrow's count stays.
    store.record(&settings, TODAY, &zen, StatsEvent::Shown);
    let summary = store.summary(TODAY);
    assert_eq!(summary.days.len(), 2);
    assert_eq!(summary.totals.today.shown, 1, "tomorrow is not today");
    assert_eq!(summary.totals.retained.shown, 2);
    let day_after = tomorrow.tomorrow().expect("date");
    assert!(!store.reconcile(TODAY));
    store.record(&settings, day_after, &zen, StatsEvent::Shown);
    assert!(
        store.reconcile(days_ago(1)),
        "two days ahead is a clock error"
    );
}

#[test]
fn a_full_day_counts_further_subjects_in_its_total() {
    let (_directory, path, mut store) = store();
    let subjects: Vec<StableIdentity> = (0..=MAX_SUBJECTS_PER_DAY)
        .map(|index| site(&format!("site{index:03}.example")))
        .collect();
    let settings = SettingsV2 {
        paused: false,
        all_web_origins: true,
        ..SettingsV2::deny_by_default()
    };
    for identity in &subjects {
        assert!(store.record(
            &settings,
            TODAY,
            identity,
            StatsEvent::Accepted { words: 1 }
        ));
    }
    let summary = store.summary(TODAY);
    let expected = u64::try_from(subjects.len()).expect("small");
    assert_eq!(summary.totals.today, counts(0, expected, expected));
    assert_eq!(summary.days[0].words, expected);
    store.persist().expect("persist");
    let reopened = StatsStore::open(path).expect("reopen");
    assert_eq!(reopened.summary(TODAY).totals, summary.totals);
}

#[test]
fn counters_stay_json_safe() {
    let (_directory, _path, mut store) = store();
    let zen = site("example.com");
    let settings = allowing(std::slice::from_ref(&zen));
    store.record(
        &settings,
        TODAY,
        &zen,
        StatsEvent::Accepted {
            words: MAX_SAFE_COUNTER,
        },
    );
    store.record(&settings, TODAY, &zen, StatsEvent::Accepted { words: 9 });
    assert_eq!(store.summary(TODAY).totals.today.words, MAX_SAFE_COUNTER);
}

#[test]
fn a_summary_names_the_leaders_of_each_window() {
    let (_directory, _path, mut store) = store();
    let settings = SettingsV2 {
        paused: false,
        all_web_origins: true,
        ..SettingsV2::deny_by_default()
    };
    // Forty sites wrote many words long ago; one site wrote a single word
    // today. Today's leader is named although it ranks last over 90 days.
    for index in 0..40 {
        let identity = site(&format!("old{index:02}.example"));
        store.record(
            &settings,
            days_ago(30),
            &identity,
            StatsEvent::Accepted { words: 100 },
        );
    }
    let fresh = site("fresh.example");
    store.record(&settings, TODAY, &fresh, StatsEvent::Accepted { words: 1 });
    let summary = store.summary(TODAY);
    assert_eq!(summary.subjects.len(), MAX_SUMMARY_SUBJECTS + 1);
    assert_eq!(summary.subjects.last().expect("fresh").identity, fresh);
    assert_eq!(summary.totals.retained.words, 4_001, "totals stay exact");
}

#[test]
fn the_largest_summary_fits_one_frame() {
    let (_directory, _path, mut store) = store();
    let settings = SettingsV2 {
        paused: false,
        all_web_origins: true,
        ..SettingsV2::deny_by_default()
    };
    let long_label = "a".repeat(63);
    let host = format!("{long_label}.{long_label}.{long_label}.{}", "b".repeat(61));
    assert_eq!(host.len(), 253);
    for day in 0..i64::from(STATS_RETENTION_DAYS) {
        for index in 0..MAX_SUMMARY_SUBJECTS {
            let identity = StableIdentity::browser_origin(
                BrowserAdapter::Chromium,
                WebScheme::Https,
                &format!("{index:02}{}", &host[2..]),
                Some(65_535),
            )
            .expect("identity");
            store.record(
                &settings,
                days_ago(day),
                &identity,
                StatsEvent::Accepted {
                    words: MAX_SAFE_COUNTER,
                },
            );
        }
    }
    let summary = store.summary(TODAY);
    let payload = crate::protocol::StatsStatusPayload {
        available: true,
        summary,
    };
    let frame = crate::protocol::WireEnvelope::global(
        crate::protocol::MessageType::StatsStatus,
        0,
        &payload,
    )
    .expect("envelope")
    .at_version(crate::protocol::CURRENT_PROTOCOL_VERSION)
    .expect("v2");
    let bytes = serde_json::to_vec(&frame).expect("frame JSON");
    assert!(
        bytes.len() < crate::protocol::MAX_FRAME_BYTES,
        "{} bytes",
        bytes.len()
    );
}

#[test]
fn opening_removes_a_copy_an_interrupted_write_left() {
    let (_directory, path, mut store) = store();
    let zen = site("example.com");
    let settings = allowing(std::slice::from_ref(&zen));
    store.record(&settings, TODAY, &zen, StatsEvent::Shown);
    store.persist().expect("persist");
    let leftover = path.with_file_name(format!(".{STATS_FILE_NAME}.{}.tmp", "0a".repeat(16)));
    fs::copy(&path, &leftover).expect("leftover copy");
    fs::set_permissions(&leftover, fs::Permissions::from_mode(0o600)).expect("mode");
    let reopened = StatsStore::open(path).expect("reopen");
    assert!(!leftover.exists());
    assert_eq!(reopened.summary(TODAY).totals.today.shown, 1);
}

#[test]
fn forget_deletes_every_count_and_the_file() {
    let (_directory, path, mut store) = store();
    let zen = site("example.com");
    let settings = allowing(std::slice::from_ref(&zen));
    store.record(&settings, TODAY, &zen, StatsEvent::Shown);
    store.persist().expect("persist");
    assert!(path.exists());
    store.forget().expect("forget");
    assert!(!path.exists());
    assert_eq!(store.summary(TODAY).days, []);
    assert!(!store.persist().expect("nothing pending"));
}

#[test]
fn corrupt_or_noncanonical_files_are_refused() {
    let identity = r#"{"kind":"linux_app","adapter":"fcitx","app_id":"omawrite"}"#;
    let other = r#"{"kind":"linux_app","adapter":"fcitx","app_id":"zed"}"#;
    let subject = |identity: &str, words: u64| {
        format!(r#"{{"identity":{identity},"shown":1,"accepted":1,"words":{words}}}"#)
    };
    let document = |days: &str| format!(r#"{{"schema":"badi.stats.v1","days":[{days}]}}"#);
    let day = |day: &str, subjects: &str| format!(r#"{{"day":"{day}","subjects":[{subjects}]}}"#);
    let valid = document(&day("2026-10-20", &subject(identity, 1)));
    let cases = [
        // Text never belongs in the file.
        valid.replace(r#""words":1"#, r#""words":1,"text":"hello""#),
        valid.replace("badi.stats.v1", "badi.stats.v0"),
        document(&day("2026-10-2", &subject(identity, 1))),
        document(&day("2026-13-01", &subject(identity, 1))),
        // Days and subjects in canonical order, without duplicates.
        document(&format!(
            "{},{}",
            day("2026-10-20", &subject(identity, 1)),
            day("2026-10-19", &subject(identity, 1))
        )),
        document(&day(
            "2026-10-20",
            &format!("{},{}", subject(other, 1), subject(identity, 1)),
        )),
        document(&day(
            "2026-10-20",
            &format!("{},{}", subject(identity, 1), subject(identity, 1)),
        )),
        document(&day(
            "2026-10-20",
            r#"{"identity":{"kind":"linux_app","adapter":"fcitx","app_id":"omawrite"},"shown":0,"accepted":0,"words":0}"#,
        )),
        document(&day("2026-10-20", "")),
        document(&day("2026-10-20", &subject(identity, MAX_SAFE_COUNTER + 1))),
        document(&day(
            "2026-10-20",
            &subject(
                r#"{"kind":"linux_app","adapter":"fcitx","app_id":"Bad App"}"#,
                1,
            ),
        )),
    ];
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("badi").join(STATS_FILE_NAME);
    ensure_private_directory(path.parent().expect("parent")).expect("private directory");
    fs::write(&path, &valid).expect("write");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("mode");
    StatsStore::open(path.clone()).expect("the valid document opens");
    for case in cases {
        fs::write(&path, &case).expect("write");
        assert!(
            matches!(StatsStore::open(path.clone()), Err(StatsError::Corrupt)),
            "{case}"
        );
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("mode");
    fs::write(&path, &valid).expect("write");
    assert!(matches!(
        StatsStore::open(path),
        Err(StatsError::Storage(_))
    ));
}

#[test]
fn a_summary_round_trips_and_rejects_unknown_fields() {
    let (_directory, _path, mut store) = store();
    let zen = site("example.com");
    let settings = allowing(std::slice::from_ref(&zen));
    store.record(&settings, TODAY, &zen, StatsEvent::Shown);
    let summary = store.summary(TODAY);
    let mut value = serde_json::to_value(&summary).expect("JSON");
    let decoded: StatsSummary = serde_json::from_value(value.clone()).expect("decode");
    assert_eq!(decoded, summary);
    value["subjects"][0]["text"] = serde_json::json!("hello");
    assert!(serde_json::from_value::<StatsSummary>(value).is_err());
}
