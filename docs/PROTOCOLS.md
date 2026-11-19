# Shared protocol behavior

The sole PDF operation/schema authority is `contracts/v1/openapi.json` (1.0.0).
The contract hash and pinned Forge provenance are checked during Rust builds.
Discovery includes resolved input, success and error schemas, limits, capability
status, and explicit unsupported/excluded capabilities.

| Operation | CLI | HTTP | MCP tool |
|---|---|---|---|
| inspectPdf | inspect / validate | POST /v1/inspect | inspectPdf |
| mergePdf | merge | POST /v1/merge | mergePdf |
| extractPdf | extract | POST /v1/extract | extractPdf |
| splitPdf | split | POST /v1/split | splitPdf |

CLI discovery/utility commands: `operations`, `schema <operationId>`, `help [command]` / `--help` / `-h`, `version` / `--version` / `-V`.
When neither `--qpdf` nor `ANPDF_QPDF` is set, the CLI/server checks standard system paths (`/opt/qpdf/bin/qpdf`, `/usr/bin/qpdf`, `/opt/homebrew/bin/qpdf`, `/usr/local/bin/qpdf`) and trusted `PATH` directories (absolute, owned by the effective user or root, without group/other write bits).

Authenticated HTTP discovery: GET `/v1/operations` and `/v1/openapi.json`.
A canonical error is `{contractVersion, operationId, error:{code,message,retryable}}`.
Messages never include raw backend output, absolute input paths, or credentials.

| Error | HTTP | CLI |
|---|---|---|
| INVALID_REQUEST | 400 | 2 |
| NOT_FOUND / ACCESS_DENIED | 404 / 403 | 3 |
| OUTPUT_EXISTS / INPUT_CHANGED | 409 | 4 |
| INVALID_PDF / ENCRYPTED_UNSUPPORTED / SIGNED_UNSUPPORTED / FORM_UNSUPPORTED / PRESERVATION_UNKNOWN | 422 | 5 |
| UNSUPPORTED_OPERATION | 422 | 6 |
| LIMIT_EXCEEDED / TIMEOUT | 413 / 504 | 7 |
| BACKEND_UNAVAILABLE / INTERNAL | 503 / 500 | 8 |
| UNAUTHENTICATED | 401 | HTTP only |

CLI cancellation/termination exits 130 after cancelling work. `--dry-run` returns
a surface-control response identifying that content validation still requires
processing; it is not a PDF success result or a fourth PDF operation.

MCP is newline-delimited JSON-RPC 2.0 on stdio, negotiating version 2025-11-25.
Send each line and await its response; initialization notification has no response:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"example-client","version":"1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspectPdf","arguments":{"input":{"path":"three.pdf"}}}}
```

Successful MCP tools return validated `structuredContent` plus equivalent JSON
text. Application failures use `isError:true` and canonical JSON error text.
Invalid JSON-RPC/protocol requests use protocol errors. Unknown tools are not
registered. `notifications/cancelled` cancels the matching active request. EOF
cancels active work and waits for cleanup. No log messages are written to stdout.
The adapter uses the owned contract for tool schemas and truthful read-only and
local-operation annotations; annotations do not grant filesystem permissions.

The integration corpus drives all three surfaces with identical requests and
compares complete domain envelopes, errors, discovery schemas, and input hashes.
The example MCP sequence, CLI invocations and authenticated HTTP operations are
exercised by that harness, not by an external client certification suite.
