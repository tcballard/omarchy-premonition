#![allow(clippy::expect_used, clippy::panic)]

use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use premonition_core::{ApplyEngine, SafetyCore};
use premonition_daemon::Service;
use premonition_executor::{AgentExecutor, Cancellation, Candidate, ExecutorError, Provenance};
use premonition_protocol::{
    CONTRACT_VERSION, EmptyParams, ErrorCode, Operation, ProposalParams, Request, Response,
    ResultPayload, SafeId, ServiceState, SubmitParams,
};
use tempfile::TempDir;

const PATCH: &str = "diff --git a/value.txt b/value.txt\n--- a/value.txt\n+++ b/value.txt\n@@ -1 +1 @@\n-old\n+new\n";

struct Fixture {
    _temporary: TempDir,
    repository: PathBuf,
    service: Arc<Service>,
}

impl Fixture {
    fn new(delay: Duration) -> Self {
        let temporary = tempfile::tempdir().expect("tempdir");
        let repository = temporary.path().join("repo");
        fs::create_dir(&repository).expect("repository");
        git(&repository, &["init", "-q"]);
        git(&repository, &["config", "user.name", "Fixture"]);
        git(
            &repository,
            &["config", "user.email", "fixture@example.invalid"],
        );
        fs::write(repository.join("value.txt"), "old\n").expect("fixture");
        git(&repository, &["add", "--", "value.txt"]);
        git(&repository, &["commit", "-q", "-m", "fixture"]);
        let config = temporary.path().join("config.toml");
        fs::write(
            &config,
            format!(
                "version = 1\ngit_binary = \"/usr/bin/git\"\n\n[[repositories]]\nid = \"fixture\"\nlabel = \"Fixture\"\npath = \"{}\"\n",
                repository.display()
            ),
        )
        .expect("config");
        let core = SafetyCore::load(&config).expect("core");
        let apply = ApplyEngine::new(&temporary.path().join("state")).expect("apply");
        let executor: Arc<dyn AgentExecutor> = Arc::new(FakeExecutor {
            delay,
            provenance: Provenance {
                version: "fake 1".into(),
                sha256: "0".repeat(64),
            },
        });
        let service = Service::new(core, apply, Some(executor)).expect("service");
        Self {
            _temporary: temporary,
            repository,
            service,
        }
    }
}

struct FakeExecutor {
    delay: Duration,
    provenance: Provenance,
}

