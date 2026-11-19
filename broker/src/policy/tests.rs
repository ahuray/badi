use super::{PolicyDecision, PolicyInput, PolicyReason, evaluate};
use crate::protocol::{Activation, FieldDescriptor, FieldPurpose, TargetKind};

const fn field(purpose: FieldPurpose) -> FieldDescriptor {
    FieldDescriptor {
        purpose,
        editable: true,
        multiline: true,
        composing: false,
        sensitive: false,
        identity_known: true,
        focused: true,
        lock_screen: false,
    }
}

#[test]
fn hard_deny_beats_explicit_and_always() {
    for purpose in [
        FieldPurpose::Password,
        FieldPurpose::Pin,
        FieldPurpose::Otp,
        FieldPurpose::PaymentSecret,
    ] {
        assert_eq!(
            evaluate(PolicyInput {
                activation: Activation::Always,
                explicit: true,
                field: field(purpose),
                target_kind: TargetKind::Browser,
                paused: false,
                selection_collapsed: true,
            }),
            PolicyDecision::Deny(PolicyReason::FieldSensitive)
        );
    }
}

#[test]
fn unknown_field_requires_explicit_manual_authority() {
    let mut unknown = field(FieldPurpose::Unknown);
    unknown.identity_known = false;
    assert_eq!(
        evaluate(PolicyInput {
            activation: Activation::Always,
            explicit: false,
            field: unknown,
            target_kind: TargetKind::Browser,
            paused: false,
            selection_collapsed: true,
        }),
        PolicyDecision::ManualRequired(PolicyReason::ManualRequired)
    );
    assert_eq!(
        evaluate(PolicyInput {
            activation: Activation::Manual,
            explicit: true,
            field: unknown,
            target_kind: TargetKind::DesktopApplication,
            paused: false,
            selection_collapsed: true,
        }),
        PolicyDecision::Allow(PolicyReason::AllowedExplicit)
    );
}

#[test]
fn terminal_is_never_ambient() {
    assert_eq!(
        evaluate(PolicyInput {
            activation: Activation::Always,
            explicit: false,
            field: field(FieldPurpose::Terminal),
            target_kind: TargetKind::Terminal,
            paused: false,
            selection_collapsed: true,
        }),
        PolicyDecision::ManualRequired(PolicyReason::ManualRequired)
    );
}

#[test]
fn libreoffice_writer_extension_may_ask_on_its_own() {
    let input = |explicit| PolicyInput {
        activation: Activation::Always,
        explicit,
        field: field(FieldPurpose::Normal),
        target_kind: TargetKind::Libreoffice,
        paused: false,
        selection_collapsed: true,
    };
    assert_eq!(
        evaluate(input(false)),
        PolicyDecision::Allow(PolicyReason::AllowedAlways)
    );
    assert_eq!(
        evaluate(input(true)),
        PolicyDecision::Allow(PolicyReason::AllowedAlways)
    );
}

#[test]
fn never_and_noncollapsed_selection_fail_closed() {
    assert_eq!(
        evaluate(PolicyInput {
            activation: Activation::Never,
            explicit: true,
            field: field(FieldPurpose::Normal),
            target_kind: TargetKind::Browser,
            paused: false,
            selection_collapsed: true,
        }),
        PolicyDecision::Deny(PolicyReason::PolicyNever)
    );
    assert_eq!(
        evaluate(PolicyInput {
            activation: Activation::Always,
            explicit: false,
            field: field(FieldPurpose::Normal),
            target_kind: TargetKind::Browser,
            paused: false,
            selection_collapsed: false,
        }),
        PolicyDecision::Deny(PolicyReason::FieldAmbiguous)
    );
}
