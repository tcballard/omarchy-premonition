//! Strict JSON client for the owner-only service and explicit clipboard actions.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand};
use premonition_protocol::{
    CONTRACT_VERSION, EmptyParams, ErrorCode, Operation, ProposalParams, Request, Response,
    ResultPayload, SafeId, SubmitParams, WireError, decode_frame, encode_frame,
};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::Command;

#[derive(Parser)]
#[command(version, about = "Controlled error-to-candidate-patch bridge")]
struct Cli {
    /// Owner-only daemon socket. Defaults to `PREMONITION_SOCKET`.
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    /// Emit stable v1 JSON. Every v0.1 command is JSON-only.
    #[arg(long = "json", global = true)]
    _json: bool,
    #[command(subcommand)]
    command: TopCommand,
}

#[derive(Subcommand)]
enum TopCommand {
    /// Return content-free status.
    Status,
    /// Explicitly submit stdin or the current clipboard.
    Submit(SubmitArguments),
    /// Review or act on one proposal.
    Proposal {
        #[command(subcommand)]
        command: ProposalCommand,
    },
    /// Cancel the running investigation.
    Cancel,
    /// List allowlisted repository choices.
    Repositories,
    /// Return daemon/runtime health.
    Health,
}

#[derive(Args)]
struct SubmitArguments {
    /// Allowlisted repository ID, never a path.
    #[arg(long)]
    repo: String,
    /// Read observed text from stdin.
    #[arg(
        long,
        conflicts_with_all = ["clipboard", "selection"],
        required_unless_present_any = ["clipboard", "selection"]
    )]
    stdin: bool,
    /// Explicitly read the current clipboard once.
    #[arg(
        long,
        conflicts_with_all = ["stdin", "selection"],
        required_unless_present_any = ["stdin", "selection"]
    )]
    clipboard: bool,
    /// Explicitly read the current primary selection once.
    #[arg(
        long,
        conflicts_with_all = ["stdin", "clipboard"],
        required_unless_present_any = ["stdin", "clipboard"]
    )]
    selection: bool,
    /// Optional caller correlation ID.
    #[arg(long)]
    correlation_id: Option<String>,
    /// Absolute wl-paste-compatible binary.
    #[arg(long, default_value = "/usr/bin/wl-paste")]
    clipboard_reader: PathBuf,
}

#[derive(Subcommand)]
enum ProposalCommand {
    /// Return the bounded proposal body for review.
    Show(ProposalId),
    /// Explicitly revalidate and apply the proposal.
    Apply(ProposalId),
    /// Dismiss the proposal without mutation.
    Dismiss(ProposalId),
    /// Explicitly copy the patch without printing it.
    Copy(CopyArguments),
}

#[derive(Args)]
struct ProposalId {
    /// Active proposal identifier.
    id: String,
}

