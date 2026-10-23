use std::path::PathBuf;

use super::{
    CLI_CAPABILITIES, CliError, Command, LocalCommand, ParsedCommand, StatsCommand, build_overview,
    global, parse_arguments, read_probe_text, redact_health_status, validate_control_response,
    validate_correlated_response, validate_handshake, validate_health_response,
    validate_probe_response, validate_settings_response,
};
use badi_broker::metrics::MetricsSnapshot;
use badi_broker::model_selection::ModelUseCase;
use badi_broker::protocol::{
    ActiveLocator, AutomaticScheduling, CURRENT_PROTOCOL_VERSION, ControlAction,
    ControlResultPayload, DesktopSession, DesktopStatusPayload, GlobalControlRequestPayload,
    HealthStatusPayload, HelloAckPayload, MAX_AFTER_CHARS, MAX_BEFORE_CHARS, MAX_FRAME_BYTES,
    MAX_SUGGESTION_CHARS, MAX_SUGGESTION_WORDS, MessageType, ProbeRequestPayload,
    ProbeResultPayload, ProviderKind, ReasonCode, ScreenShare, SessionId, SettingsStatusPayload,
    SnoozeStatus, WireEnvelope,
};
use badi_broker::settings::{LinuxAdapter, SettingsV2, StableIdentity};
use jsonschema::Registry;
use serde_json::json;

fn arguments(values: &[&str]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn overview_schema_validator() -> jsonschema::Validator {
    let schema_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas");
    let read_schema = |name: &str| {
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(schema_root.join(name)).expect("read schema"),
        )
        .expect("parse schema")
    };
    let hardware = read_schema("badi.hardware.v1.schema.json");
    let advice = read_schema("badi.model-advice.v3.schema.json");
    let settings = read_schema("badi.settings.v2.schema.json");
    let schema = read_schema("badi.overview.v2.schema.json");
    let registry = Registry::new()
        .add("urn:badi:schema:hardware:v1", hardware)
        .expect("hardware resource")
        .add("urn:badi:schema:model-advice:v3", advice)
        .expect("advice resource")
        .add("urn:badi:schema:settings:v2", settings)
        .expect("settings resource")
        .prepare()
        .expect("overview schema registry");
    jsonschema::options()
        .with_registry(&registry)
        .build(&schema)
        .expect("overview schema")
}

#[test]
fn overview_matches_its_versioned_schema() {
    let health = HealthStatusPayload {
        provider: ProviderKind::PhraseV1,
        paused: true,
        authority_epoch: 0,
        settings_revision: 0,
        control_plane_degraded: false,
        sessions: 0,
        socket_mode: "0600".to_owned(),
        max_frame_bytes: MAX_FRAME_BYTES,
        metrics: MetricsSnapshot::default(),
        active: None,
        desktop: None,
        snoozes: Vec::new(),
    };
    let settings = SettingsV2::deny_by_default();
    let status = SettingsStatusPayload {
        document: serde_json::to_value(settings).expect("settings json"),
        personalization_revision: 0,
        personalization_records: 0,
        personalization_bytes: 0,
        personalization_store_available: true,
        personalization_recorder_available: true,
        personalization_write_failures: 0,
        personalization_dropped_signals: 0,
    };
    let local_overview = serde_json::to_value(
        build_overview(
            HealthStatusPayload {
                provider: ProviderKind::LocalModel,
                // The panel names a screen-share hold from this report.
                desktop: Some(DesktopStatusPayload {
                    observed: true,
                    session: DesktopSession::Active,
                    low_power: false,
                    automatic: AutomaticScheduling::Normal,
                    screen_share: ScreenShare::Held,
                }),
                ..health.clone()
            },
            status.clone(),
        )
        .expect("local overview"),
    )
    .expect("json");
    assert_eq!(
        local_overview.pointer("/broker/desktop/screen_share"),
        Some(&json!("held"))
    );
    assert_eq!(
        local_overview.pointer("/models/writing/installed"),
        Some(&json!(true))
    );
    assert_eq!(
        local_overview.pointer("/models/writing/configured"),
        Some(&json!(true))
    );
    let overview = serde_json::to_value(build_overview(health, status).expect("overview"))
        .expect("overview json");
    let validator = overview_schema_validator();
    if let Err(error) = validator.validate(&local_overview) {
        panic!("local overview failed schema: {error}");
    }
    let mut shared = local_overview.clone();
    shared["broker"]["desktop"]["screen_share"] = json!("shown");
    assert!(!validator.is_valid(&shared));
    if let Err(error) = validator.validate(&overview) {
        panic!("overview failed schema: {error}");
    }
    let mut widened = overview.clone();
    widened["privacy"]["network"] = json!(true);
    assert!(!validator.is_valid(&widened));
    let mut snoozed = overview.clone();
    snoozed["broker"]["snoozes"] = json!([{
        "subject": {"kind": "linux_app", "adapter": "fcitx", "app_id": "telegram"},
        "remaining_ms": 600_000,
    }]);
    assert!(validator.is_valid(&snoozed));
    snoozed["broker"]["snoozes"][0]["subject"]["app_id"] = json!("Inbox - Mail");
    assert!(
        !validator.is_valid(&snoozed),
        "a snooze names an exact app id"
    );
    let mut extended = overview;
    extended["support"] = json!({});
    assert!(!validator.is_valid(&extended));
}

