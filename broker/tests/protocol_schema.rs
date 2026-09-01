use std::fs;
use std::path::{Path, PathBuf};

use badi_broker::protocol::{
    AuthorityAckPayload, AuthorityChangedPayload, CommitPreparePayload, CommitResultPayload,
    ContextChangedPayload, ControlAction, ControlResultPayload, EmptyPayload, ErrorPayload,
    GlobalControlRequestPayload, HealthStatusPayload, HelloAckPayload, HelloPayload,
    MemoryStatusPayload, MessageType, PolicyQueryPayload, PolicyStatusPayload, SessionClosePayload,
    SessionControlRequestPayload, SessionOpenPayload, SettingsReplacePayload,
    SettingsStatusPayload, SuggestCancelPayload, SuggestRequestPayload, SuggestionClearPayload,
    SuggestionShowPayload, WireEnvelope, valid_opaque_id, validate_fingerprint,
};
use badi_broker::segment::{accept_word, sanitize_suggestion};
use serde::de::DeserializeOwned;
use serde_json::Value;

fn protocol_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("broker has workspace parent")
        .join("protocol/v1")
}

fn protocol_v2_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("broker has workspace parent")
        .join("protocol/v2")
}

fn rust_scalar_schema(mut schema: Value) -> Value {
    // serde_json strings cannot contain lone UTF-16 surrogates. Keep the ECMA-only
    // exclusion normative, assert its presence, and compile the scalar-representable
    // remainder here; the browser/Ajv suite exercises the unmodified pattern.
    for pointer in [
        "/$defs/safeSuggestionText/allOf/0/pattern",
        "/$defs/contextText/pattern",
    ] {
        let pattern = schema
            .pointer(pointer)
            .and_then(Value::as_str)
            .expect("shared surrogate-safe pattern");
        assert!(
            pattern.contains(r"\ud800-\udfff"),
            "the normative ECMA pattern must reject lone UTF-16 surrogates"
        );
        let scalar_pattern = if pointer == "/$defs/contextText/pattern" {
            "(?s:.*)".to_owned()
        } else {
            pattern.replace(r"\ud800-\udfff", "")
        };
        *schema
            .pointer_mut(pointer)
            .expect("shared surrogate-safe pattern") = Value::String(scalar_pattern);
    }
    schema
}