#[derive(Args)]
struct CopyArguments {
    /// Active proposal identifier.
    id: String,
    /// Absolute wl-copy-compatible binary.
    #[arg(long, default_value = "/usr/bin/wl-copy")]
    clipboard_writer: PathBuf,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let arguments = Cli::parse();
    let request_id = generated_id("r").unwrap_or_else(|_| fallback_id());
    let socket = arguments
        .socket
        .or_else(|| std::env::var_os("PREMONITION_SOCKET").map(PathBuf::from))
        .or_else(default_socket);
    let response = match socket {
        Some(socket) => execute(&socket, request_id.clone(), arguments.command)
            .await
            .unwrap_or_else(|error| local_failure(request_id, error)),
        None => local_failure(request_id, ClientError::Service),
    };
    let ok = response.ok;
    match serde_json::to_string(&response) {
        Ok(json) => println!("{json}"),
        Err(_) => return std::process::ExitCode::FAILURE,
    }
    if ok {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

async fn execute(
    socket: &Path,
    request_id: SafeId,
    command: TopCommand,
) -> Result<Response, ClientError> {
    let (operation, copy_writer) = match command {
        TopCommand::Status => (Operation::Status(EmptyParams {}), None),
        TopCommand::Cancel => (Operation::Cancel(EmptyParams {}), None),
        TopCommand::Repositories => (Operation::Repositories(EmptyParams {}), None),
        TopCommand::Health => (Operation::Health(EmptyParams {}), None),
        TopCommand::Submit(arguments) => {
            let input = if arguments.clipboard || arguments.selection {
                read_clipboard(&arguments.clipboard_reader, arguments.selection).await?
            } else {
                read_stdin()?
            };
            let correlation_id = arguments
                .correlation_id
                .map(SafeId::new)
                .transpose()?
                .map_or_else(|| generated_id("c"), Ok)?;
            (
                Operation::Submit(SubmitParams {
                    repository_id: SafeId::new(arguments.repo)?,
                    correlation_id,
                    input,
                }),
                None,
            )
        }
        TopCommand::Proposal { command } => match command {
            ProposalCommand::Show(arguments) => (
                Operation::ProposalShow(proposal_parameters(arguments.id)?),
                None,
            ),
            ProposalCommand::Apply(arguments) => (
                Operation::ProposalApply(proposal_parameters(arguments.id)?),
                None,
            ),
            ProposalCommand::Dismiss(arguments) => (
                Operation::ProposalDismiss(proposal_parameters(arguments.id)?),
                None,
            ),
            ProposalCommand::Copy(arguments) => (
                Operation::ProposalCopy(proposal_parameters(arguments.id)?),
                Some(arguments.clipboard_writer),
            ),
        },
    };
    let request = Request {
        contract_version: CONTRACT_VERSION,
        request_id: request_id.clone(),
        operation,
    };
    request.validate()?;
    let response = send(socket, &request).await?;
    response.validate()?;
    if let Some(writer) = copy_writer {
        match response.result {
            Some(ResultPayload::CopyPayload { proposal_id, patch }) if response.ok => {
                write_clipboard(&writer, patch.as_bytes()).await?;
                return Ok(Response::success(
                    request_id,
                    ResultPayload::Copied { proposal_id },
                ));
            }
            _ => return Ok(response),
        }
    }
    Ok(response)
}

async fn send(socket: &Path, request: &Request) -> Result<Response, ClientError> {
    if !socket.is_absolute() {
        return Err(ClientError::Service);
    }
    let mut stream = tokio::time::timeout(Duration::from_secs(2), UnixStream::connect(socket))
        .await
        .map_err(|_| ClientError::Service)?
        .map_err(|_| ClientError::Service)?;
    let frame = encode_frame(request)?;
    tokio::time::timeout(Duration::from_secs(5), stream.write_all(&frame))
        .await
        .map_err(|_| ClientError::Service)?
        .map_err(|_| ClientError::Service)?;
    let mut prefix = [0_u8; 4];
    tokio::time::timeout(Duration::from_secs(300), stream.read_exact(&mut prefix))
        .await
        .map_err(|_| ClientError::Service)?
        .map_err(|_| ClientError::Service)?;
    let length = usize::try_from(u32::from_be_bytes(prefix)).map_err(|_| ClientError::Protocol)?;
    if length == 0 || length > premonition_protocol::MAX_FRAME_BYTES {
        return Err(ClientError::Protocol);
    }
    let mut frame = Vec::with_capacity(length + 4);
    frame.extend_from_slice(&prefix);
    frame.resize(length + 4, 0);
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut frame[4..]))
        .await
        .map_err(|_| ClientError::Service)?
        .map_err(|_| ClientError::Service)?;
    decode_frame(&frame).map_err(Into::into)
}

fn read_stdin() -> Result<String, ClientError> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(
            u64::try_from(premonition_protocol::MAX_INPUT_BYTES + 1)
                .map_err(|_| ClientError::Input)?,
        )
        .read_to_end(&mut bytes)
        .map_err(|_| ClientError::Input)?;
    if bytes.len() > premonition_protocol::MAX_INPUT_BYTES {
        return Err(ClientError::Input);
    }
    String::from_utf8(bytes).map_err(|_| ClientError::Input)
}

