# tinymcp-bus

Shared serialized MCP vocabulary, fixed tool declarations, errors, method names
and contract version. The implementation re-exports the same definitions.
Normal/build dependency closures contain only serialization/schema utilities,
including with all features enabled. No optional runtime, transport, native or
TinyTools dependency belongs here.

DTO constructors, fixed catalog/schema declarations and serde helpers are
vocabulary. Protocol routing, transport, argument normalization, dynamic
metadata preparation, query filtering, persisted-value parsing and redaction
execute in the implementation. Generic lexical helpers live in TinyTools.
The sanitize module retains only MCP presentation limits.

Contract 1.9 adds four one-argument operations: TransformText,
NormalizeToolArguments, DisplayRemoteTool and RenderToolOutput. Their requests
and output are capped at 1 MiB. Rendering applies its cap incrementally, before
buffering oversized pretty JSON or joined content. Existing member arities and
wire forms are preserved. Prefer member constants from names::methods.

Contract 1.10 adds the one-argument `OpenConfigured` operation and its
`OpenConfiguredRequest` payload. It pairs a data directory with that host's
MCP client settings; `Open` and its original string argument remain unchanged.
Because those settings may contain credentials, callers send this request using
TinyBus's confidential-call path. Contract 1.11 adds `Close(data_dir)` and the
nonblocking `TransitionStatus(id)` observation. `OpenConfigured` returns an
`OpenConfiguredResponse` carrying the object path and lifecycle transition id;
replacement and old-service cleanup are complete only when the status is
`complete`. The root remains available after close, with no persistent
directory or static client configuration.

Contract 1.12 adds no-argument `CachedOverview`, which returns live tools for
connected installs and persisted, fingerprint-matched cached tools for enabled
disconnected installs without dialling them. `StaticCachedListTools(server)`
reads one static server's persisted cache without a network request;
`StaticListTools(server)` keeps its response shape and refreshes that cache
after a successful listing. Both cached reads use the current definition's
fingerprint and reapply its allow and deny rules.

Contract 1.14 adds module-owned configuration-document read and apply members.
The apply request can contain credentials and therefore requires TinyBus's
confidential-call path. Its response includes the re-rendered document, which
contains credential names but never credential values, and the ordered change
report.

Library consumers bring implementation extension traits into scope for moved
DTO methods. Contract-only hosts call the compiled module. See
[the relocation specification](../../docs/specs/pure-vocabulary.md) for public
API compatibility and remaining transport/host integration work.
