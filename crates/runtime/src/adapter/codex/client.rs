//! A correlated JSON-RPC client over the Codex `app-server`'s stdio.
//!
//! Every request the adapter sends carries a locally assigned integer id;
//! a background driver task owns the [`ManagedProcess`] exclusively (its
//! `write_stdin`/`next_stdout_frame` both need `&mut self`, so no other
//! caller may hold it directly) and either resolves a pending request's
//! `oneshot` by id, or forwards an unsolicited server notification/request
//! to the adapter through [`InboundMessage`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::supervisor::{ManagedProcess, TerminationOutcome};

/// A message from the Codex app-server transport: an unsolicited
/// notification, a server -> client request (e.g. an approval), or the
/// driver loop's own lifecycle report when the supervised process exits.
#[derive(Debug, Clone)]
pub enum InboundMessage {
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// The supervised `codex app-server` process has exited and the driver
    /// loop is finished. Not a vendor frame: this is the transport's own
    /// lifecycle report, emitted exactly once, always last.
    ProcessExited {
        exit_code: Option<i32>,
        signal: Option<String>,
    },
}

/// A command sent to the driver loop over `outbound_tx`.
enum DriverCommand {
    Write(Vec<u8>),
    Terminate(oneshot::Sender<TerminationOutcome>),
}

/// An error from the correlated JSON-RPC transport itself (distinct from
/// an `AdapterError`, which is this crate's public error boundary --
/// `CodexAdapter` wraps every [`ClientError`] into one).
#[derive(Debug, Clone, thiserror::Error)]
pub enum ClientError {
    #[error("codex app-server closed its stdout before responding")]
    Closed,
    #[error("codex app-server sent a malformed JSON-RPC frame: {0}")]
    Malformed(String),
    #[error("codex app-server returned a JSON-RPC error: {0}")]
    RpcError(Value),
    #[error("the correlated response channel was dropped")]
    ResponseDropped,
}

type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, Value>>>>>;

/// A correlated JSON-RPC client over one supervised `codex app-server`
/// process's stdio.
pub struct CodexRpcClient {
    outbound_tx: mpsc::UnboundedSender<DriverCommand>,
    pending: PendingMap,
    next_id: AtomicI64,
    driver: JoinHandle<()>,
}

impl CodexRpcClient {
    /// Takes ownership of `process`'s stdio and starts the background
    /// driver loop. Returns the client and the [`InboundMessage`] receiver
    /// the adapter must keep draining (unsolicited notifications/requests
    /// are dropped once this channel is closed, never buffered
    /// unboundedly forever -- the adapter's own event pump is what keeps
    /// this alive for the life of the run).
    #[must_use]
    pub fn spawn(process: ManagedProcess) -> (Self, mpsc::UnboundedReceiver<InboundMessage>) {
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

        let driver = tokio::spawn(Self::driver_loop(
            process,
            outbound_rx,
            inbound_tx,
            Arc::clone(&pending),
        ));

        (
            Self {
                outbound_tx,
                pending,
                next_id: AtomicI64::new(1),
                driver,
            },
            inbound_rx,
        )
    }

    async fn driver_loop(
        mut process: ManagedProcess,
        mut outbound_rx: mpsc::UnboundedReceiver<DriverCommand>,
        inbound_tx: mpsc::UnboundedSender<InboundMessage>,
        pending: PendingMap,
    ) {
        let outcome = loop {
            tokio::select! {
                frame = process.next_stdout_frame() => {
                    match frame {
                        Some(bytes) => Self::dispatch_frame(&bytes, &inbound_tx, &pending),
                        None => break process.settle().await,
                    }
                }
                outbound = outbound_rx.recv() => {
                    match outbound {
                        Some(DriverCommand::Write(bytes)) => {
                            if process.write_stdin(&bytes).await.is_err() {
                                break process.settle().await;
                            }
                        }
                        Some(DriverCommand::Terminate(reply)) => {
                            let outcome = process.terminate().await;
                            let _ = reply.send(outcome);
                            break outcome;
                        }
                        // Every `CodexRpcClient` (the only outbound_tx
                        // owner) was dropped; nothing more to write.
                        None => break process.settle().await,
                    }
                }
            }
        };
        let (exit_code, signal) = outcome.exit_signals();
        let _ = inbound_tx.send(InboundMessage::ProcessExited { exit_code, signal });
        // The process (and its stdio) is dropped here, closing the pipes;
        // any request still awaiting a response never resolves, matching
        // `ClientError::ResponseDropped`'s Sender-dropped semantics.
    }