#[test]
fn overview_requires_one_settings_revision_but_allows_runtime_pause() {
    let settings = SettingsV2 {
        paused: false,
        ..SettingsV2::deny_by_default()
    };
    let status = SettingsStatusPayload {
        document: serde_json::to_value(&settings).expect("settings json"),
        personalization_revision: 0,
        personalization_records: 0,
        personalization_bytes: 0,
        personalization_store_available: true,
        personalization_recorder_available: true,
        personalization_write_failures: 0,
        personalization_dropped_signals: 0,
    };
    let health = HealthStatusPayload {
        provider: ProviderKind::PhraseV1,
        paused: true,
        authority_epoch: 3,
        settings_revision: settings.revision,
        control_plane_degraded: false,
        sessions: 0,
        socket_mode: "0600".to_owned(),
        max_frame_bytes: MAX_FRAME_BYTES,
        metrics: MetricsSnapshot::default(),
        active: None,
        desktop: None,
        snoozes: Vec::new(),
    };
    assert!(build_overview(health.clone(), status.clone()).is_ok());

    let mismatched = HealthStatusPayload {
        settings_revision: settings.revision + 1,
        ..health
    };
    assert!(matches!(
        build_overview(mismatched, status),
        Err(CliError::InconsistentOverview)
    ));
}

fn hello_acknowledgment(request_id: &str) -> WireEnvelope {
    let mut acknowledgment = WireEnvelope::global(
        MessageType::HelloAck,
        0,
        &HelloAckPayload {
            selected_v: CURRENT_PROTOCOL_VERSION,
            connection_id: "c:test-connection".to_owned(),
            enabled_capabilities: CLI_CAPABILITIES.to_vec(),
            max_frame_bytes: MAX_FRAME_BYTES,
            max_before_chars: MAX_BEFORE_CHARS,
            max_after_chars: MAX_AFTER_CHARS,
            max_suggestion_chars: MAX_SUGGESTION_CHARS,
            max_suggestion_words: MAX_SUGGESTION_WORDS,
            paused: true,
        },
    )
    .expect("hello acknowledgment")
    .at_version(CURRENT_PROTOCOL_VERSION)
    .expect("v2 hello acknowledgment");
    acknowledgment.id = Some(request_id.to_owned());
    acknowledgment
}

#[test]
fn strict_handshake_accepts_correlated_complete_acknowledgment() {
    let payload = validate_handshake(&hello_acknowledgment("ctl:request"), "ctl:request")
        .expect("strict acknowledgment");
    assert!(payload.paused);
    assert_eq!(payload.enabled_capabilities, CLI_CAPABILITIES.to_vec());
}

