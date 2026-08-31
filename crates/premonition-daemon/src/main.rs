//! Owner-only Unix-socket host for the Premonition state machine.

use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use premonition_core::{ApplyEngine, SafetyCore};
use premonition_daemon::Service;
use premonition_executor::{AgentExecutor, CodexCliExecutor};
use premonition_protocol::{MAX_FRAME_BYTES, Request, Response, decode_frame, encode_frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

#[derive(Parser)]
#[command(version, about = "Premonition owner-only proposal service")]
struct Arguments {
    /// Strict repository allowlist TOML.
    #[arg(long)]
    config: PathBuf,
    /// Owner-only Unix socket.
    #[arg(long)]
    socket: PathBuf,
    /// Private Apply transaction directory.
    #[arg(long)]
    state_dir: PathBuf,
    /// Absolute Codex CLI executable.
    #[arg(long, default_value = "/opt/codex/bin/codex")]
    codex: PathBuf,
    /// Explicit Codex model identifier recorded in proposal provenance.
    #[arg(long, default_value = "gpt-5.6-sol")]
    model: String,
    /// Strict final-response JSON schema.
    #[arg(long)]
    output_schema: PathBuf,
    /// Maximum agent execution time.
    #[arg(long, default_value_t = 120)]
    timeout_seconds: u64,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run(Arguments::parse()).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("premonitiond: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run(arguments: Arguments) -> Result<(), &'static str> {
    let core =
        SafetyCore::load(&arguments.config).map_err(|_| "repository configuration rejected")?;
    let apply = ApplyEngine::new(&arguments.state_dir).map_err(|_| "private state rejected")?;
    let executor: Option<Arc<dyn AgentExecutor>> = CodexCliExecutor::new(
        &arguments.codex,
        &arguments.output_schema,
        &arguments.model,
        Duration::from_secs(arguments.timeout_seconds),
    )
    .await
    .ok()
    .map(|executor| Arc::new(executor) as Arc<dyn AgentExecutor>);
    let service =
        Service::new(core, apply, executor).map_err(|_| "recovery initialization failed")?;
    let listener = bind_owner_socket(&arguments.socket).await?;

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(|_| "socket accept failed")?;
                let service = Arc::clone(&service);
                tokio::spawn(async move {
                    let _ = serve_one(stream, service).await;
                });
            }
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(|_| "signal handling failed")?;
                break;
            }
        }
    }
    drop(listener);
    if socket_is_owned(&arguments.socket) {
        fs::remove_file(&arguments.socket).map_err(|_| "socket cleanup failed")?;
    }
    Ok(())
}

async fn bind_owner_socket(path: &Path) -> Result<UnixListener, &'static str> {
    if !path.is_absolute() {
        return Err("socket path is not absolute");
    }
    let parent = path.parent().ok_or("socket parent is missing")?;
    let existed = parent.exists();
    fs::create_dir_all(parent).map_err(|_| "socket parent creation failed")?;
    if !existed {
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|_| "socket parent permissions failed")?;
    }
    let parent_metadata =
        fs::symlink_metadata(parent).map_err(|_| "socket parent metadata failed")?;
    if parent_metadata.file_type().is_symlink()
        || !parent_metadata.is_dir()
        || parent_metadata.uid() != current_uid()
        || parent_metadata.permissions().mode() & 0o077 != 0
    {
        return Err("socket parent is unsafe");
    }
    if fs::symlink_metadata(path).is_ok() {
        if !socket_is_owned(path) || UnixStream::connect(path).await.is_ok() {
            return Err("socket path is occupied");
        }
        fs::remove_file(path).map_err(|_| "stale socket removal failed")?;
    }
    let listener = UnixListener::bind(path).map_err(|_| "socket bind syscall failed")?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| "socket permissions failed")?;
    Ok(listener)
}

async fn serve_one(mut stream: UnixStream, service: Arc<Service>) -> Result<(), ()> {
    let credentials = stream.peer_cred().map_err(|_| ())?;
    if credentials.uid() != current_uid() {
        return Err(());
    }
    let mut prefix = [0_u8; 4];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut prefix))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
    let length = usize::try_from(u32::from_be_bytes(prefix)).map_err(|_| ())?;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(());
    }
    let mut frame = Vec::with_capacity(length + 4);
    frame.extend_from_slice(&prefix);
    frame.resize(length + 4, 0);
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut frame[4..]))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
    let request: Request = decode_frame(&frame).map_err(|_| ())?;
    let response: Response = service.handle(request).await;
    response.validate().map_err(|_| ())?;
    let response_frame = encode_frame(&response).map_err(|_| ())?;
    tokio::time::timeout(Duration::from_secs(5), stream.write_all(&response_frame))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
    stream.shutdown().await.map_err(|_| ())
}

fn socket_is_owned(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_socket() && metadata.uid() == current_uid())
}

fn current_uid() -> u32 {
    fs::metadata("/proc/self").map_or(u32::MAX, |metadata| metadata.uid())
}
