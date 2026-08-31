//! Strict, size-bounded local wire contract shared by `premonitiond` and the
//! `premonition` CLI.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

/// Current local protocol contract version.
pub const CONTRACT_VERSION: u16 = 1;
/// Maximum accepted encoded frame, excluding the four-byte length prefix.
pub const MAX_FRAME_BYTES: usize = 384 * 1024;
/// Maximum submitted observed-error text.
pub const MAX_INPUT_BYTES: usize = 32 * 1024;
/// Maximum patch body returned by an explicit proposal operation.
pub const MAX_PATCH_BYTES: usize = 256 * 1024;
/// Maximum rationale body returned by an explicit proposal operation.
pub const MAX_RATIONALE_BYTES: usize = 8 * 1024;
/// Maximum content-free recent-history entries in status.
pub const MAX_RECENT: usize = 20;
/// Maximum configured repository summaries returned to the UI.
pub const MAX_REPOSITORIES: usize = 64;

/// A validated opaque identifier safe to expose in status and argv.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SafeId(String);

impl SafeId {
    /// Constructs an identifier containing 1–64 ASCII alphanumeric, dot,
    /// underscore, or hyphen characters.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidIdentifier`] when the value is empty,
    /// oversized, non-ASCII, or contains a character outside the safe set.
    pub fn new(value: impl Into<String>) -> Result<Self, ProtocolError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(ProtocolError::InvalidIdentifier);
        }
        Ok(Self(value))
    }

    /// Returns the validated identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SafeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("SafeId").field(&self.0).finish()
    }
}

impl<'de> Deserialize<'de> for SafeId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// One complete request sent over the owner-only Unix socket.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Contract version; must be [`CONTRACT_VERSION`].
    pub contract_version: u16,
    /// Idempotency key for this exact payload.
    pub request_id: SafeId,
    /// Requested operation and its strictly typed parameters.
    pub operation: Operation,
}

impl Request {
    /// Checks semantic bounds not expressible through Serde derives.
    ///
    /// # Errors
    ///
    /// Returns a stable [`ProtocolError`] for unsupported versions or an
    /// empty, oversized, or NUL-bearing submission.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.contract_version != CONTRACT_VERSION {
            return Err(ProtocolError::UnsupportedContract);
        }
        if let Operation::Submit(SubmitParams { input, .. }) = &self.operation {
            if input.is_empty() {
                return Err(ProtocolError::EmptyInput);
            }
            if input.len() > MAX_INPUT_BYTES {
                return Err(ProtocolError::InputTooLarge);
            }
            reject_nul(input)?;
        }
        Ok(())
    }
}

/// Supported v1 operations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Return content-free service status.
    Status(EmptyParams),
    /// Admit explicit observed-error text for one repository.
    Submit(SubmitParams),
    /// Return an active proposal body for explicit review.
    ProposalShow(ProposalParams),
    /// Revalidate and explicitly apply an active proposal.
    ProposalApply(ProposalParams),
    /// Dismiss an active proposal without mutation.
    ProposalDismiss(ProposalParams),
    /// Return a patch only to the CLI's explicit clipboard-copy path.
    ProposalCopy(ProposalParams),
    /// Cancel the currently running executor job.
    Cancel(EmptyParams),
    /// List configured safe repository identifiers and labels.
    Repositories(EmptyParams),
    /// Return service/protocol health without proposal content.
    Health(EmptyParams),
}

/// Strict empty parameter object used by parameter-free operations.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}

/// Strict explicit-submission parameters.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitParams {
    /// Public allowlist identifier, never a path.
    pub repository_id: SafeId,
    /// Correlates the executor job and resulting proposal.
    pub correlation_id: SafeId,
    /// Sensitive observed-error text; never persisted or logged.
    pub input: String,
}

/// Strict parameter object for one active proposal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalParams {
    /// Opaque active proposal identifier.
    pub proposal_id: SafeId,
}

/// One complete daemon response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    /// Contract version used to encode this response.
    pub contract_version: u16,
    /// Echoed request identifier.
    pub request_id: SafeId,
    /// True exactly when `result` is present and `error` is absent.
    pub ok: bool,
    /// Typed success payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ResultPayload>,
    /// Stable, content-free failure payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