#[test]
fn strict_handshake_rejects_wrong_id_limits_capabilities_and_paused_shape() {
    let acknowledgment = hello_acknowledgment("ctl:other");
    assert!(matches!(
        validate_handshake(&acknowledgment, "ctl:request"),
        Err(CliError::Handshake)
    ));

    let mut wrong_limit = hello_acknowledgment("ctl:request");
    wrong_limit.payload["max_frame_bytes"] = json!(MAX_FRAME_BYTES - 1);
    assert!(matches!(
        validate_handshake(&wrong_limit, "ctl:request"),
        Err(CliError::Handshake)
    ));

    let mut wrong_version = hello_acknowledgment("ctl:request");
    wrong_version.payload["selected_v"] = json!(CURRENT_PROTOCOL_VERSION - 1);
    assert!(matches!(
        validate_handshake(&wrong_version, "ctl:request"),
        Err(CliError::Handshake)
    ));

    let mut missing_capability = hello_acknowledgment("ctl:request");
    missing_capability.payload["enabled_capabilities"] = json!(["control"]);
    assert!(matches!(
        validate_handshake(&missing_capability, "ctl:request"),
        Err(CliError::Handshake)
    ));

    let mut invalid_paused = hello_acknowledgment("ctl:request");
    invalid_paused.payload["paused"] = json!("false");
    assert!(matches!(
        validate_handshake(&invalid_paused, "ctl:request"),
        Err(CliError::Handshake)
    ));
}

#[test]
fn v2_settings_response_rejects_a_legacy_document() {
    let mut response = WireEnvelope::global(
        MessageType::SettingsStatus,
        1,
        &SettingsStatusPayload {
            document: json!({
                "schema": "badi.settings.v1",
                "revision": 0,
                "paused": true,
                "subjects": []
            }),
            personalization_revision: 0,
            personalization_records: 0,
            personalization_bytes: 0,
            personalization_store_available: true,
            personalization_recorder_available: true,
            personalization_write_failures: 0,
            personalization_dropped_signals: 0,
        },
    )
    .expect("settings response")
    .at_version(CURRENT_PROTOCOL_VERSION)
    .expect("v2 settings response");
    response.id = Some("ctl:settings".to_owned());

    assert!(matches!(
        validate_settings_response(&response, "ctl:settings"),
        Err(CliError::UnexpectedResponse)
    ));
}

fn control_response(request_id: &str, action: ControlAction) -> WireEnvelope {
    let mut response = WireEnvelope::global(
        MessageType::ControlResult,
        1,
        &ControlResultPayload {
            action,
            accepted: true,
            reason: ReasonCode::Accepted,
            paused: action == ControlAction::Pause,
            snooze: None,
        },
    )
    .expect("control response")
    .at_version(CURRENT_PROTOCOL_VERSION)
    .expect("v2 control response");
    response.id = Some(request_id.to_owned());
    response
}

#[test]
fn post_hello_responses_require_matching_id_type_and_control_payload() {
    let response = control_response("ctl:request", ControlAction::Pause);
    assert!(validate_control_response(&response, "ctl:request", ControlAction::Pause).is_ok());
    assert!(matches!(
        validate_correlated_response(&response, "ctl:other", MessageType::ControlResult),
        Err(CliError::UnexpectedResponse)
    ));
    assert!(matches!(
        validate_correlated_response(&response, "ctl:request", MessageType::HealthStatus),
        Err(CliError::UnexpectedResponse)
    ));
    assert!(matches!(
        validate_control_response(&response, "ctl:request", ControlAction::Resume),
        Err(CliError::UnexpectedResponse)
    ));

    let mut contradictory_pause = control_response("ctl:request", ControlAction::Pause);
    contradictory_pause.payload["paused"] = json!(false);
    assert!(matches!(
        validate_control_response(&contradictory_pause, "ctl:request", ControlAction::Pause),
        Err(CliError::UnexpectedResponse)
    ));
    let mut still_persistently_paused = control_response("ctl:request", ControlAction::Resume);
    still_persistently_paused.payload["paused"] = json!(true);
    assert!(
        validate_control_response(
            &still_persistently_paused,
            "ctl:request",
            ControlAction::Resume,
        )
        .is_ok()
    );

    let mut rejected = control_response("ctl:request", ControlAction::Pause);
    rejected.payload["accepted"] = json!(false);
    assert!(matches!(
        validate_control_response(&rejected, "ctl:request", ControlAction::Pause),
        Err(CliError::UnexpectedResponse)
    ));
    let mut wrong_reason = control_response("ctl:request", ControlAction::Pause);
    wrong_reason.payload["reason"] = json!("provider_error");
    assert!(matches!(
        validate_control_response(&wrong_reason, "ctl:request", ControlAction::Pause),
        Err(CliError::UnexpectedResponse)
    ));
}

