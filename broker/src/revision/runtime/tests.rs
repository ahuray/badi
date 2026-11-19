use std::path::Path;

use super::{cgroup_headroom_mib, token_budget};

#[test]
fn the_service_limit_counts_what_the_kernel_cannot_drop() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let group = directory.path().join("user.slice/badi-broker.service");
    std::fs::create_dir_all(&group).expect("cgroup directory");
    let membership = directory.path().join("cgroup");
    std::fs::write(&membership, "0::/user.slice/badi-broker.service\n").expect("membership");
    let mib = 1_024 * 1_024_u64;
    std::fs::write(group.join("memory.max"), format!("{}\n", 3_746 * mib)).expect("limit");
    std::fs::write(group.join("memory.current"), format!("{}\n", 2_000 * mib)).expect("current");
    std::fs::write(
        group.join("memory.stat"),
        format!("anon 1\nfile {}\n", 1_000 * mib),
    )
    .expect("stat");
    assert_eq!(
        cgroup_headroom_mib(&membership, directory.path()),
        Some(2_746)
    );
    std::fs::write(group.join("memory.max"), "max\n").expect("no limit");
    assert_eq!(cgroup_headroom_mib(&membership, directory.path()), None);
    assert_eq!(
        cgroup_headroom_mib(Path::new("/nonexistent"), directory.path()),
        None
    );
}

#[test]
fn a_longer_selection_may_use_more_tokens_within_the_runtime_context() {
    assert_eq!(token_budget(0), 64);
    assert_eq!(token_budget(300), 248);
    assert_eq!(token_budget(600), 448);
    assert_eq!(token_budget(5_000), 512);
}