impl Response {
    /// Creates a success envelope.
    #[must_use]
    pub fn success(request_id: SafeId, result: ResultPayload) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            request_id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Creates a failure envelope.
    #[must_use]
    pub fn failure(request_id: SafeId, error: WireError) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            request_id,
            ok: false,
            result: None,
            error: Some(error),
        }
    }

    /// Checks envelope consistency and every response body ceiling.
    ///
    /// # Errors
    ///
    /// Returns a stable [`ProtocolError`] for an inconsistent envelope,
    /// unsupported version, hostile display text, or oversized body.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.contract_version != CONTRACT_VERSION {
            return Err(ProtocolError::UnsupportedContract);
        }
        match (self.ok, self.result.as_ref(), self.error.as_ref()) {
            (true, Some(result), None) => validate_result(result),
            (false, None, Some(error)) => validate_error(error),
            _ => Err(ProtocolError::InvalidEnvelope),
        }
    }
}

/// Typed successful response payloads.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResultPayload {
    /// Content-free current state used by bar/panel polling.
    Status {
        /// Truthful state machine state.
        state: ServiceState,
        /// Active proposal, when ready/applying.
        #[serde(skip_serializing_if = "Option::is_none")]
        proposal_id: Option<SafeId>,
        /// Active request/job correlation.
        #[serde(skip_serializing_if = "Option::is_none")]
        correlation_id: Option<SafeId>,
        /// Safe allowlist identifier, never a path.
        #[serde(skip_serializing_if = "Option::is_none")]
        repository_id: Option<SafeId>,
        /// Proposal creation time.
        #[serde(skip_serializing_if = "Option::is_none")]
        created_unix_ms: Option<u64>,
        /// Patch size only, never its content.
        #[serde(skip_serializing_if = "Option::is_none")]
        patch_bytes: Option<u32>,
        /// Number of changed files.
        #[serde(skip_serializing_if = "Option::is_none")]
        file_count: Option<u16>,
        /// Stable failure classification for invalid/error states.
        #[serde(skip_serializing_if = "Option::is_none")]
        failure_code: Option<ErrorCode>,
        /// Bounded content-free recent outcomes.
        recent: Vec<RecentSummary>,
    },
    /// Submission was accepted into the single-flight state machine.
    Accepted {
        /// Correlation ID assigned to the active job.
        correlation_id: SafeId,
    },
    /// Sensitive proposal body returned only after explicit review.
    Proposal {
        /// Opaque proposal identifier.
        proposal_id: SafeId,
        /// Safe repository identifier.
        repository_id: SafeId,
        /// Validated unified diff.
        patch: String,
        /// Concise agent rationale.
        rationale: String,
        /// Number of changed files.
        file_count: u16,
        /// Creation time.
        created_unix_ms: u64,
    },
    /// Sensitive patch handoff consumed internally by the CLI copy command.
    CopyPayload {
        /// Opaque proposal identifier.
        proposal_id: SafeId,
        /// Validated patch body.
        patch: String,
    },
    /// Explicit Apply completed or was already completed idempotently.
    Applied {
        /// Opaque proposal identifier.
        proposal_id: SafeId,
        /// True when the original successful result was replayed.
        already_applied: bool,
    },
    /// Explicit Dismiss completed or was replayed.
    Dismissed {
        /// Opaque proposal identifier.
        proposal_id: SafeId,
        /// True when no additional state transition was required.
        already_dismissed: bool,
    },
    /// Cancellation won, or the job was already terminal.
    Cancelled {
        /// True when no running job remained.
        already_terminal: bool,
    },
    /// CLI copied a proposal patch without exposing it on stdout.
    Copied {
        /// Opaque proposal identifier.
        proposal_id: SafeId,
    },
    /// Configured safe repository choices.
    Repositories {
        /// Bounded repository summaries.
        repositories: Vec<RepositorySummary>,
    },
    /// Content-free service health.
    Health {
        /// Service semantic version.
        service_version: String,
        /// Active protocol version.
        protocol_version: u16,
        /// Whether recovery blocks ordinary operations.
        recovery_required: bool,
        /// Whether the configured executor can be resolved and probed.
        executor_available: bool,
    },
}

/// Truthful public service states.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    /// Ready for an explicit submission.
    Idle,
    /// One executor job is running.
    Working,
    /// A validated proposal awaits review.
    Ready,
    /// Candidate output failed deterministic validation.
    Invalid,
    /// A non-candidate operational failure occurred.
    Error,
    /// No configured executor runtime is available.
    RuntimeMissing,
    /// Explicit Apply is publishing or recovering.
    Applying,
    /// Durable recovery must complete before serving.
    RecoveryRequired,
}