#[test]
fn health_response_requires_correlated_type_and_fixed_transport_limits() {
    let mut response = WireEnvelope::global(
        MessageType::HealthStatus,
        1,
        &HealthStatusPayload {
            provider: ProviderKind::PhraseV1,
            paused: false,
            authority_epoch: 0,
            settings_revision: 0,
            control_plane_degraded: false,
            sessions: 0,
            socket_mode: "0600".to_owned(),
            max_frame_bytes: MAX_FRAME_BYTES,
            metrics: MetricsSnapshot::default(),
            active: None,
            desktop: None,
            snoozes: Vec::new(),
        },
    )
    .expect("health response")
    .at_version(CURRENT_PROTOCOL_VERSION)
    .expect("v2 health response");
    response.id = Some("ctl:health".to_owned());
    assert!(validate_health_response(&response, "ctl:health").is_ok());

    response.payload["max_frame_bytes"] = json!(MAX_FRAME_BYTES - 1);
    assert!(matches!(
        validate_health_response(&response, "ctl:health"),
        Err(CliError::UnexpectedResponse)
    ));
}

#[test]
fn parses_help_without_requiring_a_runtime_directory() {
    assert_eq!(
        parse_arguments(arguments(&["--help"])).expect("help"),
        ParsedCommand::Help
    );
    assert_eq!(
        parse_arguments(arguments(&["--socket", "/tmp/broker.sock", "-h"]))
            .expect("help after socket"),
        ParsedCommand::Help
    );
}

#[test]
fn parses_commands_with_absolute_socket() {
    assert_eq!(
        parse_arguments(arguments(&["--socket", "/tmp/broker.sock", "accept-word",]))
            .expect("accept word"),
        ParsedCommand::Remote {
            socket_path: PathBuf::from("/tmp/broker.sock"),
            command: Command::Session(ControlAction::AcceptWord),
        }
    );
    assert_eq!(
        parse_arguments(arguments(&["--socket", "/tmp/broker.sock", "pause", "off"]))
            .expect("pause off"),
        ParsedCommand::Remote {
            socket_path: PathBuf::from("/tmp/broker.sock"),
            command: global(ControlAction::Resume),
        }
    );
    for (word, command) in [
        ("status", StatsCommand::Status),
        ("forget", StatsCommand::Forget),
    ] {
        assert_eq!(
            parse_arguments(arguments(&["--socket", "/tmp/broker.sock", "stats", word]))
                .expect("stats"),
            ParsedCommand::Remote {
                socket_path: PathBuf::from("/tmp/broker.sock"),
                command: Command::Stats(command),
            }
        );
    }
    for rejected in [
        &["stats"][..],
        &["stats", "clear"],
        &["stats", "status", "--json"],
    ] {
        assert!(matches!(
            parse_arguments(arguments(rejected)),
            Err(CliError::Arguments)
        ));
    }
}

#[test]
fn parses_snooze_minutes_with_cotypists_default() {
    let snooze = |values: &[&str]| {
        let mut command = vec!["--socket", "/tmp/broker.sock", "snooze"];
        command.extend_from_slice(values);
        parse_arguments(arguments(&command))
    };
    let remote = |action, minutes| ParsedCommand::Remote {
        socket_path: PathBuf::from("/tmp/broker.sock"),
        command: Command::Global(GlobalControlRequestPayload { action, minutes }),
    };
    assert_eq!(
        snooze(&[]).expect("default"),
        remote(ControlAction::Snooze, Some(10))
    );
    assert_eq!(
        snooze(&["1440"]).expect("a day"),
        remote(ControlAction::Snooze, Some(1440))
    );
    assert_eq!(
        snooze(&["end"]).expect("end"),
        remote(ControlAction::SnoozeEnd, None)
    );
    for invalid in [
        &["0"][..],
        &["1441"],
        &["ten"],
        &["-5"],
        &["5", "6"],
        &["end", "now"],
    ] {
        assert!(
            matches!(snooze(invalid), Err(CliError::Arguments)),
            "{invalid:?}"
        );
    }
}

