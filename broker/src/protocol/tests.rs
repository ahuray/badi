use serde_json::json;

use super::{
    AdapterDescriptor, AdapterKind, AutomaticScheduling, CURRENT_PROTOCOL_VERSION, Capability,
    DesktopSession, DesktopStatePayload, DesktopStatusPayload, HelloPayload,
    LIBREOFFICE_WRITER_APP_ID, MAX_SAFE_COUNTER, MessageType, Origin, OriginScheme,
    PolicyQueryPayload, ProtocolError, ScreenShare, SessionId, TargetDescriptor, TargetKind,
    WireEnvelope, valid_language_tag,
};

#[test]
fn hello_accepts_each_base_capability() {
    let hello = HelloPayload {
        min_v: CURRENT_PROTOCOL_VERSION,
        max_v: CURRENT_PROTOCOL_VERSION,
        adapter: AdapterDescriptor {
            kind: AdapterKind::Test,
            name: "all-capabilities".to_owned(),
            version: "1".to_owned(),
        },
        capabilities: vec![
            Capability::Context,
            Capability::Suggestion,
            Capability::CommitApplied,
            Capability::CommitDispatchedUnverified,
            Capability::Control,
            Capability::Health,
            Capability::Policy,
            Capability::Settings,
        ],
    };
    hello.validate().expect("all schema capabilities");
}

#[test]
fn window_text_is_bounded_and_only_for_an_observed_unrestricted_field() {
    let context = |window_text: &str| super::ContextChangedPayload {
        fingerprint: "fingerprint_window_01".to_owned(),
        before: "Hi Marin".to_owned(),
        after: String::new(),
        selection: super::Selection {
            anchor: 8,
            head: 8,
            unit: super::OffsetUnit::UnicodeScalarValues,
        },
        field: super::FieldDescriptor {
            purpose: super::FieldPurpose::Normal,
            editable: true,
            multiline: true,
            composing: false,
            sensitive: false,
            identity_known: true,
            focused: true,
            lock_screen: false,
        },
        activation: super::Activation::Always,
        explicit: false,
        language: Some("en".to_owned()),
        window_text: Some(window_text.to_owned()),
    };
    context("To: Marinella Costa")
        .validate()
        .expect("observed field");
    context(&"é".repeat(super::MAX_WINDOW_TEXT_CHARS))
        .validate()
        .expect("at the bound");
    assert!(
        context(&"é".repeat(super::MAX_WINDOW_TEXT_CHARS + 1))
            .validate()
            .is_err()
    );
    let mut unobserved = context("To: Marinella Costa");
    unobserved.field.identity_known = false;
    assert!(unobserved.validate().is_err());
    let mut sensitive = context("To: Marinella Costa");
    sensitive.field.sensitive = true;
    sensitive.before.clear();
    assert!(sensitive.validate().is_err());
}

#[test]
fn a_hello_names_exactly_the_current_version() {
    let hello = |kind, min_v, max_v| HelloPayload {
        min_v,
        max_v,
        adapter: AdapterDescriptor {
            kind,
            name: "adapter".to_owned(),
            version: "1".to_owned(),
        },
        capabilities: vec![
            Capability::Context,
            Capability::Suggestion,
            Capability::CommitApplied,
            Capability::LongReadingWindow,
        ],
    };
    for kind in [
        AdapterKind::Fcitx,
        AdapterKind::Terminal,
        AdapterKind::Neovim,
        AdapterKind::Libreoffice,
    ] {
        hello(kind, 2, 2).validate().expect("the current version");
        for (min_v, max_v) in [(1, 1), (1, 2), (2, 3), (3, 3)] {
            assert!(
                matches!(
                    hello(kind, min_v, max_v).validate(),
                    Err(ProtocolError::VersionNegotiationFailed)
                ),
                "{kind:?} {min_v}..{max_v}"
            );
        }
    }
}

#[test]
fn selection_revision_requires_an_implemented_editor() {
    let hello = |kind| HelloPayload {
        min_v: CURRENT_PROTOCOL_VERSION,
        max_v: CURRENT_PROTOCOL_VERSION,
        adapter: AdapterDescriptor {
            kind,
            name: "adapter".to_owned(),
            version: "1".to_owned(),
        },
        capabilities: vec![
            Capability::Context,
            Capability::Suggestion,
            Capability::SelectionRevision,
        ],
    };
    hello(AdapterKind::Fcitx)
        .validate()
        .expect("the Fcitx addon");
    hello(AdapterKind::Neovim)
        .validate()
        .expect("the Neovim editor");
    hello(AdapterKind::Obsidian)
        .validate()
        .expect("the Obsidian editor");
    assert!(hello(AdapterKind::Terminal).validate().is_err());
}

