use super::{BrokerError, SessionAuthority, validate_commit_authority};
use crate::protocol::{AdapterKind, Capability, CommitStatus};

#[test]
fn commit_status_requires_matching_adapter_kind_and_declared_capability() {
    let test_dispatch = SessionAuthority {
        adapter_kind: AdapterKind::Test,
        capabilities: vec![Capability::CommitDispatchedUnverified],
    };
    for status in [CommitStatus::DispatchedUnverified, CommitStatus::Applied] {
        assert!(matches!(
            validate_commit_authority(&test_dispatch, status),
            Err(BrokerError::InvalidCapability)
        ));
    }

    let test_applied = SessionAuthority {
        adapter_kind: AdapterKind::Test,
        capabilities: vec![Capability::CommitApplied],
    };
    assert!(validate_commit_authority(&test_applied, CommitStatus::Applied).is_ok());

    let test_undeclared = SessionAuthority {
        adapter_kind: AdapterKind::Test,
        capabilities: Vec::new(),
    };
    assert!(matches!(
        validate_commit_authority(&test_undeclared, CommitStatus::DispatchedUnverified),
        Err(BrokerError::InvalidCapability)
    ));

    let obsidian_wrong_status = SessionAuthority {
        adapter_kind: AdapterKind::Obsidian,
        capabilities: vec![Capability::CommitDispatchedUnverified],
    };
    assert!(matches!(
        validate_commit_authority(&obsidian_wrong_status, CommitStatus::DispatchedUnverified,),
        Err(BrokerError::InvalidCapability)
    ));

    let neovim_applied = SessionAuthority {
        adapter_kind: AdapterKind::Neovim,
        capabilities: vec![Capability::CommitApplied],
    };
    assert!(validate_commit_authority(&neovim_applied, CommitStatus::Applied).is_ok());
    assert!(matches!(
        validate_commit_authority(&neovim_applied, CommitStatus::DispatchedUnverified),
        Err(BrokerError::InvalidCapability)
    ));

    let fcitx_dispatch = SessionAuthority {
        adapter_kind: AdapterKind::Fcitx,
        capabilities: vec![Capability::CommitDispatchedUnverified],
    };
    assert!(validate_commit_authority(&fcitx_dispatch, CommitStatus::DispatchedUnverified).is_ok());
}
