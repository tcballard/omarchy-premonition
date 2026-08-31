//! Same-UID in-memory proposal owner and single-flight state machine.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use premonition_core::{
    ApplyEngine, ApplyError, CoreError, GenerationContext, SafetyCore, ValidatedProposal,
};
use premonition_executor::{
    AgentExecutor, Cancellation, ExecutorError, Provenance, ReasoningEffort,
};
use premonition_protocol::{
    EmptyParams, ErrorCode, ExecutorEvidence, Operation, ProposalEffort, ProposalParams,
    RecentOutcome, RecentSummary, RepositorySummary, Request, Response, ResultPayload, SafeId,
    ServiceState, SubmitParams, WireError,
};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

const REQUEST_CACHE_LIMIT: usize = 64;
const TERMINAL_LIMIT: usize = 20;

/// In-memory daemon state. No source, input, rationale, or patch is persisted.
pub struct Service {
    core: SafetyCore,
    apply: ApplyEngine,
    executor: Option<Arc<dyn AgentExecutor>>,
    gate: Mutex<()>,
    inner: Arc<Mutex<Inner>>,
}

impl Service {
    /// Constructs the service after mandatory transaction recovery.
    ///
    /// # Errors
    ///
    /// Returns an Apply error if recovery cannot prove an all-pre/all-post
    /// repository state.
    pub fn new(
        core: SafetyCore,
        apply: ApplyEngine,
        executor: Option<Arc<dyn AgentExecutor>>,
    ) -> Result<Arc<Self>, ApplyError> {
        let recovery_required = apply.recover().is_err();
        let state = if recovery_required {
            ServiceState::RecoveryRequired
        } else if executor.is_some() {
            ServiceState::Idle
        } else {
            ServiceState::RuntimeMissing
        };
        Ok(Arc::new(Self {
            core,
            apply,
            executor,
            gate: Mutex::new(()),
            inner: Arc::new(Mutex::new(Inner::new(state))),
        }))
    }

    /// Validates, idempotently dispatches, and caches one strict request.
    pub async fn handle(self: &Arc<Self>, request: Request) -> Response {
        let _gate = self.gate.lock().await;
        let request_id = request.request_id.clone();
        let digest = request_digest(&request);
        {
            let inner = self.inner.lock().await;
            if let Some(cached) = inner
                .requests
                .iter()
                .find(|cached| cached.request_id == request_id)
            {
                return if cached.digest == digest {
                    cached.response.clone()
                } else {
                    failure(request_id, ErrorCode::RequestIdReuse)
                };
            }
        }
        let response = match request.validate() {
            Ok(()) => self.dispatch(request.operation, request_id.clone()).await,
            Err(error) => failure(
                request_id.clone(),
                match error {
                    premonition_protocol::ProtocolError::UnsupportedContract => {
                        ErrorCode::UnsupportedContract
                    }
                    premonition_protocol::ProtocolError::InputTooLarge => ErrorCode::InputTooLarge,
                    _ => ErrorCode::InvalidRequest,
                },
            ),
        };
        if !response_contains_body(&response) {
            let mut inner = self.inner.lock().await;
            if inner.requests.len() == REQUEST_CACHE_LIMIT {
                let _ = inner.requests.pop_front();
            }
            inner.requests.push_back(CachedRequest {
                request_id,
                digest,
                response: response.clone(),
            });
        }
        response
    }

    async fn dispatch(&self, operation: Operation, request_id: SafeId) -> Response {
        if !matches!(
            &operation,
            Operation::Status(EmptyParams {}) | Operation::Health(EmptyParams {})
        ) && self.inner.lock().await.state == ServiceState::RecoveryRequired
        {
            return failure(request_id, ErrorCode::RecoveryRequired);
        }
        match operation {
            Operation::Status(EmptyParams {}) => self.status(request_id).await,
            Operation::Submit(parameters) => self.submit(parameters, request_id).await,
            Operation::ProposalShow(parameters) => {
                self.proposal_body(parameters, request_id, false).await
            }
            Operation::ProposalCopy(parameters) => {
                self.proposal_body(parameters, request_id, true).await
            }
            Operation::ProposalApply(parameters) => self.apply(parameters, request_id).await,
            Operation::ProposalDismiss(parameters) => self.dismiss(parameters, request_id).await,
            Operation::Cancel(EmptyParams {}) => self.cancel(request_id).await,
            Operation::Repositories(EmptyParams {}) => self.repositories(request_id),
            Operation::Health(EmptyParams {}) => self.health(request_id).await,
        }
    }

