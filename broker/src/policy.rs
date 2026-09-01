use crate::protocol::{Activation, FieldDescriptor, FieldPurpose, TargetKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyReason {
    AllowedAlways,
    AllowedExplicit,
    FieldAmbiguous,
    FieldNotEditable,
    FieldSensitive,
    ManualRequired,
    Paused,
    PolicyNever,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyDecision {
    Allow(PolicyReason),
    ManualRequired(PolicyReason),
    Deny(PolicyReason),
}

#[derive(Clone, Copy, Debug)]
pub struct PolicyInput {
    pub activation: Activation,
    pub explicit: bool,
    pub field: FieldDescriptor,
    pub target_kind: TargetKind,
    pub paused: bool,
    pub selection_collapsed: bool,
}

#[must_use]
pub const fn evaluate(input: PolicyInput) -> PolicyDecision {
    if input.field.sensitive
        || input.field.lock_screen
        || matches!(
            input.field.purpose,
            FieldPurpose::Password
                | FieldPurpose::Pin
                | FieldPurpose::Otp
                | FieldPurpose::PaymentSecret
        )
    {
        return PolicyDecision::Deny(PolicyReason::FieldSensitive);
    }
    if !input.field.editable {
        return PolicyDecision::Deny(PolicyReason::FieldNotEditable);
    }
    if !input.field.focused || input.field.composing || !input.selection_collapsed {
        return PolicyDecision::Deny(PolicyReason::FieldAmbiguous);
    }
    if input.paused {
        return PolicyDecision::Deny(PolicyReason::Paused);
    }
    if matches!(input.activation, Activation::Never) {
        return PolicyDecision::Deny(PolicyReason::PolicyNever);
    }
    if (matches!(input.target_kind, TargetKind::Terminal)
        || matches!(input.field.purpose, FieldPurpose::Terminal))
        && !input.explicit
    {
        return PolicyDecision::ManualRequired(PolicyReason::ManualRequired);
    }
    if !input.field.identity_known || matches!(input.field.purpose, FieldPurpose::Unknown) {
        return if input.explicit {
            PolicyDecision::Allow(PolicyReason::AllowedExplicit)
        } else {
            PolicyDecision::ManualRequired(PolicyReason::ManualRequired)
        };
    }
    match input.activation {
        Activation::Always => PolicyDecision::Allow(PolicyReason::AllowedAlways),
        Activation::Manual if input.explicit => {
            PolicyDecision::Allow(PolicyReason::AllowedExplicit)
        }
        Activation::Manual => PolicyDecision::ManualRequired(PolicyReason::ManualRequired),
        Activation::Never => PolicyDecision::Deny(PolicyReason::PolicyNever),
    }
}

#[cfg(test)]
mod tests {
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
}