/// Content-free completed-event summary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecentSummary {
    /// Request/job correlation.
    pub correlation_id: SafeId,
    /// Safe repository identifier.
    pub repository_id: SafeId,
    /// Stable terminal classification.
    pub outcome: RecentOutcome,
    /// Terminal event time.
    pub completed_unix_ms: u64,
}

/// Content-free terminal outcomes retained in bounded memory.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecentOutcome {
    /// Proposal was explicitly applied.
    Applied,
    /// Proposal was explicitly dismissed.
    Dismissed,
    /// Running execution was cancelled.
    Cancelled,
    /// Candidate was deterministically invalid.
    Invalid,
    /// Executor or service failed.
    Error,
    /// Proposal expired without mutation.
    Expired,
}

/// Safe repository picker entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositorySummary {
    /// Allowlist identifier used by submit.
    pub id: SafeId,
    /// Bounded display label containing no path.
    pub label: String,
}

/// Stable content-free protocol error.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WireError {
    /// Machine-stable error code.
    pub code: ErrorCode,
    /// Whether retrying after an external state change can succeed.
    pub retryable: bool,
    /// Fixed, bounded, non-content-bearing message.
    pub message: String,
}

/// Stable v1 error codes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Wire version is not supported.
    UnsupportedContract,
    /// Request fields or operation are invalid.
    InvalidRequest,
    /// Reused request ID does not match its original payload.
    RequestIdReuse,
    /// User service/socket is unavailable.
    ServiceUnavailable,
    /// Executor binary cannot be resolved or verified.
    RuntimeMissing,
    /// Another job or proposal owns the single-flight slot.
    Busy,
    /// Repository ID is absent from the allowlist.
    UnknownRepository,
    /// Repository/path/operation state is unsafe.
    RepositoryUnsafe,
    /// Explicit input exceeded its ceiling.
    InputTooLarge,
    /// Repository snapshot exceeded a bound.
    SnapshotTooLarge,
    /// Executor exceeded its deadline.
    ExecutorTimeout,
    /// Candidate was cancelled.
    Cancelled,
    /// Executor output or patch was invalid.
    CandidateInvalid,
    /// Repository changed after proposal generation.
    Stale,
    /// Final applicability check failed.
    ApplyCheckFailed,
    /// Apply is in its non-cancellable publication phase.
    ApplyInProgress,
    /// Crash recovery must be resolved first.
    RecoveryRequired,
    /// Proposal ID is unknown or no longer active.
    UnknownProposal,
    /// Clipboard reader/writer runtime is unavailable.
    ClipboardUnavailable,
    /// Internal failure with details deliberately redacted.
    Internal,
}

/// Protocol/framing validation failures.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ProtocolError {
    /// Unsupported contract version.
    #[error("unsupported contract")]
    UnsupportedContract,
    /// Identifier syntax or size is invalid.
    #[error("invalid identifier")]
    InvalidIdentifier,
    /// Submit input is empty.
    #[error("input is empty")]
    EmptyInput,
    /// Submit input exceeds its ceiling.
    #[error("input is too large")]
    InputTooLarge,
    /// NUL bytes are never accepted in text fields.
    #[error("text contains a forbidden NUL")]
    ForbiddenNul,
    /// Exactly one of result/error was not present.
    #[error("invalid response envelope")]
    InvalidEnvelope,
    /// A response body exceeded its defined bound.
    #[error("response body is out of bounds")]
    ResponseOutOfBounds,
    /// Frame is incomplete, oversized, or has trailing bytes.
    #[error("invalid frame")]
    InvalidFrame,
    /// JSON cannot be decoded under the strict schema.
    #[error("invalid JSON")]
    InvalidJson,
}

/// Encodes one value as a four-byte big-endian length followed by strict JSON.
///
/// # Errors
///
/// Returns [`ProtocolError::InvalidJson`] when serialization fails and
/// [`ProtocolError::InvalidFrame`] when the encoded value exceeds the ceiling.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    let json = serde_json::to_vec(value).map_err(|_| ProtocolError::InvalidJson)?;
    if json.is_empty() || json.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::InvalidFrame);
    }
    let length = u32::try_from(json.len()).map_err(|_| ProtocolError::InvalidFrame)?;
    let mut frame = Vec::with_capacity(json.len() + 4);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&json);
    Ok(frame)
}

