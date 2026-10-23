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

- No database client. The crate depends on the `tinystoragedrivers-api` and
  `tinystoragedrivers-types` port contracts and the bus proxy, and on no
  storage engine.
- `Store` and `AuditStore` open over `StoragePorts`
  (`documents`, `streams`, `secrets`). Their API stays synchronous; each call
  runs its port future on one shared bridge thread.
- A host supplies ports in process (`StoragePorts::from_storage`), or installs a
  `StorageProvider` that maps a data directory to ports; the module's
  `McpService` asks the installed provider. Tests may install an in-memory
  provider. A loaded module requires an explicit host callback destination and
  never falls back to process-local memory. The storage bus client implements
  the same port traits, so the module code is identical over the bus.

## Layout

| Port | Name | Holds |
| --- | --- | --- |
| documents | `mcp.servers` | installed servers, indexed on `qualified_name` and `installed_ms` |
| documents | `mcp.server_names` | one claim per qualified name; makes insert-if-absent atomic |
| documents | `mcp.registry_cache` | browse responses with a timestamp |
| documents | `mcp.tool_cache` | last advertised tools per server, keyed by a keyed fingerprint |
| documents | `mcp.registry_index` | the catalog, searched with `DocumentStore::search` |
| documents | `mcp.registry_index_state` | the sync cursor per catalog |
| streams | `mcp.audit` | tool-write audit records; id is offset + 1 |
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
- The v1 audit-stream import uses `StreamStore::append_batch_if_offset` to
  serialize competing owner opens without duplicating entries. A host stream
  backend that lacks conditional append support fails migration closed.

## Hosting as a loadable module

Loaded as a `cdylib`, the module cannot reach the host's storage handles. When
its host-provided `storage_callback` destination is present and the host has
not installed a `StorageProvider` itself, `setup` connects to that exact
callback (`BusProvider`, over
`tinystoragedrivers-busproxy::connect_storage_at`) and installs it as the
provider. The host binds the scope from the broker-authenticated caller and
enforces the grant above; the module never names a tenant. Every data
directory answers the one host-bound scope. A host with no storage objects
fails the load rather than falling back to memory, which would lose every
install at the next restart. A host that links the module into its own
process installs its own provider before `setup` and skips the bus.

A host that serves one storage service for every module, rather than binding a
private callback to each instance, names that service as the destination:
`storage_callback = "ai.tinyhumans.tinystorage.Storage"`. `setup` then connects
to the peer that owns the name (`BusProvider::connect_service`). The host still
binds the scope from the broker-authenticated caller and enforces the grant.
The name is explicit and exact, so an absent destination, or a service nobody
owns, still fails the load rather than falling back to memory.

Ports reached over the bus are host-managed: the host declared the module's
collections when it bound the scope and its grant is the allowlist, so
`prepare` only re-declares the current collections. It does not run the
in-module migrations, whose shared progress ledger and v1 source collections
lie outside a host grant. Migration of v1 records is the host's import.

Secrets travel on the confidential route, which the broker completes only to
an attested recipient. The host's storage objects must therefore be served by
an attested artifact before a released module can use the bus provider in
production; an in-process broker in tests cannot supply that, so the bus
tests keep secrets in a local store.

## Not in this crate

- Importing an existing `mcp_clients.db` / `mcp_audit.db`. The host reads the
  legacy files read-only and writes the records through these ports
  (tinyhumansai/openhuman#7636).
- Selecting a backend. Features such as `storage-sqlite` belong to the host.

## Schema v2

The module's private declaration is composed from the public
`tinymcp_bus::storage::collection_specs()` (owner `mcp`, tenant-scoped data,
tinystoragedrivers v2.1); the storage grant includes those destinations and
the legacy migration sources. v1 names become `mcp.*`, `*_at` times become
`*_ms` fields, ids use the released escaped composite-key encoding with its
overlength digest fallback, and JSON kept inside strings (`body_json`,
`record_json`) becomes an object body. Steps `servers_v2`, `server_names_v2`,
`registry_cache_v2`, `tool_cache_v2`, `registry_index_v2`,
`registry_index_state_v2` and the stream step `audit_streams_v2` migrate a v1 database.
Secret names are unchanged (a secret cannot be renamed by a step, and they sit
outside the `mcp/` owner prefix).
