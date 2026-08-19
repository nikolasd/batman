//! Integration test for `batcave lease release` (R86): the operator
//! remedy for a lease whose owning session correlation was never
//! persisted. Such a lease is unreleasable over RPC -- `workspace/release`
//! is owner-gated and a new session is a different principal -- so the
//! compiled CLI binary, run directly against the lease database with no
//! daemon involved, must be able to force-release it by id.

use std::path::PathBuf;
use std::process::Command;

use batman_protocol::{IsolationKind, LeaseMode, RunId};
use batman_runtime::paths::RuntimePaths;
use batman_runtime::workspace::LeaseService;

/// Creates a state root plus a git repository, resolves the runtime
/// paths exactly as the CLI does, and seeds one active lease nobody's
/// session owns.
fn seed_orphan_lease() -> (tempfile::TempDir, PathBuf, String) {
    let state_dir = tempfile::Builder::new()
        .prefix("bat-lease-cli-")
        .tempdir_in("/tmp")
        .expect("create state dir");
    let repo = state_dir.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let git = Command::new("git")
        .current_dir(&repo)
        .args(["init"])
        .output()
        .expect("git init");
    assert!(git.status.success());

    let paths = RuntimePaths::resolve(state_dir.path(), &repo).expect("resolve runtime paths");
    std::fs::create_dir_all(&paths.root).expect("create runtime root");
    let leases = LeaseService::open(paths.project_id, &paths.root.join("workspace-leases.db"))
        .expect("open lease service");
    let created = leases
        .acquire(RunId::new(), LeaseMode::Write, Some(IsolationKind::Shared))
        .expect("acquire lease");
    leases
        .activate(created.lease_id.clone(), repo.display().to_string())
        .expect("activate lease");

    (state_dir, repo, created.lease_id)
}

#[test]
fn lease_release_frees_an_orphaned_lease_by_id() {
    let (state_dir, repo, lease_id) = seed_orphan_lease();

    let output = Command::new(env!("CARGO_BIN_EXE_batcave"))
        .args([
            "lease",
            "release",
            "--state-dir",
            state_dir.path().to_str().unwrap(),
            "--repo",
            repo.to_str().unwrap(),
            "--lease-id",
            &lease_id,
        ])
        .output()
        .expect("run batcave lease release");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "release must succeed: stdout={stdout} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(&format!("lease {lease_id} released")),
        "must confirm the release: {stdout}"
    );
    // The shared worktree is the repository itself: still on disk, now
    // unmanaged, and the operator is told so.
    assert!(
        stdout.contains("unmanaged"),
        "must report the surviving directory: {stdout}"
    );

    // The claim is genuinely gone: a fresh exclusive acquire succeeds.
    let paths = RuntimePaths::resolve(state_dir.path(), &repo).expect("resolve runtime paths");
    let leases = LeaseService::open(paths.project_id, &paths.root.join("workspace-leases.db"))
        .expect("reopen lease service");
    leases
        .acquire(RunId::new(), LeaseMode::Write, Some(IsolationKind::Shared))
        .expect("the repository must be free after the forced release");

    // Releasing again is refused, not silently repeated.
    let again = Command::new(env!("CARGO_BIN_EXE_batcave"))
        .args([
            "lease",
            "release",
            "--state-dir",
            state_dir.path().to_str().unwrap(),
            "--repo",
            repo.to_str().unwrap(),
            "--lease-id",
            &lease_id,
        ])
        .output()
        .expect("run batcave lease release again");
    assert!(
        !again.status.success(),
        "an already-released lease must exit nonzero"
    );
    assert!(
        String::from_utf8_lossy(&again.stdout).contains("already released"),
        "must name the idempotency refusal"
    );
}

#[test]
fn lease_release_refuses_an_unknown_lease_id() {
    let (state_dir, repo, _lease_id) = seed_orphan_lease();

    let output = Command::new(env!("CARGO_BIN_EXE_batcave"))
        .args([
            "lease",
            "release",
            "--state-dir",
            state_dir.path().to_str().unwrap(),
            "--repo",
            repo.to_str().unwrap(),
            "--lease-id",
            "no-such-lease",
        ])
        .output()
        .expect("run batcave lease release");
    assert!(!output.status.success(), "unknown id must exit nonzero");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no lease no-such-lease exists"),
        "must name the missing lease"
    );
}