impl AgentExecutor for FakeExecutor {
    fn investigate<'a>(
        &'a self,
        _repository: &'a Path,
        _observed_error: &'a str,
        cancellation: Cancellation,
    ) -> Pin<Box<dyn Future<Output = Result<Candidate, ExecutorError>> + Send + 'a>> {
        Box::pin(async move {
            tokio::select! {
                () = tokio::time::sleep(self.delay) => Candidate::new(PATCH.into(), "Fix the value.".into()),
                () = wait_cancel(cancellation) => Err(ExecutorError::Cancelled),
            }
        })
    }

    fn provenance(&self) -> &Provenance {
        &self.provenance
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn full_workflow_is_single_flight_explicit_and_idempotent() {
    let fixture = Fixture::new(Duration::from_millis(30));
    let accepted = fixture
        .service
        .handle(request(
            "submit-1",
            Operation::Submit(SubmitParams {
                repository_id: id("fixture"),
                correlation_id: id("corr-1"),
                input: "observed error".into(),
            }),
        ))
        .await;
    assert!(matches!(
        accepted.result,
        Some(ResultPayload::Accepted { .. })
    ));
    assert_eq!(
        fs::read_to_string(fixture.repository.join("value.txt")).expect("preimage"),
        "old\n"
    );

    let busy = fixture
        .service
        .handle(request(
            "submit-2",
            Operation::Submit(SubmitParams {
                repository_id: id("fixture"),
                correlation_id: id("corr-2"),
                input: "other error".into(),
            }),
        ))
        .await;
    assert_eq!(busy.error.expect("busy error").code, ErrorCode::Busy);

    let status = wait_for_state(&fixture.service, ServiceState::Ready).await;
    let proposal_id = match status.result.as_ref().expect("status") {
        ResultPayload::Status {
            proposal_id: Some(proposal_id),
            ..
        } => proposal_id.clone(),
        _ => unreachable!("ready status has proposal"),
    };
    let status_json = serde_json::to_string(&status).expect("status JSON");
    assert!(!status_json.contains("Fix the value"));
    assert!(!status_json.contains("diff --git"));

    let shown = fixture
        .service
        .handle(request(
            "show-1",
            Operation::ProposalShow(ProposalParams {
                proposal_id: proposal_id.clone(),
            }),
        ))
        .await;
    assert!(matches!(shown.result, Some(ResultPayload::Proposal { .. })));

    let applied = fixture
        .service
        .handle(request(
            "apply-1",
            Operation::ProposalApply(ProposalParams {
                proposal_id: proposal_id.clone(),
            }),
        ))
        .await;
    assert!(matches!(
        applied.result,
        Some(ResultPayload::Applied {
            already_applied: false,
            ..
        })
    ));
    assert_eq!(
        fs::read_to_string(fixture.repository.join("value.txt")).expect("postimage"),
        "new\n"
    );
    assert!(git_output(&fixture.repository, &["diff", "--cached", "--name-only"]).is_empty());

    let replay = fixture
        .service
        .handle(request(
            "apply-2",
            Operation::ProposalApply(ProposalParams { proposal_id }),
        ))
        .await;
    assert!(matches!(
        replay.result,
        Some(ResultPayload::Applied {
            already_applied: true,
            ..
        })
    ));

    let exact = fixture
        .service
        .handle(request("health-same", Operation::Health(EmptyParams {})))
        .await;
    let exact_replay = fixture
        .service
        .handle(request("health-same", Operation::Health(EmptyParams {})))
        .await;
    assert_eq!(exact, exact_replay);
    let reused = fixture
        .service
        .handle(request("health-same", Operation::Status(EmptyParams {})))
        .await;
    assert_eq!(
        reused.error.expect("reuse error").code,
        ErrorCode::RequestIdReuse
    );
}

#[tokio::test]
async fn cancellation_is_truthful_and_terminal() {
    let fixture = Fixture::new(Duration::from_secs(30));
    let _accepted = fixture
        .service
        .handle(request(
            "submit-cancel",
            Operation::Submit(SubmitParams {
                repository_id: id("fixture"),
                correlation_id: id("corr-cancel"),
                input: "observed error".into(),
            }),
        ))
        .await;
    let cancelled = fixture
        .service
        .handle(request("cancel-1", Operation::Cancel(EmptyParams {})))
        .await;
    assert!(matches!(
        cancelled.result,
        Some(ResultPayload::Cancelled {
            already_terminal: false
        })
    ));
    let status = wait_for_state(&fixture.service, ServiceState::Idle).await;
    assert!(matches!(status.result, Some(ResultPayload::Status { .. })));
    assert_eq!(
        fs::read_to_string(fixture.repository.join("value.txt")).expect("unchanged"),
        "old\n"
    );
}

async fn wait_for_state(service: &Arc<Service>, expected: ServiceState) -> Response {
    for sequence in 0..100 {
        let response = service
            .handle(request(
                &format!("poll-{sequence}"),
                Operation::Status(EmptyParams {}),
            ))
            .await;
        if matches!(
            response.result,
            Some(ResultPayload::Status { state, .. }) if state == expected
        ) {
            return response;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    unreachable!("state transition timed out")
}

async fn wait_cancel(cancellation: Cancellation) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn request(request_id: &str, operation: Operation) -> Request {
    Request {
        contract_version: CONTRACT_VERSION,
        request_id: id(request_id),
        operation,
    }
}

fn id(value: &str) -> SafeId {
    SafeId::new(value).expect("valid fixture ID")
}

fn git(repository: &Path, arguments: &[&str]) {
    let status = Command::new("/usr/bin/git")
        .current_dir(repository)
        .args(arguments)
        .status()
        .expect("git");
    assert!(status.success());
}

fn git_output(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
        .current_dir(repository)
        .args(arguments)
        .output()
        .expect("git");
    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("utf8")
}
