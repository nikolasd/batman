//! The runtime socket server: binds the per-repository Unix domain socket,
//! enforces the same-user peer-credential boundary on every accepted
//! connection before any JSON is parsed, and hands each accepted connection
//! to [`super::connection`].

use std::future::Future;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use batman_protocol::ProjectId;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

use crate::db::DatabaseHandle;
use crate::paths::RuntimePaths;
use crate::security::redaction::{RawEventKind, RawRuntimeEvent, Redactor};

use super::{IpcError, PeerCredentials, ServerConfig};

/// State shared by every connection served by a [`Server`].
pub(crate) struct Shared {
    pub(crate) db: Arc<DatabaseHandle>,
    pub(crate) config: ServerConfig,
    pub(crate) project_id: ProjectId,
    pub(crate) started_at: Instant,
    pub(crate) events_tx: broadcast::Sender<batman_protocol::EventEnvelope>,
}

/// Per-connection context derived from the accepted peer's credentials.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ConnContext {
    /// Whether the runtime verified the peer's OS credentials.
    pub(crate) peer_credentials_verified: bool,
    /// The peer process's pid, if known (used for worker-MCP ancestry).
    pub(crate) peer_pid: Option<i32>,
}

/// The runtime socket server. Bind once, then [`Server::serve`] until a
/// shutdown signal.
pub struct Server {
    listener: UnixListener,
    socket: PathBuf,
    shared: Arc<Shared>,
    owner_only_verified: bool,
}

impl Server {
    /// Binds the runtime socket at `socket`, removing any stale socket file
    /// left by a previous run, and tightening the socket to mode `0600`.
    ///
    /// # Errors
    /// Returns [`IpcError`] if the socket cannot be bound or secured.
    pub async fn bind(
        socket: PathBuf,
        db: Arc<DatabaseHandle>,
        project_id: ProjectId,
        config: ServerConfig,
    ) -> Result<Self, IpcError> {
        // Remove a stale socket file from a previous run so bind() succeeds.
        match std::fs::remove_file(&socket) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(IpcError::Io(err)),
        }

        let listener = UnixListener::bind(&socket).map_err(|source| IpcError::Bind {
            path: socket.clone(),
            source,
        })?;

        // Tighten the socket file itself to owner-only. The parent directory
        // is already mode 0700 (see RuntimePaths::resolve), but defense in
        // depth: the socket node should not be group/other accessible.
        let mut perms = std::fs::metadata(&socket)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&socket, perms)?;

        let owner_only_verified = config
            .owner_only_override
            .unwrap_or_else(|| check_owner_only(&socket, config.euid));

        let (events_tx, _events_rx) = broadcast::channel(64);

        let shared = Arc::new(Shared {
            db,
            config,
            project_id,
            started_at: Instant::now(),
            events_tx,
        });

        Ok(Self {
            listener,
            socket,
            shared,
            owner_only_verified,
        })
    }

    /// The bound socket path.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    /// Accepts and serves connections until `shutdown` resolves.
    ///
    /// Each accepted connection has its peer credentials read *before* any
    /// bytes are consumed; a connection whose peer uid differs from the
    /// runtime's is dropped immediately, before parsing.
    ///
    /// # Errors
    /// Returns [`IpcError`] only on a fatal accept-loop error; ordinary
    /// per-connection failures are logged and the loop continues.
    pub async fn serve<F>(self, shutdown: F) -> Result<(), IpcError>
    where
        F: Future<Output = ()> + Send,
    {
        let Server {
            listener,
            shared,
            owner_only_verified,
            socket: _,
        } = self;

        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                () = &mut shutdown => break,
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _addr)) => {
                            Self::admit(stream, &shared, owner_only_verified);
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "failed to accept runtime socket connection");
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Applies the same-user peer-credential boundary and, if the connection
    /// is admitted, spawns its handler. Rejected connections are dropped here
    /// -- before a single byte of JSON is read.
    fn admit(stream: UnixStream, shared: &Arc<Shared>, owner_only_verified: bool) {
        let creds: PeerCredentials = shared.config.credential_reader.read(&stream);
        let euid = shared.config.euid;

        let (admitted, peer_verified) = match creds.uid {
            Some(uid) if uid == euid => (true, true),
            Some(uid) => {
                tracing::warn!(
                    peer_uid = uid,
                    expected = euid,
                    "rejecting connection from a different uid before parsing"
                );
                (false, true)
            }
            None => {
                // Peer credentials unavailable: fail closed unless the
                // owner-only directory/socket permission check passed.
                if owner_only_verified {
                    (true, false)
                } else {
                    tracing::warn!(
                        "rejecting connection: peer credentials unavailable and owner-only check failed"
                    );
                    (false, false)
                }
            }
        };

        if !admitted {
            drop(stream);
            return;
        }

        let ctx = ConnContext {
            peer_credentials_verified: peer_verified,
            peer_pid: creds.pid,
        };
        let shared = Arc::clone(shared);
        tokio::spawn(async move {
            super::connection::handle(stream, ctx, shared).await;
        });
    }
}

