//! Replayable operations, acquisition reservations and joined cancellation.

use super::handler::{CallbackHandler, WaitingCallback};
use crate::RequestHeadersExt;
use crate::server::{ClientSession, RequestHeaders};
use crate::{Error, Result};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use tinymcp_bus::{
    HEADER_PROTOCOL_VERSION, HEADER_SESSION_ID, ServerHostReply, ServerHttpResponseMetadata,
    ServerHttpSessionConfig, ServerHttpSessionMethod, ServerHttpSessionRequest, ServerInput,
    ServerOperationSnapshot, ServerOperationState, ServerSessionConfig,
};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

struct Operation {
    input: ServerInput,
    headers: RequestHeaders,
    task: Option<JoinHandle<std::result::Result<Option<String>, ()>>>,
    receiver: mpsc::Receiver<WaitingCallback>,
    waiting: Option<WaitingCallback>,
    terminal: Option<ServerOperationState>,
    reply_bytes: usize,
    http_initialization: bool,
    http_response: Option<ServerHttpResponseMetadata>,
}
impl Drop for Operation {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
impl Operation {
    async fn finish(&mut self) {
        if self.task.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(task) = self.task.take()
        {
            self.terminal = Some(match task.await {
                Ok(Err(())) => ServerOperationState::Failed {
                    message: "server output exceeds byte limit".into(),
                },
                Ok(Ok(response)) => ServerOperationState::Complete { response },
                Err(_) => ServerOperationState::Failed {
                    message: "server operation stopped".into(),
                },
            });
        }
    }
    async fn stop(&mut self) {
        self.waiting = None;
        self.receiver.close();
        while self.receiver.try_recv().is_ok() {}
        if let Some(task) = self.task.as_mut() {
            task.abort();
            let _ = task.await;
        }
        self.task = None;
    }
}
struct Session {
    client: Arc<Mutex<ClientSession>>,
    config: ServerSessionConfig,
    http_open_context: Option<HttpSessionContext>,
    http: Option<HttpSessionContext>,
    operation: Option<Operation>,
    operation_sequence: u128,
}

#[derive(Clone, PartialEq, Eq)]
struct HttpSessionContext {
    requested_session_id: Option<String>,
    known_session: bool,
    expected_protocol_version: Option<String>,
}

const ADMISSION_WINDOW: u128 = 4096;

#[derive(Default)]
struct Registry {
    active: HashMap<String, Session>,
    reserved: HashSet<u128>,
    sequence: u128,
}
impl Registry {
    fn reserve(&mut self, sequence: u128) -> Result<()> {
        if sequence <= self.sequence.saturating_sub(ADMISSION_WINDOW)
            || self.reserved.contains(&sequence)
        {
            return Err(invalid(
                "server session identifier already reserved or expired",
            ));
        }
        self.sequence = self.sequence.max(sequence);
        let floor = self.sequence.saturating_sub(ADMISSION_WINDOW);
        self.reserved.retain(|id| *id > floor);
        self.reserved.insert(sequence);
        Ok(())
    }

