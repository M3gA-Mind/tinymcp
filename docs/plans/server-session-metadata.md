# Server session metadata implementation

The accepted behavior is in `docs/specs/mcp-server.md`: a bus client can query
module-owned initialization state and negotiated protocol version without
parsing an MCP response.

1. Add typed session and HTTP response metadata, plus an HTTP-aware open member;
   pin serde forms and update the contract minor version.
2. Record successful initialization and the selected protocol version in the
   module-owned client session; validate HTTP admission before protocol dispatch
   and return status/header metadata from `ServerSessions`.
3. Add bridge regressions for initialize negotiation, absent and stale session
   IDs, version mismatch, and callback refusal before initialization. Run
   focused tests first, then workspace contract gates and module artifact checks.

The response body and all pre-existing member arities remain unchanged.