#[test]
fn alternative_lists_belong_to_the_addon_and_the_editors() {
    let hello = |kind| HelloPayload {
        min_v: CURRENT_PROTOCOL_VERSION,
        max_v: CURRENT_PROTOCOL_VERSION,
        adapter: AdapterDescriptor {
            kind,
            name: "adapter".to_owned(),
            version: "1".to_owned(),
        },
        capabilities: vec![
            Capability::Context,
            Capability::Suggestion,
            Capability::AlternativeList,
        ],
    };
    for kind in [
        AdapterKind::Fcitx,
        AdapterKind::Obsidian,
        AdapterKind::Neovim,
    ] {
        hello(kind).validate().expect("a list-capable adapter");
    }
    for kind in [AdapterKind::Terminal, AdapterKind::Libreoffice] {
        assert!(hello(kind).validate().is_err(), "{kind:?}");
    }
}

#[test]
fn a_writer_target_names_its_exact_app_id_and_no_origin() {
    let mut target = TargetDescriptor {
        kind: TargetKind::Libreoffice,
        app_id: LIBREOFFICE_WRITER_APP_ID.to_owned(),
        target_id: "document".to_owned(),
        origin: None,
    };
    target.validate_wire().expect("exact Writer target");
    target.app_id = "libreoffice".to_owned();
    assert!(target.validate().is_err());
    target.app_id = LIBREOFFICE_WRITER_APP_ID.to_owned();
    target.origin = Some(Origin {
        scheme: OriginScheme::File,
        host: "localhost".to_owned(),
        port: None,
    });
    assert!(target.validate().is_err());
}

#[test]
fn a_browser_target_on_the_wire_names_its_origin() {
    let mut query = PolicyQueryPayload {
        target: TargetDescriptor {
            kind: TargetKind::Browser,
            app_id: "chromium".to_owned(),
            target_id: "field-1".to_owned(),
            origin: None,
        },
    };
    assert!(query.validate().is_err());
    query.target.origin = Some(Origin {
        scheme: OriginScheme::Https,
        host: "example.com".to_owned(),
        port: None,
    });
    query.validate().expect("a browser target with its origin");
    for (kind, app_id) in [
        (TargetKind::Neovim, "nvim"),
        (TargetKind::DesktopApplication, "omawrite"),
    ] {
        PolicyQueryPayload {
            target: TargetDescriptor {
                kind,
                app_id: app_id.to_owned(),
                target_id: "ic:42".to_owned(),
                origin: None,
            },
        }
        .validate()
        .expect("an editor or desktop target");
    }
}

#[test]
fn an_envelope_of_any_other_version_is_unsupported() {
    for version in [0, 1, 3] {
        let envelope: WireEnvelope = serde_json::from_value(json!({
            "v": version,
            "type": "health.request",
            "mono_ms": 4,
            "payload": {}
        }))
        .expect("wire decoding should succeed before shape validation");
        assert!(matches!(
            envelope.validate_shape(),
            Err(ProtocolError::UnsupportedVersion(v)) if v == version
        ));
    }
}

#[test]
fn rejects_coordinates_on_global_message() {
    let envelope: WireEnvelope = serde_json::from_value(json!({
        "v": 2,
        "type": "health.request",
        "session_id": SessionId::new(),
        "focus_epoch": 0,
        "revision": 0,
        "mono_ms": 4,
        "payload": {}
    }))
    .expect("wire decoding should succeed before shape validation");

    assert!(matches!(
        envelope.validate_shape(),
        Err(ProtocolError::UnexpectedCoordinates)
    ));
    assert_eq!(envelope.message_type, MessageType::HealthRequest);
}

#[test]
fn authority_messages_reject_correlation_ids_for_schema_parity() {
    for message_type in ["authority.changed", "authority.ack"] {
        let envelope: WireEnvelope = serde_json::from_value(json!({
            "v": 2,
            "id": "unexpected",
            "type": message_type,
            "mono_ms": 4,
            "payload": if message_type == "authority.changed" {
                json!({"authority_epoch": 1, "settings_revision": 1, "paused": true})
            } else {
                json!({"authority_epoch": 1})
            }
        }))
        .expect("wire decoding should succeed before shape validation");
        assert!(matches!(
            envelope.validate_shape(),
            Err(ProtocolError::UnexpectedId)
        ));
    }
}

#[test]
fn rejects_noncanonical_session_id() {
    let result = serde_json::from_value::<WireEnvelope>(json!({
        "v": 2,
        "type": "session.open",
        "session_id": "550E8400-E29B-41D4-A716-446655440000",
        "focus_epoch": 0,
        "revision": 0,
        "mono_ms": 4,
        "payload": {}
    }));
    assert!(result.is_err());
}