    fn reserve_close(&mut self, sequence: u128) {
        let floor = self.sequence.saturating_sub(ADMISSION_WINDOW);
        let ceiling = self.sequence.saturating_add(ADMISSION_WINDOW);
        if sequence > floor && sequence <= ceiling && !self.reserved.contains(&sequence) {
            self.reserved.insert(sequence);
        }
    }
}

/// One registry object's bounded active sessions and rolling admission window.
#[derive(Default)]
pub struct ServerSessions {
    sessions: Mutex<Registry>,
}
impl std::fmt::Debug for ServerSessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerSessions").finish_non_exhaustive()
    }
}
fn invalid(message: &str) -> Error {
    Error::invalid_argument(message)
}
fn http_error(status_code: u16, message: &str) -> ServerHttpResponseMetadata {
    ServerHttpResponseMetadata {
        status_code,
        headers: BTreeMap::new(),
        content_type: Some("text/plain; charset=utf-8".into()),
        error_body: Some(message.into()),
    }
}
fn validate_id(id: &str) -> Result<uuid::Uuid> {
    let id = uuid::Uuid::parse_str(id).map_err(|_| invalid("invalid server identifier"))?;
    if id.is_nil() {
        return Err(invalid("invalid server identifier"));
    }
    Ok(id)
}
fn bounded<T: serde::Serialize>(value: &T) -> Result<usize> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| invalid("invalid server payload"))?
        .len();
    if bytes > super::MAX_BYTES {
        return Err(invalid("server payload exceeds byte limit"));
    }
    Ok(bytes)
}
fn is_initialize_request(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|value| {
            value
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .is_some_and(|method| method == "initialize")
}
fn header_value<'a>(headers: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_str())
}
fn http_admission_error(
    context: &HttpSessionContext,
    session_header: Option<&str>,
    protocol_header: Option<&str>,
) -> Option<ServerHttpResponseMetadata> {
    let (status_code, message) = match context.requested_session_id.as_deref() {
        None => (400, "missing or invalid Mcp-Session-Id header"),
        Some(_) if !context.known_session => (404, "unknown or expired MCP session"),
        Some(session_id) if session_header != Some(session_id) => {
            (404, "unknown or expired MCP session")
        }
        Some(_) if protocol_header != context.expected_protocol_version.as_deref() => {
            (400, "missing or invalid MCP-Protocol-Version header")
        }
        Some(_) => return None,
    };
    Some(ServerHttpResponseMetadata {
        status_code,
        headers: BTreeMap::default(),
        content_type: Some("text/plain; charset=utf-8".into()),
        error_body: Some(message.into()),
    })
}
fn http_response_metadata(
    state: &ServerOperationState,
    initialization: bool,
    client: &tinymcp_bus::ServerSessionMetadata,
) -> (ServerHttpResponseMetadata, Option<HttpSessionContext>) {
    let (status_code, error_body, success_content_type) = match state {
        ServerOperationState::Complete { response: Some(_) } => {
            (200, None, Some("application/json".to_owned()))
        }
        ServerOperationState::Complete { response: None } => (204, None, None),
        ServerOperationState::Cancelled => (503, Some("server operation cancelled".into()), None),
        ServerOperationState::Failed { .. } => (500, Some("internal server error".into()), None),
        ServerOperationState::Pending | ServerOperationState::Callback { .. } | _ => {
            (500, Some("server operation incomplete".into()), None)
        }
    };
    let mut headers = BTreeMap::new();
    let mut next_context = None;
    if initialization
        && client.initialized
        && let Some(protocol_version) = client.protocol_version.as_ref()
        && status_code == 200
    {
        let session_id = uuid::Uuid::new_v4().to_string();
        headers.insert(HEADER_SESSION_ID.to_owned(), session_id.clone());
        next_context = Some(HttpSessionContext {
            requested_session_id: Some(session_id),
            known_session: true,
            expected_protocol_version: Some(protocol_version.clone()),
        });
    }
    let content_type = if error_body.is_some() {
        Some("text/plain; charset=utf-8".to_owned())
    } else {
        success_content_type
    };
    (
        ServerHttpResponseMetadata {
            status_code,
            headers,
            content_type,
            error_body,
        },
        next_context,
    )
}
fn rejected_http_operation(
    input: ServerInput,
    headers: RequestHeaders,
    http_initialization: bool,
    response: ServerHttpResponseMetadata,
) -> Operation {
    let (_sender, receiver) = mpsc::channel(1);
    Operation {
        input,
        headers,
        task: None,
        receiver,
        waiting: None,
        terminal: Some(ServerOperationState::Complete { response: None }),
        reply_bytes: 0,
        http_initialization,
        http_response: Some(response),
    }
}
impl ServerSessions {
    /// Opens or retries a caller-known session reservation with frozen declarations.
    ///
    /// # Errors
    /// Rejects invalid declarations, conflicting IDs, more than 32 active sessions,
    /// oversized payloads or retired/out-of-window reservation identifiers.
    pub async fn open(&self, config: ServerSessionConfig) -> Result<String> {
        self.open_with_transport(config, None).await
    }

    /// Opens a session that applies HTTP session and protocol-header admission.
    ///
    /// # Errors
    /// Rejects inconsistent HTTP session context or the same errors as [`Self::open`].
    pub async fn open_http(&self, config: ServerHttpSessionConfig) -> Result<String> {
        bounded(&config)?;
        if config.known_session
            != (config.requested_session_id.is_some() && config.expected_protocol_version.is_some())
        {
            return Err(invalid("invalid HTTP session context"));
        }
        self.open_with_transport(
            config.server,
            Some(HttpSessionContext {
                requested_session_id: config.requested_session_id,
                known_session: config.known_session,
                expected_protocol_version: config.expected_protocol_version,
            }),
        )
        .await
    }