    async fn submit(&self, parameters: SubmitParams, request_id: SafeId) -> Response {
        let Some(executor) = self.executor.clone() else {
            return failure(request_id, ErrorCode::RuntimeMissing);
        };
        {
            let inner = self.inner.lock().await;
            if matches!(
                inner.state,
                ServiceState::Working | ServiceState::Ready | ServiceState::Applying
            ) {
                return failure(request_id, ErrorCode::Busy);
            }
        }
        let context = match self
            .core
            .begin_investigation(parameters.repository_id.as_str())
        {
            Ok(context) => context,
            Err(error) => return failure(request_id, map_core_error(&error)),
        };
        let cancellation = Cancellation::default();
        {
            let mut inner = self.inner.lock().await;
            inner.state = ServiceState::Working;
            inner.failure = None;
            inner.active = Some(Active::Working {
                correlation_id: parameters.correlation_id.clone(),
                repository_id: parameters.repository_id.clone(),
                cancellation: cancellation.clone(),
            });
        }
        let inner = Arc::clone(&self.inner);
        let core = self.core.clone();
        let correlation_id = parameters.correlation_id.clone();
        let repository_id = parameters.repository_id;
        tokio::spawn(async move {
            let low = attempt(
                executor.as_ref(),
                &core,
                context.clone(),
                &parameters.input,
                ReasoningEffort::Low,
                cancellation.clone(),
            )
            .await;
            let completion = if low.as_ref().is_err_and(retryable_low_failure) {
                attempt(
                    executor.as_ref(),
                    &core,
                    context,
                    &parameters.input,
                    ReasoningEffort::Medium,
                    cancellation,
                )
                .await
            } else {
                low
            };
            finish_job(inner, correlation_id, repository_id, completion).await;
        });
        Response::success(
            request_id,
            ResultPayload::Accepted {
                correlation_id: parameters.correlation_id,
            },
        )
    }

    async fn status(&self, request_id: SafeId) -> Response {
        let inner = self.inner.lock().await;
        let (proposal_id, correlation_id, repository_id, created_unix_ms, patch_bytes, file_count) =
            match inner.active.as_ref() {
                Some(Active::Working {
                    correlation_id,
                    repository_id,
                    ..
                }) => (
                    None,
                    Some(correlation_id.clone()),
                    Some(repository_id.clone()),
                    None,
                    None,
                    None,
                ),
                Some(Active::Ready(proposal)) => (
                    Some(proposal.proposal_id.clone()),
                    Some(proposal.correlation_id.clone()),
                    Some(proposal.repository_id.clone()),
                    Some(proposal.created_unix_ms),
                    u32::try_from(proposal.proposal.patch_bytes()).ok(),
                    u16::try_from(proposal.proposal.file_count()).ok(),
                ),
                None => (None, None, None, None, None, None),
            };
        Response::success(
            request_id,
            ResultPayload::Status {
                state: inner.state,
                proposal_id,
                correlation_id,
                repository_id,
                created_unix_ms,
                patch_bytes,
                file_count,
                failure_code: inner.failure,
                recent: inner.recent.iter().cloned().collect(),
            },
        )
    }

