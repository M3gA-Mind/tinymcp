//! The jailed launcher drives a fake `TinyBox` over the bus seam.
//!
//! The fake answers the same members the module serves and records every call,
//! so the tests pin what is sent (grants, enforcement, argv, environment), what
//! is refused, and what is cleaned up.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::transport::stdio::McpStdioClient;
use serde_json::json;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;
use tinymcp_bus::{LATEST_PROTOCOL_VERSION, McpClientIdentityConfig};

#[derive(Debug, Default)]
struct State {
    calls: Vec<(&'static str, Value)>,
    stdout: VecDeque<u8>,
    written: Vec<u8>,
    closed: bool,
    start_error: Option<&'static str>,
    contract: Option<(u32, u32)>,
    next: u32,
    behavior: Behavior,
    /// Members called over the confidential route.
    confidential: Vec<&'static str>,
}

/// What the fake does differently from a plain, silent module.
#[derive(Debug, Default)]
struct Behavior {
    /// Reply as an MCP server to each line written to it.
    respond: bool,
    /// Make `Create` fail.
    create_error: bool,
    /// Report stdout closed while stderr stays open.
    stdout_only_closed: bool,
}

#[derive(Debug, Default)]
struct FakeBox {
    state: Mutex<State>,
    wake: tokio::sync::Notify,
}

impl FakeBox {
    fn serving() -> Arc<Self> {
        let fake = Self::default();
        fake.state.lock().unwrap().behavior.respond = true;
        Arc::new(fake)
    }
    fn members(&self) -> Vec<&'static str> {
        self.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .map(|c| c.0)
            .collect()
    }
    fn args(&self, member: &str) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|call| call.0 == member)
            .map(|call| call.1.clone())
            .collect()
    }
}

impl JailBus for FakeBox {
    fn call(
        &self,
        member: &'static str,
        args: Value,
        confidential: bool,
    ) -> BoxFuture<'_, Result<Value, JailBusError>> {
        Box::pin(async move {
            // Positional array in, as the bus delivers it; the fake works on
            // the single argument.
            assert!(args.is_array(), "arguments travel positionally");
            let args = args.get(0).cloned().unwrap_or(Value::Null);
            if member == names::READ_STDIO {
                // Long-poll like the module: wait briefly for output.
                let _ = tokio::time::timeout(Duration::from_millis(20), self.wake.notified()).await;
                let mut state = self.state.lock().unwrap();
                let out: Vec<u8> = state.stdout.drain(..).collect();
                let closed = state.closed && state.stdout.is_empty();
                let stdout_closed = closed || state.behavior.stdout_only_closed;
                return Ok(json!({
                    "stdout": out, "stderr": [], "stdout_closed": stdout_closed,
                    "stderr_closed": closed, "status": {"running": !closed}
                }));
            }
            let mut state = self.state.lock().unwrap();
            state.calls.push((member, args.clone()));
            if confidential {
                state.confidential.push(member);
            }
            match member {
                m if m == names::CAPABILITIES => Ok(json!({
                    "contract_version": state.contract.unwrap_or(tinybox_bus::CONTRACT_VERSION),
                    "create_backends": [], "exec_backends": [], "spawn_backends": []
                })),
                m if m == names::RESERVE => {
                    state.next += 1;
                    Ok(json!(format!("id-{}", state.next)))
                }
                m if m == names::CREATE && state.behavior.create_error => Err(JailBusError {
                    name: Some(tinybox_bus::INVALID_FILE_PATH.into()),
                    message: "bad workspace".into(),
                }),
                m if m == names::CREATE => Ok(json!({
                    "resource": args["resource"], "backend": "passthrough",
                    "state": "running", "published_ports": []
                })),
                m if m == names::START_STDIO => {
                    if let Some(name) = state.start_error {
                        return Err(JailBusError {
                            name: Some(name.into()),
                            message: "refused".into(),
                        });
                    }
                    Ok(json!({
                        "resource": args["resource"], "process": args["process"]
                    }))
                }
                m if m == names::WRITE_STDIO => {
                    let data: Vec<u8> = serde_json::from_value(args["data"].clone()).unwrap();
                    state.written.extend_from_slice(&data);
                    if state.behavior.respond {
                        let text = String::from_utf8_lossy(&data).into_owned();
                        for line in text.lines() {
                            let request: Value = serde_json::from_str(line).unwrap();
                            if let Some(id) = request.get("id") {
                                let result = match request["method"].as_str() {
                                    Some("initialize") => json!({
                                        "protocolVersion": LATEST_PROTOCOL_VERSION,
                                        "capabilities": {}, "serverInfo": {"name": "fake"}
                                    }),
                                    _ => json!({"tools": []}),
                                };
                                let reply = json!({"jsonrpc": "2.0", "id": id, "result": result});
                                state.stdout.extend(format!("{reply}\n").into_bytes());
                            }
                        }
                        self.wake.notify_waiters();
                    }
                    Ok(Value::Null)
                }
                m if m == names::CLOSE_STDIN || m == names::KILL_STDIO => {
                    state.closed = true;
                    self.wake.notify_waiters();
                    Ok(Value::Null)
                }
                _ => Ok(Value::Null),
            }
        })
    }
}