#[test]
fn snooze_replies_name_the_subject_or_why_there_was_none() {
    let subject = StableIdentity::linux_app(LinuxAdapter::Fcitx, "telegram").expect("app");
    let mut accepted = control_response("ctl:snooze", ControlAction::Snooze);
    accepted.payload["snooze"] = json!({"subject": subject, "remaining_ms": 600_000});
    let payload = validate_control_response(&accepted, "ctl:snooze", ControlAction::Snooze)
        .expect("accepted snooze");
    assert_eq!(payload.snooze.expect("status").subject, subject);

    let bare = control_response("ctl:snooze", ControlAction::Snooze);
    assert!(matches!(
        validate_control_response(&bare, "ctl:snooze", ControlAction::Snooze),
        Err(CliError::UnexpectedResponse)
    ));
    for reason in ["no_context", "ambiguous_session", "paused"] {
        let mut refused = control_response("ctl:snooze", ControlAction::Snooze);
        refused.payload["accepted"] = json!(false);
        refused.payload["reason"] = json!(reason);
        assert!(matches!(
            validate_control_response(&refused, "ctl:snooze", ControlAction::Snooze),
            Err(CliError::Rejected(code)) if code == reason
        ));
    }
    let mut stray = control_response("ctl:end", ControlAction::SnoozeEnd);
    assert!(validate_control_response(&stray, "ctl:end", ControlAction::SnoozeEnd).is_ok());
    stray.payload["snooze"] = json!({"subject": subject, "remaining_ms": 1});
    assert!(matches!(
        validate_control_response(&stray, "ctl:end", ControlAction::SnoozeEnd),
        Err(CliError::UnexpectedResponse)
    ));
}

#[test]
fn settings_replace_accepts_only_the_current_v2_schema() {
    let mut current = SettingsV2::deny_by_default();
    current.revision = 1;
    let current = serde_json::to_string(&current).expect("current settings JSON");
    assert!(
        parse_arguments(arguments(&[
            "--socket",
            "/tmp/broker.sock",
            "settings",
            "replace",
            "--if-revision",
            "0",
            "--json",
            &current,
        ]))
        .is_ok()
    );

    let legacy = r#"{"schema":"badi.settings.v1","revision":1,"paused":true,"subjects":[]}"#;
    assert!(matches!(
        parse_arguments(arguments(&[
            "--socket",
            "/tmp/broker.sock",
            "settings",
            "replace",
            "--if-revision",
            "0",
            "--json",
            legacy,
        ])),
        Err(CliError::Arguments)
    ));
}

#[tokio::test]
async fn settings_stdin_is_bounded_and_uses_the_same_schema_and_revision_checks() {
    let valid = br#"{"schema":"badi.settings.v2","revision":1,"paused":true,"subjects":[]}"#;
    let parsed = parse_arguments(arguments(&[
        "--socket",
        "/tmp/broker.sock",
        "settings",
        "replace",
        "--if-revision",
        "0",
        "--json",
        "-",
    ]))
    .expect("stdin option");
    assert!(matches!(
        parsed,
        ParsedCommand::Remote {
            command: Command::SettingsReplaceStdin {
                expected_revision: 0
            },
            ..
        }
    ));
    assert_eq!(
        super::read_settings_stdin(&valid[..], 0)
            .await
            .expect("stdin document"),
        super::settings_replacement(0, std::str::from_utf8(valid).unwrap()).unwrap()
    );
    let oversized = vec![b' '; super::MAX_SETTINGS_BYTES + 1];
    for invalid in [&b""[..], &b"\xff"[..], &b"{}"[..], &oversized[..]] {
        assert!(super::read_settings_stdin(invalid, 0).await.is_err());
    }
    assert!(super::read_settings_stdin(&valid[..], 1).await.is_err());
}

