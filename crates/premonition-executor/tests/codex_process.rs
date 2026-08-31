#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use premonition_executor::{AgentExecutor, Cancellation, CodexCliExecutor, ExecutorError};
use tempfile::TempDir;

struct Fixture {
    _temporary: TempDir,
    repository: PathBuf,
    schema: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().expect("tempdir");
        let repository = temporary.path().join("repo");
        fs::create_dir(&repository).expect("repository");
        let schema = temporary.path().join("schema.json");
        fs::write(&schema, "{\"type\":\"object\"}\n").expect("schema");
        Self {
            _temporary: temporary,
            repository,
            schema,
        }
    }

    async fn executor(&self, timeout: Duration) -> CodexCliExecutor {
        CodexCliExecutor::new(fake_agent(), &self.schema, "gpt-5.6-sol", timeout)
            .await
            .expect("executor")
    }
}

#[tokio::test]
async fn success_is_structured_and_provenanced() {
    let fixture = Fixture::new();
    let executor = fixture.executor(Duration::from_secs(2)).await;
    let candidate = executor
        .investigate(
            &fixture.repository,
            "ordinary error",
            premonition_executor::ReasoningEffort::Low,
            Cancellation::default(),
        )
        .await
        .expect("candidate");
    assert!(candidate.patch().starts_with("diff --git"));
    assert_eq!(candidate.rationale(), "Replace the incorrect value.");
    assert_eq!(
        executor.provenance().version,
        "premonition-fake-agent 1.0.0"
    );
    assert_eq!(executor.provenance().sha256.len(), 64);
    assert_eq!(executor.provenance().model, "gpt-5.6-sol");
}

#[tokio::test]
async fn medium_effort_is_explicitly_forwarded() {
    let fixture = Fixture::new();
    let executor = fixture.executor(Duration::from_secs(2)).await;
    let candidate = executor
        .investigate(
            &fixture.repository,
            "SCENARIO_REQUIRE_MEDIUM",
            premonition_executor::ReasoningEffort::Medium,
            Cancellation::default(),
        )
        .await
        .expect("medium candidate");
    assert!(candidate.patch().starts_with("diff --git"));
}

#[tokio::test]
async fn malformed_hostile_crash_and_overflow_fail_closed() {
    let fixture = Fixture::new();
    let executor = fixture.executor(Duration::from_secs(2)).await;
    for (scenario, expected) in [
        ("SCENARIO_MALFORMED", ExecutorError::MalformedOutput),
        ("SCENARIO_HOSTILE", ExecutorError::MalformedOutput),
        ("SCENARIO_CRASH", ExecutorError::Crash),
        ("SCENARIO_OVERFLOW", ExecutorError::OutputLimit),
    ] {
        let result = executor
            .investigate(
                &fixture.repository,
                scenario,
                premonition_executor::ReasoningEffort::Low,
                Cancellation::default(),
            )
            .await;
        assert_eq!(result, Err(expected));
    }
}

#[tokio::test]
async fn timeout_kills_the_process_group() {
    let fixture = Fixture::new();
    let executor = fixture.executor(Duration::from_millis(50)).await;
    let result = executor
        .investigate(
            &fixture.repository,
            "SCENARIO_TIMEOUT",
            premonition_executor::ReasoningEffort::Low,
            Cancellation::default(),
        )
        .await;
    assert_eq!(result, Err(ExecutorError::Timeout));
}

#[tokio::test]
async fn timeout_kills_descendants_before_they_can_write() {
    let fixture = Fixture::new();
    let sentinel = fixture.repository.join("descendant-survived");
    let executor = fixture.executor(Duration::from_millis(50)).await;
    let result = executor
        .investigate(
            &fixture.repository,
            "SCENARIO_DESCENDANT",
            premonition_executor::ReasoningEffort::Low,
            Cancellation::default(),
        )
        .await;
    assert_eq!(result, Err(ExecutorError::Timeout));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!sentinel.exists());
}

#[tokio::test]
async fn cancellation_kills_the_process_group() {
    let fixture = Fixture::new();
    let executor = fixture.executor(Duration::from_secs(2)).await;
    let cancellation = Cancellation::default();
    let trigger = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        trigger.cancel();
    });
    let result = executor
        .investigate(
            &fixture.repository,
            "SCENARIO_CANCEL",
            premonition_executor::ReasoningEffort::Low,
            cancellation,
        )
        .await;
    assert_eq!(result, Err(ExecutorError::Cancelled));
}

fn fake_agent() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_premonition-fake-agent"))
}