fn sandbox() -> StdioSandboxConfig {
    StdioSandboxConfig {
        workspace: "/work/mcp".into(),
        read_only: vec!["/usr".into()],
        read_write: vec!["/work/mcp/cache".into()],
        deny_read: vec!["/home/user/.ssh".into()],
        allow_net: true,
        allow_subprocess: false,
        require_process_isolation: true,
    }
}

fn launch() -> StdioLaunch {
    StdioLaunch {
        program: "/usr/bin/server".into(),
        args: vec!["--stdio".into()],
        env: vec![("TOKEN".into(), "secret-value".into())],
        cwd: Some("/work/mcp/run".into()),
    }
}

#[tokio::test]
async fn a_handshake_runs_through_the_jail_with_the_hosts_grants() {
    let fake = FakeBox::serving();
    let spawner = Arc::new(JailedSpawner::new(fake.clone(), sandbox()));
    let client = McpStdioClient::new(
        "sh",
        vec![],
        vec![],
        None,
        &McpClientIdentityConfig::default(),
    )
    .with_spawner(spawner);
    client.initialize().await.expect("handshake over the jail");
    assert_eq!(client.list_tools().await.unwrap().len(), 0);

    let members = fake.members();
    assert_eq!(
        &members[..5],
        [
            names::CAPABILITIES,
            names::RESERVE,
            names::CREATE,
            names::RESERVE,
            names::START_STDIO
        ]
    );
    let create = &fake.args(names::CREATE)[0];
    assert_eq!(create["backend"], "passthrough");
    assert_eq!(create["workspace"], json!({"Directory": "/work/mcp"}));
    let start = &fake.args(names::START_STDIO)[0];
    assert_eq!(start["jail"]["enforcement"], "enforced");
    assert_eq!(start["jail"]["read_only"], json!(["/usr"]));
    assert_eq!(start["jail"]["read_write"], json!(["/work/mcp/cache"]));
    assert_eq!(start["jail"]["deny_read"], json!(["/home/user/.ssh"]));
    assert_eq!(start["jail"]["allow_net"], true);
    assert_eq!(start["jail"]["allow_subprocess"], false);
    assert_eq!(start["jail"]["require_process_isolation"], true);

    client.close_session().await.unwrap();
    let members = fake.members();
    assert!(members.contains(&names::KILL_STDIO));
    assert_eq!(members.last(), Some(&names::CLOSE));
}

#[tokio::test]
async fn launch_arguments_reach_the_jailed_start_unchanged() {
    let fake = FakeBox::serving();
    let spawner = JailedSpawner::new(fake.clone(), sandbox());
    let mut process = spawner.spawn_process(launch()).await.unwrap();
    let start = &fake.args(names::START_STDIO)[0];
    assert_eq!(start["argv"], json!(["/usr/bin/server", "--stdio"]));
    assert_eq!(start["cwd"], "/work/mcp/run");
    assert_eq!(start["env"], json!({"TOKEN": "secret-value"}));
    process.handle.terminate().await;
}

#[tokio::test]
async fn a_refused_start_is_a_permission_error_and_closes_the_resource() {
    for name in [
        tinybox_bus::UNSUPPORTED_OPERATION,
        tinybox_bus::INVALID_FILE_PATH,
    ] {
        let fake = FakeBox::serving();
        fake.state.lock().unwrap().start_error = Some(name);
        let spawner = JailedSpawner::new(fake.clone(), sandbox());
        let error = spawner.spawn_process(launch()).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        let members = fake.members();
        assert_eq!(members.last(), Some(&names::CLOSE));
        assert!(!members.contains(&names::KILL_STDIO));
    }
}

#[tokio::test]
async fn another_start_failure_is_not_reported_as_a_policy_refusal() {
    let fake = FakeBox::serving();
    fake.state.lock().unwrap().start_error = Some(tinybox_bus::BACKEND_ERROR);
    let spawner = JailedSpawner::new(fake.clone(), sandbox());
    let error = spawner.spawn_process(launch()).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Other);
}

#[tokio::test]
async fn a_sandbox_module_too_old_for_stdio_is_refused_before_any_resource_exists() {
    let fake = FakeBox::serving();
    fake.state.lock().unwrap().contract = Some((1, 20));
    let spawner = JailedSpawner::new(fake.clone(), sandbox());
    let error = spawner.spawn_process(launch()).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    assert_eq!(fake.members(), [names::CAPABILITIES]);
}