    async fn proposal_body(
        &self,
        parameters: ProposalParams,
        request_id: SafeId,
        copy_only: bool,
    ) -> Response {
        let inner = self.inner.lock().await;
        let Some(Active::Ready(proposal)) = inner.active.as_ref() else {
            return failure(request_id, ErrorCode::UnknownProposal);
        };
        if proposal.proposal_id != parameters.proposal_id {
            return failure(request_id, ErrorCode::UnknownProposal);
        }
        if copy_only {
            Response::success(
                request_id,
                ResultPayload::CopyPayload {
                    proposal_id: proposal.proposal_id.clone(),
                    patch: proposal.proposal.patch().to_owned(),
                },
            )
        } else {
            Response::success(
                request_id,
                ResultPayload::Proposal {
                    proposal_id: proposal.proposal_id.clone(),
                    repository_id: proposal.repository_id.clone(),
                    patch: proposal.proposal.patch().to_owned(),
                    rationale: proposal.rationale.clone(),
                    file_count: u16::try_from(proposal.proposal.file_count()).unwrap_or(u16::MAX),
                    created_unix_ms: proposal.created_unix_ms,
                    executor: proposal.executor.clone(),
                },
            )
        }
    }

    async fn apply(&self, parameters: ProposalParams, request_id: SafeId) -> Response {
        {
            let inner = self.inner.lock().await;
            if inner.terminal.iter().any(|terminal| {
                terminal.proposal_id == parameters.proposal_id
                    && terminal.outcome == TerminalOutcome::Applied
            }) {
                return Response::success(
                    request_id,
                    ResultPayload::Applied {
                        proposal_id: parameters.proposal_id,
                        already_applied: true,
                    },
                );
            }
            if inner.state == ServiceState::Applying {
                return failure(request_id, ErrorCode::ApplyInProgress);
            }
        }
        let proposal = {
            let mut inner = self.inner.lock().await;
            let Some(Active::Ready(proposal)) = inner.active.as_ref() else {
                return failure(request_id, ErrorCode::UnknownProposal);
            };
            if proposal.proposal_id != parameters.proposal_id {
                return failure(request_id, ErrorCode::UnknownProposal);
            }
            let proposal = proposal.clone();
            inner.state = ServiceState::Applying;
            proposal
        };
        let engine = self.apply.clone();
        let proposal_id_text = proposal.proposal_id.as_str().to_owned();
        let validated = proposal.proposal.clone();
        let result =
            tokio::task::spawn_blocking(move || engine.apply(&proposal_id_text, &validated))
                .await
                .map_err(|_| ApplyError::RecoveryRequired)
                .and_then(|result| result);
        let mut inner = self.inner.lock().await;
        inner.active = None;
        match result {
            Ok(_) => {
                inner.state = idle_state(self.executor.is_some());
                inner.failure = None;
                inner.push_recent(&proposal, RecentOutcome::Applied);
                inner.push_terminal(proposal.proposal_id.clone(), TerminalOutcome::Applied);
                Response::success(
                    request_id,
                    ResultPayload::Applied {
                        proposal_id: proposal.proposal_id,
                        already_applied: false,
                    },
                )
            }
            Err(error) => {
                let code = map_apply_error(&error);
                inner.state = if code == ErrorCode::RecoveryRequired {
                    ServiceState::RecoveryRequired
                } else {
                    ServiceState::Error
                };
                inner.failure = Some(code);
                failure(request_id, code)
            }
        }
    }

    async fn dismiss(&self, parameters: ProposalParams, request_id: SafeId) -> Response {
        let mut inner = self.inner.lock().await;
        if inner.terminal.iter().any(|terminal| {
            terminal.proposal_id == parameters.proposal_id
                && terminal.outcome == TerminalOutcome::Dismissed
        }) {
            return Response::success(
                request_id,
                ResultPayload::Dismissed {
                    proposal_id: parameters.proposal_id,
                    already_dismissed: true,
                },
            );
        }
        let Some(Active::Ready(proposal)) = inner.active.take() else {
            return failure(request_id, ErrorCode::UnknownProposal);
        };
        if proposal.proposal_id != parameters.proposal_id {
            inner.active = Some(Active::Ready(proposal));
            return failure(request_id, ErrorCode::UnknownProposal);
        }
        inner.state = idle_state(self.executor.is_some());
        inner.failure = None;
        inner.push_recent(&proposal, RecentOutcome::Dismissed);
        inner.push_terminal(proposal.proposal_id.clone(), TerminalOutcome::Dismissed);
        Response::success(
            request_id,
            ResultPayload::Dismissed {
                proposal_id: proposal.proposal_id,
                already_dismissed: false,
            },
        )
    }