/// Decodes exactly one complete four-byte-length-prefixed JSON frame.
///
/// # Errors
///
/// Returns [`ProtocolError::InvalidFrame`] for incomplete, oversized, or
/// trailing data and [`ProtocolError::InvalidJson`] for schema-invalid JSON.
pub fn decode_frame<T>(frame: &[u8]) -> Result<T, ProtocolError>
where
    T: for<'de> Deserialize<'de>,
{
    if frame.len() < 5 {
        return Err(ProtocolError::InvalidFrame);
    }
    let prefix: [u8; 4] = frame[..4]
        .try_into()
        .map_err(|_| ProtocolError::InvalidFrame)?;
    let length =
        usize::try_from(u32::from_be_bytes(prefix)).map_err(|_| ProtocolError::InvalidFrame)?;
    if length == 0 || length > MAX_FRAME_BYTES || frame.len() != length + 4 {
        return Err(ProtocolError::InvalidFrame);
    }
    serde_json::from_slice(&frame[4..]).map_err(|_| ProtocolError::InvalidJson)
}

fn validate_result(result: &ResultPayload) -> Result<(), ProtocolError> {
    match result {
        ResultPayload::Status { recent, .. } if recent.len() > MAX_RECENT => {
            Err(ProtocolError::ResponseOutOfBounds)
        }
        ResultPayload::Proposal {
            patch, rationale, ..
        } => {
            validate_patch(patch)?;
            validate_rationale(rationale)
        }
        ResultPayload::CopyPayload { patch, .. } => validate_patch(patch),
        ResultPayload::Repositories { repositories }
            if repositories.len() > MAX_REPOSITORIES
                || repositories.iter().any(|repository| {
                    repository.label.is_empty()
                        || repository.label.len() > 80
                        || has_hostile_controls(&repository.label)
                }) =>
        {
            Err(ProtocolError::ResponseOutOfBounds)
        }
        ResultPayload::Health {
            service_version, ..
        } if service_version.is_empty()
            || service_version.len() > 32
            || has_hostile_controls(service_version) =>
        {
            Err(ProtocolError::ResponseOutOfBounds)
        }
        _ => Ok(()),
    }
}

fn validate_error(error: &WireError) -> Result<(), ProtocolError> {
    if error.message.is_empty() || error.message.len() > 256 || has_hostile_controls(&error.message)
    {
        return Err(ProtocolError::ResponseOutOfBounds);
    }
    Ok(())
}

fn validate_patch(patch: &str) -> Result<(), ProtocolError> {
    if patch.is_empty() || patch.len() > MAX_PATCH_BYTES {
        return Err(ProtocolError::ResponseOutOfBounds);
    }
    reject_nul(patch)
}

fn validate_rationale(rationale: &str) -> Result<(), ProtocolError> {
    if rationale.is_empty()
        || rationale.len() > MAX_RATIONALE_BYTES
        || has_hostile_controls(rationale)
    {
        return Err(ProtocolError::ResponseOutOfBounds);
    }
    Ok(())
}

fn reject_nul(value: &str) -> Result<(), ProtocolError> {
    if value.as_bytes().contains(&0) {
        Err(ProtocolError::ForbiddenNul)
    } else {
        Ok(())
    }
}