#[tokio::test]
async fn the_jailed_spawner_never_produces_a_child_process() {
    let spawner = JailedSpawner::new(FakeBox::serving(), sandbox());
    let error = spawner.spawn(launch()).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
}

#[tokio::test]
async fn dropping_a_started_server_releases_it_in_the_background() {
    let fake = FakeBox::serving();
    let spawner = JailedSpawner::new(fake.clone(), sandbox());
    let process = spawner.spawn_process(launch()).await.unwrap();
    drop(process);
    tokio::time::timeout(Duration::from_secs(5), async {
        while fake.members().last() != Some(&names::CLOSE) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the dropped server was killed and its resource closed");
    assert!(fake.members().contains(&names::KILL_STDIO));
}

#[tokio::test]
async fn client_writes_are_forwarded_and_end_of_input_closes_stdin() {
    let fake = FakeBox::serving();
    fake.state.lock().unwrap().behavior.respond = false;
    let spawner = JailedSpawner::new(fake.clone(), sandbox());
    let mut process = spawner.spawn_process(launch()).await.unwrap();
    process.stdin.write_all(b"ping\n").await.unwrap();
    process.stdin.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !fake.members().contains(&names::CLOSE_STDIN) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("stdin was closed");
    assert_eq!(fake.state.lock().unwrap().written, b"ping\n");
    // The fake reports end of file once stdin closed, so the reader ends.
    let mut rest = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        process.stdout.read_to_end(&mut rest),
    )
    .await
    .expect("stdout reaches end of file")
    .unwrap();
    assert_eq!(rest.len(), 0);
    process.handle.terminate().await;
}

#[tokio::test]
async fn an_untrusted_server_is_refused_when_no_sandbox_is_configured() {
    let launchers = StdioLaunchers::new();
    assert!(!launchers.has_sandbox());
    let error = launchers
        .for_server(false)
        .spawn_process(launch())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("without a sandbox"), "{error}");
}

#[tokio::test]
async fn an_untrusted_server_uses_the_sandbox_and_never_the_direct_spawn() {
    let fake = FakeBox::serving();
    let launchers =
        StdioLaunchers::sandboxed(Arc::new(JailedSpawner::new(fake.clone(), sandbox())));
    assert!(launchers.has_sandbox());
    let mut process = launchers
        .for_server(false)
        .spawn_process(launch())
        .await
        .unwrap();
    assert!(fake.members().contains(&names::START_STDIO));
    process.handle.terminate().await;
}

#[cfg(unix)]
#[tokio::test]
async fn only_an_operator_trusted_server_gets_the_direct_spawn() {
    let fake = FakeBox::serving();
    let launchers =
        StdioLaunchers::sandboxed(Arc::new(JailedSpawner::new(fake.clone(), sandbox())));
    let mut process = launchers
        .for_server(true)
        .spawn_process(StdioLaunch {
            program: "/bin/cat".into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
        })
        .await
        .unwrap();
    assert!(
        fake.members().is_empty(),
        "a trusted server bypasses the jail"
    );
    process.handle.terminate().await;
    // With no sandbox at all, trust still starts it directly.
    let mut process = StdioLaunchers::new()
        .for_server(true)
        .spawn_process(StdioLaunch {
            program: "/bin/cat".into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
        })
        .await
        .unwrap();
    process.handle.terminate().await;
}

#[tokio::test]
async fn only_the_start_that_carries_the_environment_is_confidential() {
    let fake = FakeBox::serving();
    let spawner = JailedSpawner::new(fake.clone(), sandbox());
    let mut process = spawner.spawn_process(launch()).await.unwrap();
    assert_eq!(
        fake.state.lock().unwrap().confidential,
        [names::START_STDIO]
    );
    process.handle.terminate().await;
}

#[tokio::test]
async fn a_failed_create_closes_the_reserved_resource() {
    let fake = FakeBox::serving();
    fake.state.lock().unwrap().behavior.create_error = true;
    let spawner = JailedSpawner::new(fake.clone(), sandbox());
    let error = spawner.spawn_process(launch()).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    let members = fake.members();
    assert_eq!(members.last(), Some(&names::CLOSE));
    assert!(!members.contains(&names::START_STDIO));
}

#[tokio::test]
async fn the_reader_ends_when_stdout_closes_even_if_stderr_stays_open() {
    let fake = FakeBox::serving();
    fake.state.lock().unwrap().behavior.respond = false;
    let spawner = JailedSpawner::new(fake.clone(), sandbox());
    let mut process = spawner.spawn_process(launch()).await.unwrap();
    // The fake reports both streams closed together; make only stdout close.
    fake.state.lock().unwrap().behavior.stdout_only_closed = true;
    let mut rest = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        process.stdout.read_to_end(&mut rest),
    )
    .await
    .expect("stdout reaches end of file without stderr closing")
    .unwrap();
    process.handle.terminate().await;
}
