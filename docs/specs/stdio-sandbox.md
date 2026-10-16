# Jailed stdio servers

A stdio MCP server is third-party code. The module starts one of three ways:

| Server | Started |
| --- | --- |
| `McpServerConfig::operator_trusted = true` | As an ordinary child process, unconfined. Only the host's own configuration should set it. |
| Any other, with `McpClientConfig::stdio_sandbox` set | Inside the `TinyBox` native jail, through the `TinyBox` bus contract (`StartStdio`, contract 1.21). |
| Any other, with no sandbox | Refused. There is no fallback to an unconfined start. |

Every registry-installed server (catalog installs and setup test connections) is
in the second or third row; installed servers are never operator-trusted.

## The jail

`StdioSandboxConfig` carries the workspace (the server's writable root), read
and write grants, `deny_read` paths (credential stores), network and subprocess
permissions, and whether the backend must meet the untrusted-code isolation
floor (on by default). The launcher always asks for `Enforced`: a backend that
cannot enforce the grants refuses and the server does not start.

The launcher (`transport::stdio::jailed::JailedSpawner`) reserves a passthrough
resource over the workspace, starts the server with `StartStdio`, and pumps its
stdin and stdout over `WriteStdio` and `ReadStdio` (standard error is discarded,
as for a direct child). Closing the session, or dropping it, calls `KillStdio`
and `Close`. `StartStdio` carries the server's environment (credentials), so it is sent over the bus's confidential route and fails when that delivery is unavailable. The module reaches `TinyBox` only through `tinybox-bus`; the
module declares it as an optional dependency.

## Seam

`StdioSpawner` gained `spawn_process`, which returns a `StdioProcess` (pipes plus
a handle). Its default wraps `spawn`, so existing spawners are unchanged. A
spawner whose server is not a child of this process overrides it.