#[test]
fn rejects_canonical_non_rfc_session_id_variant() {
    let result = serde_json::from_value::<WireEnvelope>(json!({
        "v": 2,
        "type": "session.open",
        "session_id": "550e8400-e29b-41d4-7716-446655440000",
        "focus_epoch": 0,
        "revision": 0,
        "mono_ms": 4,
        "payload": {}
    }));
    assert!(result.is_err());
}

#[test]
fn addressed_control_rejects_unsafe_javascript_counter() {
    let base = json!({
        "v": 2,
        "type": "control.request",
        "session_id": SessionId::new(),
        "focus_epoch": 0,
        "revision": 0,
        "mono_ms": 4,
        "payload": {
            "action": "request",
            "fingerprint": "fingerprint_000000000001"
        }
    });
    let mut unsafe_focus = base.clone();
    unsafe_focus["focus_epoch"] = json!(MAX_SAFE_COUNTER + 1);
    let envelope: WireEnvelope = serde_json::from_value(unsafe_focus)
        .expect("wire decoding should succeed before shape validation");
    assert!(matches!(
        envelope.validate_shape(),
        Err(ProtocolError::CounterOutOfRange("focus_epoch"))
    ));

    let mut unsafe_revision = base;
    unsafe_revision["revision"] = json!(MAX_SAFE_COUNTER + 1);
    let envelope: WireEnvelope = serde_json::from_value(unsafe_revision)
        .expect("wire decoding should succeed before shape validation");
    assert!(matches!(
        envelope.validate_shape(),
        Err(ProtocolError::CounterOutOfRange("revision"))
    ));
}

#[test]
fn language_tags_require_nonempty_ascii_alphanumeric_subtags() {
    for valid in ["en", "en-US", "zh-Hant-TW", "x-private"] {
        assert!(valid_language_tag(valid), "{valid}");
    }
    for invalid in ["en-", "-en", "en--x", "en_US", "e", "fr-ça"] {
        assert!(!valid_language_tag(invalid), "{invalid}");
    }
}

#[test]
fn spelling_suffixes_preserve_one_optional_ascii_space() {
    for (original, corrected) in [
        ("teh", "the"),
        ("teh ", "the "),
        ("adress ", "address "),
        ("Hauss ", "Haus "),
        ("grasias ", "gracias "),
        ("één ", "een "),
    ] {
        assert!(super::valid_spelling_replacement(original, corrected));
    }
    for (original, corrected) in [
        ("teh ", "the"),
        ("teh", "the "),
        ("teh  ", "the  "),
        ("teh\t", "the\t"),
        ("teh\n", "the\n"),
        ("teh\u{00a0}", "the\u{00a0}"),
        ("teh.", "the."),
        ("teh x", "the x"),
        ("teh ", "teh "),
        ("t ", "to "),
        ("mot-clef ", "motclef "),
        ("x2 ", "xz "),
        ("سلام ", "سلم "),
    ] {
        assert!(
            !super::valid_spelling_replacement(original, corrected),
            "{original:?} -> {corrected:?}"
        );
    }
    assert!(super::valid_spelling_replacement(
        &format!("{} ", "ä".repeat(32)),
        &format!("{} ", "b".repeat(32))
    ));
    assert!(!super::valid_spelling_replacement(
        &format!("{} ", "a".repeat(33)),
        &format!("{} ", "b".repeat(33))
    ));
}

#[test]
fn a_desktop_report_names_the_screen_share_and_only_a_share_outlives_its_helper() {
    let shared: DesktopStatePayload = serde_json::from_value(serde_json::json!({
        "session": "active", "on_battery": false, "power_saver": false, "screen_shared": true,
    }))
    .expect("a complete report");
    assert!(shared.screen_shared);
    assert!(
        serde_json::from_value::<DesktopStatePayload>(serde_json::json!({
            "session": "active", "on_battery": false, "power_saver": false,
        }))
        .is_err(),
        "every report says whether the screen is shared"
    );
    let held = DesktopStatusPayload {
        observed: true,
        session: DesktopSession::Active,
        low_power: false,
        automatic: AutomaticScheduling::Normal,
        screen_share: ScreenShare::Held,
    };
    held.validate().expect("a reported share may hold");
    DesktopStatusPayload {
        observed: false,
        ..held
    }
    .validate()
    .expect("a share stays held for a grace after its helper left");
    assert!(
        DesktopStatusPayload {
            observed: false,
            session: DesktopSession::Sleeping,
            ..held
        }
        .validate()
        .is_err(),
        "no helper, no session hold"
    );
}