    fn dispatch_frame(
        bytes: &[u8],
        inbound_tx: &mpsc::UnboundedSender<InboundMessage>,
        pending: &PendingMap,
    ) {
        let Ok(msg) = serde_json::from_slice::<Value>(bytes) else {
            return;
        };
        let has_id = msg.get("id").is_some();
        let method = msg.get("method").and_then(Value::as_str);

        if has_id && method.is_none() {
            // A response to one of our own requests.
            let Some(id) = msg.get("id").and_then(Value::as_i64) else {
                return;
            };
            let sender = pending
                .lock()
                .expect("pending mutex never poisoned")
                .remove(&id);
            let Some(sender) = sender else { return };
            if let Some(error) = msg.get("error") {
                let _ = sender.send(Err(error.clone()));
            } else {
                let _ = sender.send(Ok(msg.get("result").cloned().unwrap_or(Value::Null)));
            }
            return;
        }

        let Some(method) = method else { return };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let inbound = if has_id {
            InboundMessage::Request {
                id: msg.get("id").cloned().unwrap_or(Value::Null),
                method: method.to_string(),
                params,
            }
        } else {
            InboundMessage::Notification {
                method: method.to_string(),
                params,
            }
        };
        // The receiver may already be gone if the adapter was disposed;
        // dropping the message is correct in that case.
        let _ = inbound_tx.send(inbound);
    }

    /// Sends a correlated request and awaits its response.
    ///
    /// # Errors
    /// Returns [`ClientError`] if the transport closed before a response
    /// arrived, or the server replied with a JSON-RPC error object.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending mutex never poisoned")
            .insert(id, tx);

        let frame =
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send_frame(&frame)?;

        match rx.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) => Err(ClientError::RpcError(error)),
            Err(_) => Err(ClientError::ResponseDropped),
        }
    }

    /// Sends a fire-and-forget notification (no `id`, no response
    /// expected) -- used for the `initialized` handshake notification.
    ///
    /// # Errors
    /// Returns [`ClientError::Closed`] if the transport already closed.
    pub fn notify(&self, method: &str, params: Value) -> Result<(), ClientError> {
        let frame = serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.send_frame(&frame)
    }

    /// Sends a response to a server -> client request (e.g. resolving an
    /// approval), echoing the request's own `id`.
    ///
    /// # Errors
    /// Returns [`ClientError::Closed`] if the transport already closed.
    pub fn respond(&self, id: Value, result: Value) -> Result<(), ClientError> {
        let frame = serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result});
        self.send_frame(&frame)
    }

    fn send_frame(&self, frame: &Value) -> Result<(), ClientError> {
        let mut bytes =
            serde_json::to_vec(frame).map_err(|e| ClientError::Malformed(e.to_string()))?;
        bytes.push(b'\n');
        self.outbound_tx
            .send(DriverCommand::Write(bytes))
            .map_err(|_| ClientError::Closed)
    }

    /// Gracefully terminates the supervised `codex app-server` process
    /// (escalating SIGINT -> SIGTERM -> SIGKILL, per
    /// [`crate::supervisor::ManagedProcess::terminate`]) and stops the
    /// driver loop.
    ///
    /// # Errors
    /// Returns [`ClientError::Closed`] if the driver had already exited
    /// (e.g. the process exited on its own beforehand).
    pub async fn terminate(&self) -> Result<TerminationOutcome, ClientError> {
        let (tx, rx) = oneshot::channel();
        self.outbound_tx
            .send(DriverCommand::Terminate(tx))
            .map_err(|_| ClientError::Closed)?;
        rx.await.map_err(|_| ClientError::Closed)
    }

    /// Aborts the background driver task outright, without attempting a
    /// graceful process shutdown. Idempotent-safe to call more than once
    /// (subsequent calls are a no-op abort on an already-finished task).
    pub fn shutdown(&self) {
        self.driver.abort();
    }
}