fn has_hostile_controls(value: &str) -> bool {
    value.chars().any(|character| {
        (character.is_control() && !matches!(character, '\n' | '\t'))
            || matches!(
                character,
                '\u{202a}'
                    ..='\u{202e}' | '\u{2066}'
                    ..='\u{2069}' | '\u{061c}' | '\u{200e}' | '\u{200f}'
            )
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    fn id(value: &str) -> SafeId {
        SafeId::new(value).expect("fixture id is valid")
    }

    fn status() -> Response {
        Response::success(
            id("req-1"),
            ResultPayload::Status {
                state: ServiceState::Idle,
                proposal_id: None,
                correlation_id: None,
                repository_id: None,
                created_unix_ms: None,
                patch_bytes: None,
                file_count: None,
                failure_code: None,
                recent: Vec::new(),
            },
        )
    }

    #[test]
    fn safe_ids_reject_unsafe_or_oversized_values() {
        assert!(SafeId::new("repo.one-2_ok").is_ok());
        assert!(SafeId::new("").is_err());
        assert!(SafeId::new("../repo").is_err());
        assert!(SafeId::new("a".repeat(65)).is_err());
    }

    #[test]
    fn strict_request_rejects_unknown_and_duplicate_fields() {
        let unknown =
            br#"{"contract_version":1,"request_id":"r","operation":{"status":{}},"extra":1}"#;
        let duplicate = br#"{"contract_version":1,"contract_version":1,"request_id":"r","operation":{"status":{}}}"#;
        assert!(serde_json::from_slice::<Request>(unknown).is_err());
        assert!(serde_json::from_slice::<Request>(duplicate).is_err());
    }

    #[test]
    fn strict_operation_rejects_unknown_parameters() {
        let value =
            br#"{"contract_version":1,"request_id":"r","operation":{"status":{"patch":"secret"}}}"#;
        assert!(serde_json::from_slice::<Request>(value).is_err());
    }

    #[test]
    fn request_validation_enforces_version_and_input_bound() {
        let mut request = Request {
            contract_version: 2,
            request_id: id("r"),
            operation: Operation::Status(EmptyParams {}),
        };
        assert_eq!(request.validate(), Err(ProtocolError::UnsupportedContract));
        request.contract_version = CONTRACT_VERSION;
        request.operation = Operation::Submit(SubmitParams {
            repository_id: id("repo"),
            correlation_id: id("corr"),
            input: "x".repeat(MAX_INPUT_BYTES + 1),
        });
        assert_eq!(request.validate(), Err(ProtocolError::InputTooLarge));
    }

    #[test]
    fn response_requires_exactly_one_result_or_error() {
        let mut response = status();
        assert!(response.validate().is_ok());
        response.ok = false;
        assert_eq!(response.validate(), Err(ProtocolError::InvalidEnvelope));
    }

    #[test]
    fn status_json_contains_no_body_fields() {
        let json = serde_json::to_string(&status()).expect("status serializes");
        assert!(!json.contains("patch"));
        assert!(!json.contains("rationale"));
        assert!(!json.contains("source"));
        assert!(!json.contains("path"));
    }

    #[test]
    fn explicit_proposal_bodies_are_bounded() {
        let response = Response::success(
            id("r"),
            ResultPayload::Proposal {
                proposal_id: id("p"),
                repository_id: id("repo"),
                patch: "x".repeat(MAX_PATCH_BYTES + 1),
                rationale: "because".into(),
                file_count: 1,
                created_unix_ms: 1,
            },
        );
        assert_eq!(response.validate(), Err(ProtocolError::ResponseOutOfBounds));
    }

    #[test]
    fn display_text_rejects_terminal_and_bidi_controls() {
        for hostile in ["bad\u{1b}[31m", "right\u{202e}left"] {
            let response = Response::failure(
                id("r"),
                WireError {
                    code: ErrorCode::Internal,
                    retryable: false,
                    message: hostile.into(),
                },
            );
            assert_eq!(response.validate(), Err(ProtocolError::ResponseOutOfBounds));
        }
    }

    #[test]
    fn frame_round_trip_is_exact() {
        let expected = status();
        let frame = encode_frame(&expected).expect("frame encodes");
        let actual: Response = decode_frame(&frame).expect("frame decodes");
        assert_eq!(actual, expected);
    }

    #[test]
    fn frame_rejects_short_trailing_and_oversized_lengths() {
        assert_eq!(
            decode_frame::<Response>(&[0, 0, 0, 1]),
            Err(ProtocolError::InvalidFrame)
        );
        let mut frame = encode_frame(&status()).expect("frame encodes");
        frame.push(b'x');
        assert_eq!(
            decode_frame::<Response>(&frame),
            Err(ProtocolError::InvalidFrame)
        );
        let length = u32::try_from(MAX_FRAME_BYTES + 1)
            .expect("frame bound fits u32")
            .to_be_bytes();
        let mut oversized = length.to_vec();
        oversized.push(b'{');
        assert_eq!(
            decode_frame::<Response>(&oversized),
            Err(ProtocolError::InvalidFrame)
        );
    }

    #[test]
    fn recent_collection_is_bounded() {
        let recent = vec![
            RecentSummary {
                correlation_id: id("c"),
                repository_id: id("r"),
                outcome: RecentOutcome::Applied,
                completed_unix_ms: 1,
            };
            MAX_RECENT + 1
        ];
        let response = Response::success(
            id("request"),
            ResultPayload::Status {
                state: ServiceState::Idle,
                proposal_id: None,
                correlation_id: None,
                repository_id: None,
                created_unix_ms: None,
                patch_bytes: None,
                file_count: None,
                failure_code: None,
                recent,
            },
        );
        assert_eq!(response.validate(), Err(ProtocolError::ResponseOutOfBounds));
    }
}
