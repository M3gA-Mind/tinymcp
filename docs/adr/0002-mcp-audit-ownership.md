# 2. MCP protocol telemetry stays in tinymcp; policy events forward to the host audit stream

- **Status:** Accepted
- **Date:** 2026-10-14

## Context

The OpenHuman host is consolidating security, policy, approvals and sandboxing
behind the `tinysecurity` bus module (tinyhumansai/openhuman#7328). One goal is
a single audit log for approvals, shell and sandbox decisions, judge verdicts
and policy changes. tinymcp already keeps its own append-only `mcp_audit`
stream (`AuditStore`, `tinymcp_bus::audit`) recording every tool write, so the
question is whether that log moves into the security module or stays here.

The two record different things. The `mcp_audit` rows describe MCP protocol
activity: which server, which tool, a bounded argument summary, the outcome and
the (length-bounded) remote error. They are meaningful with no host policy at
all, and a standalone tinymcp user relies on them. Policy events (an approval
parked, granted or denied, an auto-approve verdict, a sandbox decision) are
decided by the host, not by tinymcp.

## Decision

- MCP protocol telemetry stays tinymcp-owned. `AuditStore`, the `mcp_audit`
  stream, its summary-not-arguments rule and its bounds do not move into
  tinysecurity, and the host does not re-implement them.
- Policy events are not written by tinymcp. The host (through tinysecurity)
  emits them to the shared audit stream, keyed so a policy row and the
  `mcp_audit` row for the same call can be correlated by server id, tool name
  and timestamp.
- No new forwarding hook is added to tinymcp. The existing seam is sufficient:
  `McpToolInvoker` lets a host wrap every call with its approval gate and
  audit emission, and `AuditStore` exposes the write records for a host to read
  and forward. The host's `mcp/audit` becomes a thin facade over those two.

## Consequences

- tinymcp stays usable and auditable without tinysecurity; no dependency on it.
- The host has one policy audit stream plus the tinymcp protocol log, joined at
  read time rather than merged at write time.
- If a host later needs push-style forwarding of `mcp_audit` rows, that is a
  new ADR and a contract change in `tinymcp-bus`.