    async fn cancel(&self, request_id: SafeId) -> Response {
        let inner = self.inner.lock().await;
        match inner.active.as_ref() {
            Some(Active::Working { cancellation, .. }) => {
                cancellation.cancel();
                Response::success(
                    request_id,
                    ResultPayload::Cancelled {
                        already_terminal: false,
                    },
                )
            }
            _ if inner.state == ServiceState::Applying => {
                failure(request_id, ErrorCode::ApplyInProgress)
            }
            _ => Response::success(
                request_id,
                ResultPayload::Cancelled {
                    already_terminal: true,
                },
            ),
        }
    }

    fn repositories(&self, request_id: SafeId) -> Response {
        let repositories = self
            .core
            .repository_summaries()
            .into_iter()
            .filter_map(|(id, label)| {
                Some(RepositorySummary {
                    id: SafeId::new(id).ok()?,
                    label,
                })
            })
            .collect();
        Response::success(request_id, ResultPayload::Repositories { repositories })
    }

    async fn health(&self, request_id: SafeId) -> Response {
        let inner = self.inner.lock().await;
        Response::success(
            request_id,
            ResultPayload::Health {
                service_version: env!("CARGO_PKG_VERSION").into(),
                protocol_version: premonition_protocol::CONTRACT_VERSION,
                recovery_required: inner.state == ServiceState::RecoveryRequired,
                executor_available: self.executor.is_some(),
            },
        )
    }
}

#[derive(Clone)]
enum Active {
    Working {
        correlation_id: SafeId,
        repository_id: SafeId,
        cancellation: Cancellation,
    },
    Ready(Box<ProposalRecord>),
}

#[derive(Clone)]
struct ProposalRecord {
    proposal_id: SafeId,
    correlation_id: SafeId,
    repository_id: SafeId,
    created_unix_ms: u64,
    rationale: String,
    executor: ExecutorEvidence,
    proposal: ValidatedProposal,
}

struct Inner {
    state: ServiceState,
    active: Option<Active>,
    failure: Option<ErrorCode>,
    recent: VecDeque<RecentSummary>,
    terminal: VecDeque<TerminalRecord>,
    requests: VecDeque<CachedRequest>,
    proposal_sequence: u64,
}

impl Inner {
    fn new(state: ServiceState) -> Self {
        Self {
            state,
            active: None,
            failure: None,
            recent: VecDeque::new(),
            terminal: VecDeque::new(),
            requests: VecDeque::new(),
            proposal_sequence: 0,
        }
    }

    fn push_recent(&mut self, proposal: &ProposalRecord, outcome: RecentOutcome) {
        if self.recent.len() == premonition_protocol::MAX_RECENT {
            let _ = self.recent.pop_front();
        }
        self.recent.push_back(RecentSummary {
            correlation_id: proposal.correlation_id.clone(),
            repository_id: proposal.repository_id.clone(),
            outcome,
            completed_unix_ms: unix_ms(),
        });
    }

    fn push_terminal(&mut self, proposal_id: SafeId, outcome: TerminalOutcome) {
        if self.terminal.len() == TERMINAL_LIMIT {
            let _ = self.terminal.pop_front();
        }
        self.terminal.push_back(TerminalRecord {
            proposal_id,
            outcome,
        });
    }
}

struct CachedRequest {
    request_id: SafeId,
    digest: [u8; 32],
    response: Response,
}

