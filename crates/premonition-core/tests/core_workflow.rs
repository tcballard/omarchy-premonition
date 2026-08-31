#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;

use premonition_core::{ApplyEngine, CoreError, SafetyCore};
use tempfile::TempDir;

const PATCH: &str = "diff --git a/src.txt b/src.txt\n--- a/src.txt\n+++ b/src.txt\n@@ -1 +1 @@\n-alpha\n+beta\ndiff --git a/old.txt b/old.txt\ndeleted file mode 100644\n--- a/old.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-old\ndiff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+new\n";

struct Fixture {
    _temporary: TempDir,
    repository: PathBuf,
    config: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().expect("tempdir");
        let repository = temporary.path().join("repo");
        fs::create_dir(&repository).expect("repository directory");
        git(&repository, &["init", "-q"]);
        git(&repository, &["config", "user.name", "Fixture"]);
        git(
            &repository,
            &["config", "user.email", "fixture@example.invalid"],
        );
        fs::write(repository.join("src.txt"), "alpha\n").expect("source fixture");
        fs::write(repository.join("old.txt"), "old\n").expect("delete fixture");
        git(&repository, &["add", "--", "src.txt", "old.txt"]);
        git(&repository, &["commit", "-q", "-m", "fixture"]);

        let config = temporary.path().join("config.toml");
        fs::write(
            &config,
            format!(
                "version = 1\ngit_binary = \"/usr/bin/git\"\n\n[[repositories]]\nid = \"fixture\"\nlabel = \"Fixture\"\npath = \"{}\"\n",
                repository.display()
            ),
        )
        .expect("config fixture");
        let state = temporary.path().join("state");
        Self {
            _temporary: temporary,
            repository,
            config,
            state,
        }
    }

    fn core(&self) -> SafetyCore {
        SafetyCore::load(&self.config).expect("load core")
    }
}

#[test]
fn candidate_is_read_only_until_explicit_transactional_apply() {
    let fixture = Fixture::new();
    fs::write(fixture.repository.join("unrelated.txt"), "keep my work\n")
        .expect("pre-existing unrelated edit");
    let core = fixture.core();
    let context = core.begin_investigation("fixture").expect("context");
    let proposal = core
        .validate_candidate(context, PATCH.to_owned())
        .expect("valid proposal");

    assert_eq!(
        fs::read_to_string(fixture.repository.join("src.txt")).unwrap(),
        "alpha\n"
    );
    assert!(fixture.repository.join("old.txt").exists());
    assert!(!fixture.repository.join("new.txt").exists());

    let engine = ApplyEngine::new(&fixture.state).expect("apply engine");
    assert_eq!(engine.recover().expect("empty recovery").recovered, 0);
    let outcome = engine
        .apply("proposal-1", &proposal)
        .expect("explicit apply");
    assert_eq!(outcome.files_changed, 3);
    assert_eq!(
        fs::read_to_string(fixture.repository.join("src.txt")).unwrap(),
        "beta\n"
    );
    assert!(!fixture.repository.join("old.txt").exists());
    assert_eq!(
        fs::read_to_string(fixture.repository.join("new.txt")).unwrap(),
        "new\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.repository.join("unrelated.txt")).unwrap(),
        "keep my work\n"
    );
    assert!(git_output(&fixture.repository, &["diff", "--cached", "--name-only"]).is_empty());
    assert!(!git_output(&fixture.repository, &["status", "--short"]).is_empty());
    assert_eq!(engine.recover().expect("post recovery").recovered, 0);
}

#[test]
fn any_worktree_change_after_generation_fails_closed() {
    let fixture = Fixture::new();
    let core = fixture.core();
    let context = core.begin_investigation("fixture").expect("context");
    fs::write(fixture.repository.join("unrelated.txt"), "dirty\n").expect("late change");
    assert!(matches!(
        core.validate_candidate(context, PATCH.to_owned()),
        Err(CoreError::Snapshot(_))
    ));
    assert_eq!(
        fs::read_to_string(fixture.repository.join("src.txt")).unwrap(),
        "alpha\n"
    );
}

#[test]
fn symlink_target_and_symlinked_config_are_rejected() {
    let fixture = Fixture::new();
    let real_config = fixture.config.with_extension("real");
    fs::rename(&fixture.config, &real_config).expect("move config");
    symlink(&real_config, &fixture.config).expect("config symlink");
    assert!(SafetyCore::load(&fixture.config).is_err());

    fs::remove_file(&fixture.config).expect("remove config link");
    fs::rename(&real_config, &fixture.config).expect("restore config");
    let core = fixture.core();
    let context = core.begin_investigation("fixture").expect("context");
    fs::remove_file(fixture.repository.join("src.txt")).expect("remove source");
    symlink("old.txt", fixture.repository.join("src.txt")).expect("target symlink");
    assert!(core.validate_candidate(context, PATCH.to_owned()).is_err());
}

#[test]
fn state_directory_must_be_private() {
    let fixture = Fixture::new();
    fs::create_dir(&fixture.state).expect("state directory");
    fs::set_permissions(&fixture.state, fs::Permissions::from_mode(0o755)).expect("permissions");
    assert!(ApplyEngine::new(&fixture.state).is_err());
}

fn git(repository: &Path, arguments: &[&str]) {
    let status = Command::new("/usr/bin/git")
        .current_dir(repository)
        .args(arguments)
        .status()
        .expect("run git");
    assert!(status.success());
}

fn git_output(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
        .current_dir(repository)
        .args(arguments)
        .output()
        .expect("run git");
    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("utf8 git output")
}