fn fixture_files(kind: &str) -> Vec<PathBuf> {
    let mut files = fs::read_dir(protocol_root().join("examples").join(kind))
        .expect("fixture directory")
        .map(|entry| entry.expect("fixture entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn decode<T: DeserializeOwned>(envelope: &WireEnvelope) -> T {
    envelope.decode_payload().expect("typed payload")
}

// One exhaustive match keeps Rust decoding coverage visibly aligned with the
// protocol schema's complete message enum.
#[allow(clippy::too_many_lines)]
fn validate_rust_payload(envelope: &WireEnvelope) {
    match envelope.message_type {
        MessageType::Hello => decode::<HelloPayload>(envelope)
            .validate()
            .expect("valid hello payload"),
        MessageType::HelloAck => {
            decode::<HelloAckPayload>(envelope)
                .validate()
                .expect("valid hello acknowledgment");
        }
        MessageType::SessionOpen => decode::<SessionOpenPayload>(envelope)
            .target
            .validate_for_version(envelope.v)
            .expect("valid target"),
        MessageType::SessionClose => {
            let _: SessionClosePayload = decode(envelope);
        }
        MessageType::ContextChanged => decode::<ContextChangedPayload>(envelope)
            .validate_for_version(envelope.v)
            .expect("valid context"),
        MessageType::SuggestRequest => {
            let payload: SuggestRequestPayload = decode(envelope);
            validate_fingerprint(&payload.fingerprint).expect("valid fingerprint");
        }
        MessageType::SuggestCancel => {
            let payload: SuggestCancelPayload = decode(envelope);
            validate_fingerprint(&payload.fingerprint).expect("valid fingerprint");
        }
        MessageType::SuggestionShow => {
            let payload: SuggestionShowPayload = decode(envelope);
            assert_eq!(
                sanitize_suggestion(&payload.text).expect("safe suggestion"),
                payload.text
            );
            assert_eq!(accept_word(&payload.text).accepted, payload.accept_word);
            assert!(valid_opaque_id(&payload.suggestion_id));
        }
        MessageType::SuggestionClear => {
            let payload: SuggestionClearPayload = decode(envelope);
            validate_fingerprint(&payload.fingerprint).expect("valid fingerprint");
        }
        MessageType::ControlRequest => {
            let action: ControlAction = serde_json::from_value(
                envelope
                    .payload
                    .get("action")
                    .expect("control action")
                    .clone(),
            )
            .expect("known action");
            if action.is_global() {
                decode::<GlobalControlRequestPayload>(envelope)
                    .validate()
                    .expect("valid global control");
            } else {
                decode::<SessionControlRequestPayload>(envelope)
                    .validate()
                    .expect("valid session control");
            }
        }
        MessageType::ControlResult => {
            let _: ControlResultPayload = decode(envelope);
        }
        MessageType::CommitPrepare => {
            let payload: CommitPreparePayload = decode(envelope);
            assert_eq!(
                sanitize_suggestion(&payload.text).expect("safe commit text"),
                payload.text
            );
        }
        MessageType::CommitResult => decode::<CommitResultPayload>(envelope)
            .validate()
            .expect("valid commit result"),
        MessageType::HealthRequest | MessageType::SettingsGet | MessageType::MemoryClear => {
            let _: EmptyPayload = decode(envelope);
        }
        MessageType::HealthStatus => {
            let _: HealthStatusPayload = decode(envelope);
        }
        MessageType::PolicyQuery => decode::<PolicyQueryPayload>(envelope)
            .validate_for_version(envelope.v)
            .expect("valid policy query"),
        MessageType::PolicyStatus => decode::<PolicyStatusPayload>(envelope)
            .validate()
            .expect("valid policy status"),
        MessageType::AuthorityChanged => decode::<AuthorityChangedPayload>(envelope)
            .validate()
            .expect("valid authority change"),
        MessageType::AuthorityAck => decode::<AuthorityAckPayload>(envelope)
            .validate()
            .expect("valid authority acknowledgment"),
        MessageType::SettingsReplace => decode::<SettingsReplacePayload>(envelope)
            .validate()
            .expect("valid settings replacement"),
        MessageType::SettingsStatus => decode::<SettingsStatusPayload>(envelope)
            .validate()
            .expect("valid settings status"),
        MessageType::MemoryStatus => decode::<MemoryStatusPayload>(envelope)
            .validate()
            .expect("valid memory status"),
        MessageType::Error => {
            let _: ErrorPayload = decode(envelope);
        }
    }
}

#[test]
fn every_positive_fixture_passes_schema_and_rust_types() {
    let schema = rust_scalar_schema(
        serde_json::from_str(
            &fs::read_to_string(protocol_root().join("schema.json")).expect("schema file"),
        )
        .expect("schema JSON"),
    );
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let fixtures = fixture_files("valid");
    assert!(fixtures.len() >= 18, "all message families need fixtures");

    for path in fixtures {
        let instance: Value = serde_json::from_str(
            &fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {}", path.display())),
        )
        .unwrap_or_else(|_| panic!("parse {}", path.display()));
        if let Err(error) = validator.validate(&instance) {
            panic!("{} failed schema: {error}", path.display());
        }
        let envelope: WireEnvelope = serde_json::from_value(instance)
            .unwrap_or_else(|_| panic!("decode {}", path.display()));
        envelope
            .validate_shape()
            .unwrap_or_else(|_| panic!("shape {}", path.display()));
        validate_rust_payload(&envelope);
    }
}

#[test]
fn every_negative_scalar_fixture_is_rejected_by_normative_schema() {
    let schema = rust_scalar_schema(
        serde_json::from_str(
            &fs::read_to_string(protocol_root().join("schema.json")).expect("schema file"),
        )
        .expect("schema JSON"),
    );
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let fixtures = fixture_files("invalid");
    assert!(fixtures.len() >= 9, "negative boundary fixtures required");

    for path in fixtures {
        let instance: Value = serde_json::from_str(
            &fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {}", path.display())),
        )
        .unwrap_or_else(|_| panic!("parse {}", path.display()));
        assert!(
            !validator.is_valid(&instance),
            "{} unexpectedly passed schema",
            path.display()
        );
    }
}

#[test]
fn every_v2_fixture_matches_schema_and_versioned_rust_contracts() {
    let root = protocol_v2_root();
    let schema = rust_scalar_schema(
        serde_json::from_str(
            &fs::read_to_string(root.join("schema.json")).expect("v2 schema file"),
        )
        .expect("v2 schema JSON"),
    );
    let validator = jsonschema::validator_for(&schema).expect("v2 schema compiles");

    for kind in ["valid", "invalid"] {
        let mut fixtures = fs::read_dir(root.join("examples").join(kind))
            .expect("v2 fixture directory")
            .map(|entry| entry.expect("v2 fixture entry").path())
            .collect::<Vec<_>>();
        fixtures.sort();
        assert!(!fixtures.is_empty(), "v2 {kind} fixtures required");
        for path in fixtures {
            let instance: Value = serde_json::from_str(
                &fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {}", path.display())),
            )
            .unwrap_or_else(|_| panic!("parse {}", path.display()));
            if kind == "valid" {
                validator
                    .validate(&instance)
                    .unwrap_or_else(|error| panic!("{} failed schema: {error}", path.display()));
                let envelope: WireEnvelope = serde_json::from_value(instance)
                    .unwrap_or_else(|_| panic!("decode {}", path.display()));
                envelope
                    .validate_shape()
                    .unwrap_or_else(|_| panic!("shape {}", path.display()));
                validate_rust_payload(&envelope);
            } else {
                assert!(
                    !validator.is_valid(&instance),
                    "{} unexpectedly passed v2 schema",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn protocol_versions_keep_offset_and_target_semantics_disjoint() {
    let v1_schema = rust_scalar_schema(
        serde_json::from_str(
            &fs::read_to_string(protocol_root().join("schema.json")).expect("v1 schema file"),
        )
        .expect("v1 schema JSON"),
    );
    let v2_schema = rust_scalar_schema(
        serde_json::from_str(
            &fs::read_to_string(protocol_v2_root().join("schema.json")).expect("v2 schema file"),
        )
        .expect("v2 schema JSON"),
    );
    let v1 = jsonschema::validator_for(&v1_schema).expect("v1 schema compiles");
    let v2 = jsonschema::validator_for(&v2_schema).expect("v2 schema compiles");

    let desktop: Value = serde_json::from_str(
        &fs::read_to_string(protocol_v2_root().join("examples/valid/session_open_desktop.json"))
            .expect("desktop fixture"),
    )
    .expect("desktop JSON");
    assert!(v2.is_valid(&desktop));
    assert!(!v1.is_valid(&desktop));

    let scalar: Value = serde_json::from_str(
        &fs::read_to_string(protocol_v2_root().join("examples/valid/context_changed_scalar.json"))
            .expect("scalar fixture"),
    )
    .expect("scalar JSON");
    assert!(v2.is_valid(&scalar));
    assert!(!v1.is_valid(&scalar));
}

#[test]
fn schema_enforces_browser_first_character_bounds() {
    let schema = rust_scalar_schema(
        serde_json::from_str(
            &fs::read_to_string(protocol_root().join("schema.json")).expect("schema file"),
        )
        .expect("schema JSON"),
    );
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let mut context: Value = serde_json::from_str(
        &fs::read_to_string(protocol_root().join("examples/valid/context_changed.json"))
            .expect("context fixture"),
    )
    .expect("context JSON");

    context["payload"]["before"] = Value::String("a".repeat(512));
    assert!(validator.is_valid(&context));
    context["payload"]["before"] = Value::String("a".repeat(513));
    assert!(!validator.is_valid(&context));
    context["payload"]["before"] = Value::String("line one\n\t\0🙂".to_owned());
    context["payload"]["after"] = Value::String("\r\n🚀".to_owned());
    assert!(validator.is_valid(&context));

    let mut suggestion: Value = serde_json::from_str(
        &fs::read_to_string(protocol_root().join("examples/valid/suggestion_show.json"))
            .expect("suggestion fixture"),
    )
    .expect("suggestion JSON");
    suggestion["payload"]["text"] = Value::String("é".repeat(64));
    suggestion["payload"]["accept_word"] = Value::String("é".repeat(64));
    assert!(validator.is_valid(&suggestion));
    suggestion["payload"]["text"] = Value::String("é".repeat(65));
    assert!(!validator.is_valid(&suggestion));
    for invalid_spacing in ["\u{00a0}world", " world  again", "valid "] {
        suggestion["payload"]["text"] = Value::String(invalid_spacing.to_owned());
        suggestion["payload"]["accept_word"] = Value::String(invalid_spacing.to_owned());
        assert!(!validator.is_valid(&suggestion), "{invalid_spacing:?}");
    }
}

#[test]
fn normative_ecma_schema_declares_lone_surrogates_forbidden() {
    let schema: Value = serde_json::from_str(
        &fs::read_to_string(protocol_root().join("schema.json")).expect("schema file"),
    )
    .expect("schema JSON");
    assert!(
        schema["$defs"]["safeSuggestionText"]["allOf"][0]["pattern"]
            .as_str()
            .is_some_and(|pattern| pattern.contains(r"\ud800-\udfff"))
    );
    assert!(
        schema["$defs"]["contextText"]["pattern"]
            .as_str()
            .is_some_and(|pattern| pattern.contains(r"\ud800-\udfff"))
    );
}