struct TerminalRecord {
    proposal_id: SafeId,
    outcome: TerminalOutcome,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TerminalOutcome {
    Applied,
    Dismissed,
}

enum JobError {
    Executor(ExecutorError),
    Core(CoreError),
}

async fn finish_job(
    inner: Arc<Mutex<Inner>>,
    correlation_id: SafeId,
    repository_id: SafeId,
    completion: Result<(ValidatedProposal, String, ExecutorEvidence), JobError>,
) {
    let mut inner = inner.lock().await;
    let matches_active = matches!(
        inner.active.as_ref(),
        Some(Active::Working { correlation_id: active, .. }) if active == &correlation_id
    );
    if !matches_active {
        return;
    }
    match completion {
        Ok((proposal, rationale, executor)) => {
            inner.proposal_sequence = inner.proposal_sequence.wrapping_add(1);
            let Ok(proposal_id) =
                SafeId::new(format!("p-{}-{}", unix_ms(), inner.proposal_sequence))
            else {
                inner.state = ServiceState::Error;
                inner.failure = Some(ErrorCode::Internal);
                inner.active = None;
                return;
            };
            inner.state = ServiceState::Ready;
            inner.failure = None;
            inner.active = Some(Active::Ready(Box::new(ProposalRecord {
                proposal_id,
                correlation_id,
                repository_id,
                created_unix_ms: unix_ms(),
                rationale,
                executor,
                proposal,
            })));
        }
        Err(error) => {
            let (state, code, outcome) = match error {
                JobError::Executor(ExecutorError::Cancelled) => (
                    ServiceState::Idle,
                    ErrorCode::Cancelled,
                    RecentOutcome::Cancelled,
                ),
                JobError::Executor(error) => {
                    let code = map_executor_error(error);
                    let state = match code {
                        ErrorCode::RuntimeMissing => ServiceState::RuntimeMissing,
                        ErrorCode::CandidateInvalid => ServiceState::Invalid,
                        _ => ServiceState::Error,
                    };
                    let outcome = if state == ServiceState::Invalid {
                        RecentOutcome::Invalid
                    } else {
                        RecentOutcome::Error
                    };
                    (state, code, outcome)
                }
                JobError::Core(error) => (
                    ServiceState::Invalid,
                    map_core_error(&error),
                    RecentOutcome::Invalid,
                ),
            };
            inner.state = state;
            inner.failure = if state == ServiceState::Idle {
                None
            } else {
                Some(code)
            };
            inner.active = None;
            if inner.recent.len() == premonition_protocol::MAX_RECENT {
                let _ = inner.recent.pop_front();
            }
            inner.recent.push_back(RecentSummary {
                correlation_id,
                repository_id,
                outcome,
                completed_unix_ms: unix_ms(),
            });
        }
    }
}

async fn attempt(
    executor: &dyn AgentExecutor,
    core: &SafetyCore,
    context: GenerationContext,
    input: &str,
    effort: ReasoningEffort,
    cancellation: Cancellation,
) -> Result<(ValidatedProposal, String, ExecutorEvidence), JobError> {
    let candidate = executor
        .investigate(context.repository_root(), input, effort, cancellation)
        .await
        .map_err(JobError::Executor)?;
    let proposal = core
        .validate_candidate(context, candidate.patch().to_owned())
        .map_err(JobError::Core)?;
    let evidence = evidence(executor.provenance(), effort).map_err(JobError::Executor)?;
    Ok((proposal, candidate.rationale().to_owned(), evidence))
}

fn retryable_low_failure(error: &JobError) -> bool {
    matches!(
        error,
        JobError::Executor(ExecutorError::MalformedOutput)
            | JobError::Core(CoreError::Diff(_) | CoreError::ApplyCheck)
    )
}

fn evidence(
    provenance: &Provenance,
    effort: ReasoningEffort,
) -> Result<ExecutorEvidence, ExecutorError> {
    Ok(ExecutorEvidence {
        tool_version: provenance.version.clone(),
        tool_sha256: provenance.sha256.clone(),
        model: SafeId::new(provenance.model.clone()).map_err(|_| ExecutorError::Configuration)?,
        reasoning_effort: match effort {
            ReasoningEffort::Low => ProposalEffort::Low,
            ReasoningEffort::Medium => ProposalEffort::Medium,
        },
    })
}

fn request_digest(request: &Request) -> [u8; 32] {
    let encoded = serde_json::to_vec(request).unwrap_or_default();
    Sha256::digest(encoded).into()
}

fn response_contains_body(response: &Response) -> bool {
    matches!(
        response.result.as_ref(),
        Some(ResultPayload::Proposal { .. } | ResultPayload::CopyPayload { .. })
    )
}

fn failure(request_id: SafeId, code: ErrorCode) -> Response {
    Response::failure(
        request_id,
        WireError {
            code,
            retryable: matches!(
                code,
                ErrorCode::Busy
                    | ErrorCode::RuntimeMissing
                    | ErrorCode::ExecutorTimeout
                    | ErrorCode::Stale
                    | ErrorCode::ServiceUnavailable
            ),
            message: error_message(code).into(),
        },
    )
}

fn error_message(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::UnsupportedContract => "Unsupported local contract.",
        ErrorCode::InvalidRequest => "The request is invalid.",
        ErrorCode::RequestIdReuse => "The request identifier was reused.",
        ErrorCode::ServiceUnavailable => "Premonition is unavailable.",
        ErrorCode::RuntimeMissing => "The configured agent runtime is unavailable.",
        ErrorCode::Busy => "Premonition already owns an active job or proposal.",
        ErrorCode::UnknownRepository => "The repository is not allowlisted.",
        ErrorCode::RepositoryUnsafe => "The repository is not in a safe state.",
        ErrorCode::InputTooLarge => "The submitted text exceeds its limit.",
        ErrorCode::SnapshotTooLarge => "The repository exceeds snapshot limits.",
        ErrorCode::ExecutorTimeout => "The agent exceeded its time limit.",
        ErrorCode::Cancelled => "The agent was cancelled.",
        ErrorCode::CandidateInvalid => "The candidate patch is invalid.",
        ErrorCode::Stale => "The repository changed; regenerate the proposal.",
        ErrorCode::ApplyCheckFailed => "Git rejected the candidate patch.",
        ErrorCode::ApplyInProgress => "Apply is already publishing files.",
        ErrorCode::RecoveryRequired => "Apply recovery must complete first.",
        ErrorCode::UnknownProposal => "The proposal is no longer active.",
        ErrorCode::ClipboardUnavailable => "The clipboard runtime is unavailable.",
        ErrorCode::Internal => "Premonition failed safely.",
    }
}

