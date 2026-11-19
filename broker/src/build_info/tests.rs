use super::{COMMIT, DIRTY, version_line};

#[test]
fn identity_is_a_full_commit_or_explicitly_unknown() {
    assert!(
        COMMIT == "unknown"
            || (COMMIT.len() == 40
                && COMMIT
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    );
    assert!(matches!(DIRTY, "true" | "false" | "unknown"));
    if COMMIT == "unknown" {
        assert_eq!(DIRTY, "unknown");
    }
    assert_eq!(
        version_line("badictl"),
        format!(
            "badictl {} commit={COMMIT} dirty={DIRTY}",
            env!("CARGO_PKG_VERSION")
        )
    );
}