/// Checks that `socket`'s parent directory is owned by `euid` and accessible
/// only by its owner (no group/other permission bits).
fn check_owner_only(socket: &Path, euid: u32) -> bool {
    let dir = socket.parent().unwrap_or_else(|| Path::new("/"));
    match std::fs::metadata(dir) {
        Ok(meta) => meta.uid() == euid && (meta.mode() & 0o077) == 0,
        Err(_) => false,
    }
}

/// Reads the connected peer's OS credentials via the kernel. Returns whatever
/// the platform can provide; fields the platform cannot report are `None`.
#[cfg(target_os = "macos")]
pub(crate) fn read_system_peer_credentials(stream: &UnixStream) -> PeerCredentials {
    use std::os::fd::{AsRawFd, BorrowedFd};

    use nix::sys::socket::{getsockopt, sockopt};

    let fd = stream.as_raw_fd();
    // SAFETY: `stream` outlives this borrow; the fd is valid for the call.
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    let uid = getsockopt(&borrowed, sockopt::LocalPeerCred)
        .ok()
        .map(|cred| cred.uid());
    let pid = getsockopt(&borrowed, sockopt::LocalPeerPid).ok();
    PeerCredentials { uid, pid }
}

/// Reads the connected peer's OS credentials via the kernel. Returns whatever
/// the platform can provide; fields the platform cannot report are `None`.
#[cfg(target_os = "linux")]
pub(crate) fn read_system_peer_credentials(stream: &UnixStream) -> PeerCredentials {
    use std::os::fd::{AsRawFd, BorrowedFd};

    use nix::sys::socket::{getsockopt, sockopt};

    let fd = stream.as_raw_fd();
    // SAFETY: `stream` outlives this borrow; the fd is valid for the call.
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    match getsockopt(&borrowed, sockopt::PeerCredentials) {
        Ok(cred) => PeerCredentials {
            uid: Some(cred.uid()),
            pid: Some(cred.pid()),
        },
        Err(_) => PeerCredentials::default(),
    }
}

/// Reads the connected peer's OS credentials via the kernel. On platforms
/// without a supported peer-credential mechanism, reports nothing.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn read_system_peer_credentials(_stream: &UnixStream) -> PeerCredentials {
    PeerCredentials::default()
}

/// Runs the runtime in the foreground: resolves the per-repository paths under
/// `state_dir`, opens the durable database, records a `RuntimeStarted` event,
/// binds the socket, and serves connections until `SIGINT`/`SIGTERM`.
///
/// Task 7 will add background/detached operation, single-instance locking,
/// idle-exit, and the `status`/`stop` commands; this foundation entry point is
/// structured so that work can wrap it without reshaping the server.
///
/// # Errors
/// Returns [`IpcError`] if the state cannot be secured, the database cannot be
/// opened, or the socket cannot be bound.
pub async fn serve_foreground(state_dir: &Path, repo: &Path) -> Result<(), IpcError> {
    crate::security::ensure_private_dir(state_dir)?;
    let paths = RuntimePaths::resolve(state_dir, repo)?;

    let db = Arc::new(DatabaseHandle::start(paths.database.clone()).await?);

    // Seed a durable RuntimeStarted event through the redaction boundary so
    // reconnecting clients can replay a non-empty history.
    let redactor = Redactor::new();
    let started = redactor.sanitize(RawRuntimeEvent {
        timestamp: batman_protocol::Timestamp::now(),
        project_id: paths.project_id,
        run_id: None,
        kind: RawEventKind::RuntimeStarted,
    });
    db.append_event(started).await?;

    let server = Server::bind(
        paths.socket.clone(),
        db,
        paths.project_id,
        ServerConfig::default(),
    )
    .await?;

    tracing::info!(socket = %paths.socket.display(), "batcave serving in foreground");
    server.serve(shutdown_signal()).await?;
    tracing::info!("batcave shutting down");

    Ok(())
}

/// Resolves when the process receives `SIGINT` or `SIGTERM`.
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut term = match signal(SignalKind::terminate()) {
        Ok(term) => term,
        Err(err) => {
            tracing::warn!(error = %err, "failed to install SIGTERM handler; only SIGINT will stop the runtime");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
