# Storage on the tinystoragedrivers ports

Status: accepted. Part of the storage consolidation tracked by
tinyhumansai/openhuman#7522 (child #7639).

## Why

This crate used to link `rusqlite` and keep two database files,
`mcp_clients/mcp_clients.db` and `mcp_audit/mcp_audit.db`, with server
environment values and OAuth bundles in a plain table. A module loaded over
TinyBus cannot assume a writable data directory, and a cloud host runs many
tenants in one process. The host owns storage; this crate asks for ports.

## What changed

- No database client. The crate depends on `tinystoragedrivers-core` (the
  `DocumentStore` and `StreamStore` ports and the `Blocking` bridge) and
  `tinystoragedrivers-secrets` (the `SecretStore` port), by git tag, and on no
  driver.
- `Store` and `AuditStore` open over `StoragePorts`
  (`documents`, `streams`, `secrets`). Their API stays synchronous; each call
  runs its port future on one shared bridge thread.
- A host supplies ports in process (`StoragePorts::from_scoped`), or installs a
  `StorageProvider` that maps a data directory to ports; the module's
  `McpService` asks the installed provider. The default is an in-memory
  provider. The storage bus client implements the same port traits, so the
  module code is identical over the bus.

## Layout

| Port | Name | Holds |
| --- | --- | --- |
| documents | `mcp_servers` | installed servers, indexed on `qualified_name` and `installed_at` |
| documents | `mcp_server_names` | one claim per qualified name; makes insert-if-absent atomic |
| documents | `mcp_registry_cache` | browse responses with a timestamp |
| documents | `mcp_tool_cache` | last advertised tools per server, keyed by a keyed fingerprint |
| documents | `mcp_registry_index` | the catalog, searched with `DocumentStore::search` |
| documents | `mcp_registry_index_state` | the sync cursor per catalog |
| streams | `mcp_audit` | tool-write audit records; id is offset + 1 |
| secrets | `mcp_client_env/<server id>` | environment values and the OAuth bundle, one secret |
| secrets | `mcp_cache_fingerprint_key` | key for tool-cache fingerprints (hex text) |

`storage::grant()` returns this as data (`StorageGrant`) for a host that serves
storage over the bus: it provisions exactly these collections (with the
indexes from `storage::collection_specs()`), this stream and these secret
names, and refuses the rest.

## Behaviour notes

- Index search: the port finds candidates by words and word prefixes; each is
  then checked so every query term is a substring of the label or description,
  as before. A term of one letter is checked but cannot select candidates.
- Insert-if-absent writes a claim with `Precondition::Absent` before the server.
  A claim whose server is gone (a crash between the two writes) is taken over.
- Deleting a server also removes its secret, its cached tools and its claim.
- Audit listing reads the stream in windows and filters, sorts and pages in
  memory, so ordering matches the previous SQL (`timestamp DESC, id DESC`).

## Not in this crate

- Importing an existing `mcp_clients.db` / `mcp_audit.db`. The host reads the
  legacy files read-only and writes the records through these ports
  (tinyhumansai/openhuman#7636).
- Selecting a backend. Features such as `storage-sqlite` belong to the host.