#[test]
fn parses_local_hardware_and_model_commands_without_a_socket() {
    assert_eq!(
        parse_arguments(arguments(&["hardware"])).expect("hardware"),
        ParsedCommand::Local(LocalCommand::Hardware)
    );
    assert_eq!(
        parse_arguments(arguments(&["models", "code", "--json"])).expect("code models"),
        ParsedCommand::Local(LocalCommand::Models(ModelUseCase::Code))
    );
    assert_eq!(
        parse_arguments(arguments(&["models"])).expect("writing models"),
        ParsedCommand::Local(LocalCommand::Models(ModelUseCase::Writing))
    );
}

#[test]
fn rejects_relative_socket_and_extra_arguments() {
    assert!(matches!(
        parse_arguments(arguments(&["--socket", "broker.sock", "status"])),
        Err(CliError::Arguments)
    ));
    assert!(matches!(
        parse_arguments(arguments(&["--help", "extra"])),
        Err(CliError::Arguments)
    ));
    assert!(matches!(
        parse_arguments(arguments(&["--socket", "/tmp/broker.sock", "hardware"])),
        Err(CliError::Arguments)
    ));
}

#[test]
fn status_json_redacts_all_active_locator_tokens() {
    let session_id = SessionId::new();
    let status = HealthStatusPayload {
        provider: ProviderKind::PhraseV1,
        paused: false,
        authority_epoch: 7,
        settings_revision: 11,
        control_plane_degraded: false,
        sessions: 1,
        socket_mode: "0600".to_owned(),
        max_frame_bytes: 65_536,
        metrics: MetricsSnapshot::default(),
        active: Some(ActiveLocator {
            session_id,
            focus_epoch: 7,
            revision: 11,
            fingerprint: "fingerprint_000000000001".to_owned(),
            suggestion_id: Some("s:secret".to_owned()),
        }),
        desktop: None,
        snoozes: vec![SnoozeStatus {
            subject: StableIdentity::linux_app(LinuxAdapter::Fcitx, "telegram").expect("app"),
            remaining_ms: 540_000,
        }],
    };

    let value = serde_json::to_value(redact_health_status(&status)).expect("redacted JSON");
    assert_eq!(
        value["active"],
        json!({ "present": true, "has_suggestion": true })
    );
    assert_eq!(
        value["snoozes"],
        json!([{
            "subject": {"kind": "linux_app", "adapter": "fcitx", "app_id": "telegram"},
            "remaining_ms": 540_000,
        }]),
        "status names what the user snoozed, as a settings identity"
    );
    let encoded = serde_json::to_string(&value).expect("encoded redacted JSON");
    for secret in [
        &session_id.to_string(),
        "fingerprint_000000000001",
        "s:secret",
    ] {
        assert!(!encoded.contains(secret));
    }

    let mut inactive = status;
    inactive.active = None;
    assert_eq!(
        serde_json::to_value(redact_health_status(&inactive)).expect("inactive JSON")["active"],
        json!({ "present": false, "has_suggestion": false })
    );
}

#[test]
fn version_is_local_and_exclusive() {
    assert_eq!(
        parse_arguments(arguments(&["--version"])).expect("version"),
        ParsedCommand::Version
    );
    assert!(matches!(
        parse_arguments(arguments(&["--version", "status"])),
        Err(CliError::Arguments)
    ));
}

