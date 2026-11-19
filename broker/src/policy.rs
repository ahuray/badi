use crate::protocol::{Activation, FieldDescriptor, FieldPurpose, ReasonCode, TargetKind};

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

impl PolicyReason {
    /// The wire reason of a request refused for this reason. An allowing
    /// reason never refuses; should one, it refuses as `policy_never`.
    #[must_use]
    pub const fn refusal_code(self) -> ReasonCode {
        match self {
            Self::Paused => ReasonCode::Paused,
            Self::FieldSensitive => ReasonCode::FieldSensitive,
            Self::FieldNotEditable => ReasonCode::FieldNotEditable,
            Self::FieldAmbiguous => ReasonCode::FieldAmbiguous,
            Self::ManualRequired => ReasonCode::ManualRequired,
            Self::PolicyNever | Self::AllowedAlways | Self::AllowedExplicit => {
                ReasonCode::PolicyNever
            }
        }
    }
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
    // Bash acts only on explicit requests. The Writer extension may also ask
    // on its own after a finished word; the engine keeps it to corrections.
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
mod tests;