    async fn open_with_transport(
        &self,
        config: ServerSessionConfig,
        http: Option<HttpSessionContext>,
    ) -> Result<String> {
        bounded(&config)?;
        let _: crate::server::ServerInfo = serde_json::from_value(config.info.clone())
            .map_err(|_| invalid("invalid server identity"))?;
        let _: Vec<crate::server::ResourceSpec> =
            serde_json::from_value(Value::Array(config.resources.clone()))
                .map_err(|_| invalid("invalid server resources"))?;
        if config.source_type_prefix.trim().is_empty() {
            return Err(invalid("empty server provenance prefix"));
        }
        let identifier = validate_id(&config.session_id)?;
        let id = identifier.to_string();
        let mut sessions = self.sessions.lock().await;
        if let Some(existing) = sessions.active.get(&id) {
            return if existing.config == config && existing.http_open_context == http {
                Ok(id)
            } else {
                Err(invalid("server session identifier already reserved"))
            };
        }
        if sessions.active.len() >= 32 {
            return Err(invalid("server session limit reached"));
        }
        sessions.reserve(identifier.as_u128())?;
        sessions.active.insert(
            id.clone(),
            Session {
                client: Arc::new(Mutex::new(ClientSession::new(&config.source_type_prefix))),
                config,
                http_open_context: http.clone(),
                http,
                operation: None,
                operation_sequence: 0,
            },
        );
        Ok(id)
    }
    /// Returns transport metadata captured by the module for an open session.
    ///
    /// # Errors
    /// Rejects an unknown session identifier.
    pub async fn metadata(&self, id: &str) -> Result<tinymcp_bus::ServerSessionMetadata> {
        let sessions = self.sessions.lock().await;
        let session = sessions
            .active
            .get(&validate_id(id)?.to_string())
            .ok_or_else(|| invalid("unknown server session"))?;
        let client = Arc::clone(&session.client);
        drop(sessions);
        Ok(client.lock().await.metadata())
    }