async fn read_clipboard(binary: &Path, primary: bool) -> Result<String, ClientError> {
    let binary = canonical_binary(binary)?;
    let mut command = Command::new(binary);
    command.arg("--no-newline");
    if primary {
        command.arg("--primary");
    }
    let mut child = command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ClientError::Clipboard)?;
    let stdout = child.stdout.take().ok_or(ClientError::Clipboard)?;
    let mut bytes = Vec::new();
    stdout
        .take(
            u64::try_from(premonition_protocol::MAX_INPUT_BYTES + 1)
                .map_err(|_| ClientError::Clipboard)?,
        )
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| ClientError::Clipboard)?;
    let status = tokio::time::timeout(Duration::from_secs(3), child.wait())
        .await
        .map_err(|_| ClientError::Clipboard)?
        .map_err(|_| ClientError::Clipboard)?;
    if !status.success() || bytes.len() > premonition_protocol::MAX_INPUT_BYTES {
        return Err(ClientError::Clipboard);
    }
    String::from_utf8(bytes).map_err(|_| ClientError::Clipboard)
}

async fn write_clipboard(binary: &Path, patch: &[u8]) -> Result<(), ClientError> {
    let binary = canonical_binary(binary)?;
    let mut child = Command::new(binary)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ClientError::Clipboard)?;
    let mut stdin = child.stdin.take().ok_or(ClientError::Clipboard)?;
    stdin
        .write_all(patch)
        .await
        .map_err(|_| ClientError::Clipboard)?;
    drop(stdin);
    let status = tokio::time::timeout(Duration::from_secs(3), child.wait())
        .await
        .map_err(|_| ClientError::Clipboard)?
        .map_err(|_| ClientError::Clipboard)?;
    if status.success() {
        Ok(())
    } else {
        Err(ClientError::Clipboard)
    }
}

fn canonical_binary(path: &Path) -> Result<PathBuf, ClientError> {
    if !path.is_absolute() {
        return Err(ClientError::Clipboard);
    }
    let path = std::fs::canonicalize(path).map_err(|_| ClientError::Clipboard)?;
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
        Ok(path)
    } else {
        Err(ClientError::Clipboard)
    }
}

fn proposal_parameters(id: String) -> Result<ProposalParams, ClientError> {
    Ok(ProposalParams {
        proposal_id: SafeId::new(id)?,
    })
}

fn default_socket() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(PathBuf::from(runtime).join("premonition/premonition.sock"))
}

fn generated_id(prefix: &str) -> Result<SafeId, ClientError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ClientError::Protocol)?
        .as_nanos();
    SafeId::new(format!("{prefix}-{}-{nanos}", std::process::id())).map_err(Into::into)
}

fn fallback_id() -> SafeId {
    SafeId::new("r-fallback").unwrap_or_else(|_| unreachable_safe_id())
}

fn unreachable_safe_id() -> SafeId {
    serde_json::from_str("\"r\"").unwrap_or_else(|_| std::process::exit(70))
}

fn local_failure(request_id: SafeId, error: ClientError) -> Response {
    let code = match error {
        ClientError::Clipboard => ErrorCode::ClipboardUnavailable,
        ClientError::Input => ErrorCode::InputTooLarge,
        ClientError::Protocol => ErrorCode::InvalidRequest,
        ClientError::Service => ErrorCode::ServiceUnavailable,
    };
    Response::failure(
        request_id,
        WireError {
            code,
            retryable: matches!(
                code,
                ErrorCode::ServiceUnavailable | ErrorCode::ClipboardUnavailable
            ),
            message: match code {
                ErrorCode::ClipboardUnavailable => "The clipboard runtime is unavailable.",
                ErrorCode::InputTooLarge => "The submitted text exceeds its limit.",
                ErrorCode::InvalidRequest => "The request is invalid.",
                _ => "Premonition is unavailable.",
            }
            .into(),
        },
    )
}

#[derive(Clone, Copy, Debug, Error)]
enum ClientError {
    #[error("service unavailable")]
    Service,
    #[error("protocol error")]
    Protocol,
    #[error("input error")]
    Input,
    #[error("clipboard error")]
    Clipboard,
}

impl From<premonition_protocol::ProtocolError> for ClientError {
    fn from(error: premonition_protocol::ProtocolError) -> Self {
        match error {
            premonition_protocol::ProtocolError::InputTooLarge => Self::Input,
            _ => Self::Protocol,
        }
    }
}