fn map_executor_error(error: ExecutorError) -> ErrorCode {
    match error {
        ExecutorError::Configuration | ExecutorError::Spawn => ErrorCode::RuntimeMissing,
        ExecutorError::Timeout => ErrorCode::ExecutorTimeout,
        ExecutorError::Cancelled => ErrorCode::Cancelled,
        ExecutorError::Input => ErrorCode::InputTooLarge,
        ExecutorError::MalformedOutput | ExecutorError::OutputLimit => ErrorCode::CandidateInvalid,
        ExecutorError::Repository => ErrorCode::RepositoryUnsafe,
        ExecutorError::Io | ExecutorError::Crash => ErrorCode::Internal,
    }
}

fn map_core_error(error: &CoreError) -> ErrorCode {
    match error {
        CoreError::Repository(premonition_core::RepositoryError::Unknown) => {
            ErrorCode::UnknownRepository
        }
        CoreError::Snapshot(premonition_core::SnapshotError::TooLarge) => {
            ErrorCode::SnapshotTooLarge
        }
        CoreError::Snapshot(premonition_core::SnapshotError::Stale) => ErrorCode::Stale,
        CoreError::Diff(_) => ErrorCode::CandidateInvalid,
        CoreError::ApplyCheck => ErrorCode::ApplyCheckFailed,
        CoreError::Repository(_) | CoreError::Snapshot(_) => ErrorCode::RepositoryUnsafe,
    }
}

fn map_apply_error(error: &ApplyError) -> ErrorCode {
    match error {
        ApplyError::Snapshot(premonition_core::SnapshotError::Stale) => ErrorCode::Stale,
        ApplyError::Git(_) => ErrorCode::ApplyCheckFailed,
        ApplyError::RecoveryRequired | ApplyError::State => ErrorCode::RecoveryRequired,
        ApplyError::Repository(_) | ApplyError::Snapshot(_) => ErrorCode::RepositoryUnsafe,
        ApplyError::Identifier | ApplyError::TransactionExists => ErrorCode::Internal,
    }
}

fn idle_state(executor_available: bool) -> ServiceState {
    if executor_available {
        ServiceState::Idle
    } else {
        ServiceState::RuntimeMissing
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}