    /// Applies module-owned GET/DELETE session admission for the HTTP transport.
    ///
    /// DELETE joins the session worker before acknowledging a known session;
    /// stale deletes remain idempotent. GET returns event-stream metadata only
    /// after the session and negotiated protocol version are validated.
    pub async fn http_session(
        &self,
        request: ServerHttpSessionRequest,
    ) -> ServerHttpResponseMetadata {
        if bounded(&request).is_err() {
            return http_error(400, "invalid HTTP session request");
        }
        let session_header = header_value(&request.headers, HEADER_SESSION_ID);
        let protocol_header = header_value(&request.headers, HEADER_PROTOCOL_VERSION);
        let Some(supplied_id) = session_header else {
            return http_error(400, "missing Mcp-Session-Id header");
        };
        match request.method {
            ServerHttpSessionMethod::Delete => {
                if request.requested_session_id.as_deref() == Some(supplied_id)
                    && let Some(internal_id) = request.internal_session_id.as_deref()
                    && self
                        .close_http_session(supplied_id, internal_id)
                        .await
                        .is_err()
                {
                    return http_error(500, "internal server error");
                }
                ServerHttpResponseMetadata {
                    status_code: 204,
                    headers: BTreeMap::new(),
                    content_type: None,
                    error_body: None,
                }
            }
            ServerHttpSessionMethod::Get => {
                let (Some(requested_id), Some(internal_id)) = (
                    request.requested_session_id.as_deref(),
                    request.internal_session_id.as_deref(),
                ) else {
                    return http_error(404, "unknown or expired MCP session");
                };
                if requested_id != supplied_id {
                    return http_error(404, "unknown or expired MCP session");
                }
                let Ok(identifier) = validate_id(internal_id) else {
                    return http_error(404, "unknown or expired MCP session");
                };
                let sessions = self.sessions.lock().await;
                let Some(session) = sessions.active.get(&identifier.to_string()) else {
                    return http_error(404, "unknown or expired MCP session");
                };
                let Some(context) = session.http.as_ref() else {
                    return http_error(404, "unknown or expired MCP session");
                };
                if !context.known_session
                    || context.requested_session_id.as_deref() != Some(requested_id)
                {
                    return http_error(404, "unknown or expired MCP session");
                }
                let expected_protocol = context.expected_protocol_version.clone();
                let client = Arc::clone(&session.client);
                drop(sessions);
                let metadata = client.lock().await.metadata();
                if !metadata.initialized {
                    return http_error(404, "unknown or expired MCP session");
                }
                if protocol_header != expected_protocol.as_deref()
                    || protocol_header != metadata.protocol_version.as_deref()
                {
                    return http_error(400, "missing or invalid MCP-Protocol-Version header");
                }
                ServerHttpResponseMetadata {
                    status_code: 200,
                    headers: BTreeMap::new(),
                    content_type: Some("text/event-stream".into()),
                    error_body: None,
                }
            }
        }
    }
    /// Submits or retries caller-known input. New input acknowledges prior terminal state.
    ///
    /// # Errors
    /// Rejects closed sessions, concurrent operations, conflicting/retired IDs,
    /// invalid headers, byte limits and non-increasing operation identifiers.
    pub async fn submit(
        &self,
        id: &str,
        headers: Map<String, Value>,
        input: ServerInput,
    ) -> Result<()> {
        bounded(&input)?;
        bounded(&headers)?;
        let operation_sequence = validate_id(&input.operation_id)?.as_u128();
        if serde_json::from_str::<Value>(&input.line)
            .is_ok_and(|value| value.as_array().is_some_and(|items| items.len() > 256))
        {
            return Err(invalid("server batch exceeds item limit"));
        }
        let http_initialization = is_initialize_request(&input.line);
        let session_header = header_value(&headers, HEADER_SESSION_ID).map(str::to_owned);
        let protocol_header = header_value(&headers, HEADER_PROTOCOL_VERSION).map(str::to_owned);
        let mut request_headers = RequestHeaders::new();
        for (key, value) in headers {
            request_headers.insert(
                &key,
                value
                    .as_str()
                    .ok_or_else(|| invalid("server header values must be strings"))?,
            );
        }
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .active
            .get_mut(&validate_id(id)?.to_string())
            .ok_or_else(|| invalid("unknown server session"))?;
        if let Some(operation) = &session.operation {
            if operation.input.operation_id == input.operation_id {
                return if operation.input == input && operation.headers == request_headers {
                    Ok(())
                } else {
                    Err(invalid("conflicting server operation retry"))
                };
            }
            if operation.terminal.is_none() {
                return Err(invalid("server session already has an operation"));
            }
        }
        if operation_sequence <= session.operation_sequence {
            return Err(invalid("server operation identifier already used"));
        }
        let http_rejection = if http_initialization {
            None
        } else {
            session.http.as_ref().and_then(|context| {
                http_admission_error(
                    context,
                    session_header.as_deref(),
                    protocol_header.as_deref(),
                )
            })
        };
        if let Some(http_response) = http_rejection {
            session.operation_sequence = operation_sequence;
            session.operation = Some(rejected_http_operation(
                input,
                request_headers,
                http_initialization,
                http_response,
            ));
            return Ok(());
        }
        let (sender, receiver) = mpsc::channel(1);
        let handler = Arc::new(CallbackHandler {
            info: serde_json::from_value(session.config.info.clone())
                .map_err(|_| invalid("invalid server identity"))?,
            prefix: session.config.source_type_prefix.clone(),
            resources: serde_json::from_value(Value::Array(session.config.resources.clone()))
                .map_err(|_| invalid("invalid server resources"))?,
            sender,
        });
        let client = Arc::clone(&session.client);
        let line = input.line.clone();
        let task_headers = request_headers.clone();
        let task = tokio::spawn(async move {
            let mut client = client.lock().await;
            crate::server::protocol::handle_line_bounded(
                &handler,
                &mut client,
                &task_headers,
                &line,
                super::MAX_BYTES,
            )
            .await
        });
        session.operation_sequence = operation_sequence;
        session.operation = Some(Operation {
            input,
            headers: request_headers,
            task: Some(task),
            receiver,
            waiting: None,
            terminal: None,
            reply_bytes: 0,
            http_initialization,
            http_response: None,
        });
        Ok(())
    }
    /// Observes replayable callbacks and terminal states without destructive drains.
    ///
    /// # Errors
    /// Rejects unknown or idle sessions.
    pub async fn poll(&self, id: &str) -> Result<ServerOperationSnapshot> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .active
            .get_mut(&validate_id(id)?.to_string())
            .ok_or_else(|| invalid("unknown server session"))?;
        let operation = session
            .operation
            .as_mut()
            .ok_or_else(|| invalid("server session has no operation"))?;
        operation.finish().await;
        let completed = operation.terminal.clone();
        let http_initialization = operation.http_initialization;
        if session.http.is_some()
            && operation.http_response.is_none()
            && let Some(state) = completed.as_ref()
        {
            let client = Arc::clone(&session.client);
            let client_metadata = client.lock().await.metadata();
            let (response, next_context) =
                http_response_metadata(state, http_initialization, &client_metadata);
            if let Some(next_context) = next_context {
                session.http = Some(next_context);
            }
            operation.http_response = Some(response);
        }
        let operation = session
            .operation
            .as_mut()
            .ok_or_else(|| invalid("server session has no operation"))?;
        let state = if let Some(terminal) = &operation.terminal {
            terminal.clone()
        } else if let Some(waiting) = &operation.waiting {
            ServerOperationState::Callback {
                callback: waiting.callback.clone(),
            }
        } else if let Ok(waiting) = operation.receiver.try_recv() {
            let callback = waiting.callback.clone();
            operation.waiting = Some(waiting);
            ServerOperationState::Callback { callback }
        } else {
            ServerOperationState::Pending
        };
        Ok(ServerOperationSnapshot {
            operation_id: operation.input.operation_id.clone(),
            state,
            http_response: operation.http_response.clone(),
        })
    }
    /// Completes one outstanding callback after host policy and execution.
    ///
    /// # Errors
    /// Rejects closed sessions, stale callback IDs and excessive aggregate replies.
    pub async fn complete(
        &self,
        id: &str,
        callback_id: &str,
        reply: ServerHostReply,
    ) -> Result<()> {
        let bytes = bounded(&reply)?;
        let mut sessions = self.sessions.lock().await;
        let operation = sessions
            .active
            .get_mut(&validate_id(id)?.to_string())
            .ok_or_else(|| invalid("unknown server session"))?
            .operation
            .as_mut()
            .ok_or_else(|| invalid("server session has no operation"))?;
        if operation.terminal.is_some() {
            return Err(invalid("server operation finished"));
        }
        let waiting = operation
            .waiting
            .as_ref()
            .ok_or_else(|| invalid("server callback is not outstanding"))?;
        if waiting.callback.id != callback_id {
            return Err(invalid("unknown server callback"));
        }
        if operation.reply_bytes + bytes > super::MAX_BYTES {
            return Err(invalid("server replies exceed aggregate byte limit"));
        }
        operation.reply_bytes += bytes;
        let waiting = operation
            .waiting
            .take()
            .ok_or_else(|| invalid("server callback is not outstanding"))?;
        waiting
            .reply
            .send(reply)
            .map_err(|_| invalid("server callback stopped"))
    }
    /// Cancels and joins active protocol work. Already terminal operations are preserved.
    ///
    /// # Errors
    /// Rejects unknown or idle sessions.
    pub async fn cancel(&self, id: &str, operation_id: &str) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        let operation = sessions
            .active
            .get_mut(&validate_id(id)?.to_string())
            .ok_or_else(|| invalid("unknown server session"))?
            .operation
            .as_mut()
            .ok_or_else(|| invalid("server session has no operation"))?;
        if operation.input.operation_id != operation_id {
            return Err(invalid("stale server cancellation"));
        }
        operation.finish().await;
        if operation.terminal.is_none() {
            operation.stop().await;
            operation.terminal = Some(ServerOperationState::Cancelled);
        }
        Ok(())
    }
    /// Closes a caller-known reservation, including one whose open is still queued.
    ///
    /// # Errors
    /// Rejects malformed identifiers. Retired identifiers are already closed.
    pub async fn close(&self, id: &str) -> Result<()> {
        let identifier = validate_id(id)?;
        let mut sessions = self.sessions.lock().await;
        if !sessions.active.contains_key(&identifier.to_string()) {
            sessions.reserve_close(identifier.as_u128());
        }
        if let Some(mut session) = sessions.active.remove(&identifier.to_string())
            && let Some(mut operation) = session.operation.take()
        {
            operation.stop().await;
        }
        Ok(())
    }
    async fn close_http_session(&self, requested_id: &str, internal_id: &str) -> Result<()> {
        let identifier = validate_id(internal_id)?;
        let mut sessions = self.sessions.lock().await;
        let Some(session) = sessions.active.get(&identifier.to_string()) else {
            sessions.reserve_close(identifier.as_u128());
            return Ok(());
        };
        if !session.http.as_ref().is_some_and(|context| {
            context.known_session && context.requested_session_id.as_deref() == Some(requested_id)
        }) {
            return Ok(());
        }
        if let Some(mut session) = sessions.active.remove(&identifier.to_string())
            && let Some(mut operation) = session.operation.take()
        {
            operation.stop().await;
        }
        Ok(())
    }
    /// Closes and joins all sessions on this object, retaining bounded retry fencing.
    pub async fn shutdown(&self) -> usize {
        let mut sessions = self.sessions.lock().await;
        let count = sessions.active.len();
        for (_, mut session) in sessions.active.drain() {
            if let Some(mut operation) = session.operation.take() {
                operation.stop().await;
            }
        }
        count
    }
}

#[cfg(test)]
#[path = "operations_tests.rs"]
mod tests;