#[test]
fn probe_parses_flags_text_and_bounds() {
    let parsed = |values: &[&str]| {
        let mut all = vec!["--socket", "/tmp/broker.sock", "probe"];
        all.extend_from_slice(values);
        parse_arguments(arguments(&all))
    };
    let probe_with =
        |before: &str, after: &str, language: &str, allow_replacement: bool, explicit: bool| {
            ParsedCommand::Remote {
                socket_path: PathBuf::from("/tmp/broker.sock"),
                command: Command::Probe {
                    payload: ProbeRequestPayload {
                        before: before.to_owned(),
                        after: after.to_owned(),
                        language: Some(language.to_owned()),
                        allow_replacement,
                        explicit,
                    },
                    text_from_stdin: false,
                },
            }
        };
    let probe = |before: &str, after: &str, language: &str, allow_replacement: bool| {
        probe_with(before, after, language, allow_replacement, false)
    };
    assert_eq!(
        parsed(&["Please find attached the"]).expect("default probe"),
        probe("Please find attached the", "", "en", false)
    );
    assert_eq!(
        parsed(&["--language", "fa", "--replace", "--after", " tail", "متن"])
            .expect("flagged probe"),
        probe("متن", " tail", "fa", true)
    );
    assert_eq!(
        parsed(&["--", "--not a flag"]).expect("separated text"),
        probe("--not a flag", "", "en", false)
    );
    assert_eq!(
        parsed(&["--explicit", "--language", "de", "Vielen Dank für Ihre "])
            .expect("explicit probe"),
        probe_with("Vielen Dank für Ihre ", "", "de", false, true)
    );
    assert_eq!(
        parsed(&["--", "--explicit"]).expect("literal flag text"),
        probe("--explicit", "", "en", false)
    );
    assert_eq!(
        parsed(&["--", "-"]).expect("literal dash"),
        probe("-", "", "en", false)
    );
    let ParsedCommand::Remote {
        command: Command::Probe {
            payload,
            text_from_stdin: true,
        },
        ..
    } = parsed(&["--language", "de", "-"]).expect("stdin probe")
    else {
        panic!("- selects standard input");
    };
    assert_eq!(
        (payload.before.as_str(), payload.language.as_deref()),
        ("", Some("de"))
    );
    for invalid in [
        &[][..],
        &["--language"][..],
        &["--unknown", "text"][..],
        &["one", "two"][..],
        &["-", "text"][..],
        &["--language", "e", "text"][..],
    ] {
        assert!(
            matches!(parsed(invalid), Err(CliError::Arguments)),
            "{invalid:?}"
        );
    }
    let oversized = "a".repeat(MAX_BEFORE_CHARS + 1);
    assert!(matches!(
        parsed(&[oversized.as_str()]),
        Err(CliError::Arguments)
    ));
}

#[tokio::test]
async fn probe_text_from_stdin_is_bounded_utf8_without_its_final_newline() {
    for (input, expected) in [
        (
            &b"Please find attached the\n"[..],
            "Please find attached the",
        ),
        (&b"Thank you\r\n"[..], "Thank you"),
        (&b"two lines\nkept\n\n"[..], "two lines\nkept\n"),
        (
            "\u{0645}\u{062a}\u{0646} ".as_bytes(),
            "\u{0645}\u{062a}\u{0646} ",
        ),
        (&b""[..], ""),
    ] {
        assert_eq!(read_probe_text(input).await.expect("probe text"), expected);
    }
    let widest = "\u{1f600}".repeat(MAX_BEFORE_CHARS) + "\r\n";
    assert_eq!(
        read_probe_text(widest.as_bytes())
            .await
            .expect("widest text")
            .chars()
            .count(),
        MAX_BEFORE_CHARS
    );
    let oversized = "a".repeat(MAX_BEFORE_CHARS * 4 + 3);
    for invalid in [&b"\xff\n"[..], oversized.as_bytes()] {
        assert!(matches!(
            read_probe_text(invalid).await,
            Err(CliError::Arguments)
        ));
    }
}

#[test]
fn probe_response_requires_a_consistent_result() {
    let response = |payload: serde_json::Value| {
        let mut envelope = WireEnvelope::global(MessageType::ProbeResult, 1, &payload)
            .expect("probe response")
            .at_version(CURRENT_PROTOCOL_VERSION)
            .expect("v2 probe response");
        envelope.id = Some("ctl:probe".to_owned());
        envelope
    };
    let suggested =
        ProbeResultPayload::suggested(ProviderKind::PhraseV1, " for your time".to_owned(), None, 3);
    assert_eq!(
        validate_probe_response(
            &response(serde_json::to_value(&suggested).expect("json")),
            "ctl:probe"
        )
        .expect("valid result"),
        suggested
    );
    for invalid in [
        json!({"outcome": "no_suggestion", "provider": "local_model", "latency_ms": 1}),
        json!({"outcome": "paused", "provider": "local_model", "text": " kept"}),
        json!({"outcome": "suggested", "provider": "local_model", "text": " a", "reason": "stale", "latency_ms": 1}),
    ] {
        assert!(matches!(
            validate_probe_response(&response(invalid), "ctl:probe"),
            Err(CliError::UnexpectedResponse)
        ));
    }
}
