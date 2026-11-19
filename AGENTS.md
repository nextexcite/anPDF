# AGENTS.md — working guide for coding agents

Applies to this entire repository. Read `README.md`, `docs/ADR-002.md`, `docs/FORGE.md`, `docs/PROTOCOLS.md`, `docs/QUALIFICATION.md`, and `SECURITY.md` before changing an interface or a safety property.

## Mission and intended users

Build **anPDF**, an independent, open-source, agent-native PDF toolkit. It should provide dependable document primitives to **coding agents, personal agents, desktop automations, scripts, self-hosted APIs, and OCI/cloud jobs**. A user should be able to inspect a document, extract pages, combine files, or eventually fill a form and sign it without a heavyweight editor, a remote SaaS account, or an always-on background process.

Treat the CLI, MCP, and HTTP API as three interfaces to **one owned operation contract**; do not create a separate semantics layer per transport. A desktop UI or remote/cloud service may be added later as a client of these primitives. This project is **not** a PDF viewer, a fork of another PDF product, or an attempt to rewrite the PDF specification.

Examples and fixtures must be entirely generic: synthetic reports, blank forms, sample contracts, numbered pages. Do not insert real users' documents or personal workflows.

## Current baseline (do not overclaim)

- `0.1.0-alpha.1`: Rust 1.100.0 runtime; qpdf **12.4.2** adapter; inspect/validate, merge, selected-page extract, multi-output split, page removal, and lossless/lossy compression.
- CLI JSON, optional authenticated **loopback-only** HTTP, and local MCP stdio use the same validated dispatcher.
- `contracts/v1/openapi.json` is canonical. Pinned upstream Cloudflare **Forge 0.2.0** generates qualified TypeScript/MCP/Cap'n Web artifacts at build time. The Rust runtime adapters are **handwritten**, not Forge-generated Rust.
- Hosted CI qualifies `ubuntu-26.04` (Linux amd64) and `macos-26` (macOS arm64), plus Forge output probes and full-history privacy checks. Preliminary publication (`.github/workflows/oci.yml`) targets the `linux/amd64` OCI image (GHCR) and the native `macos/arm64` release tarball (GitHub prerelease). `linux/arm64` OCI, remote HTTP hosting, and full production release gates have **not** been established. See `docs/QUALIFICATION.md` and `docs/BUILD.md` for what actually passed.
- Page reordering, forms, signatures, OCR, and existing-text replacement are **not implemented**. State that explicitly in API discovery, docs, and reports.

## Repository map

- `contracts/v1/openapi.json`: operation IDs, schemas, versioned API authority.
- `src/contract.rs`, `src/lib.rs`, `src/error.rs`: contract validation, shared dispatcher, canonical errors.
- `src/main.rs`, `src/http.rs`, `src/mcp.rs`: CLI and transport adapters.
- `src/backend.rs`, `src/filesystem.rs`: qpdf adapter, process controls, path and output policy.
- `generated/`, `build/forge-pin.json`, `build/generate.py`: pinned Forge outputs, provenance and regeneration.
- `fixtures/`, `tests/integration.py`, `src/tests.rs`: synthetic corpus and cross-surface tests.
- `containers/`, `.github/workflows/`, `docs/BUILD.md`: pending and preliminary OCI/CI qualification.
- `build/privacy.py`, `SECURITY.md`, `license-manifest.json`: privacy, threat model, license evidence.

## Implementation rules

1. **Contract first.** Define or update versioned operation semantics and JSON schemas before changing implementations. Keep CLI, HTTP, MCP, error codes, discovery, cancellation and dry-run behavior equivalent. Unsupported features must fail explicitly; never simulate success.
2. **Use Forge, but verify it.** Keep the upstream source/version/checksum pin. Generate via the documented official pipeline, review the resulting artifacts and executable schema-fidelity probes, and update provenance and tests. Do not hand-edit `generated/`; do not represent Forge's generated MCP/TS artifacts as the running Rust adapter. Known upstream fidelity gaps are documented in `docs/FORGE.md`. If the pinned archive is unavailable, report the blocker rather than fabricating regenerated output.
3. **Keep the core small.** Ordinary CLI/MCP use must not require Node, a web server, a cloud account, or a resident daemon. Rust owns the runtime and transport-neutral dispatcher; backends are replaceable. qpdf remains the qualified structural backend until another passes the same tests. Evaluate alternate engines as **optional adapters**, not the project identity.
4. **Preserve document semantics.** Do not overwrite inputs. Paths must remain workspace-relative; no URL inputs, path traversal, symlinks, device files, or silent fallback to flatten/rasterize. Respect encrypted/form/XFA/signed/preservation-unknown rejection until a tested preservation policy exists. Distinguish visual signatures from certificate-based digital signatures.
5. **Keep security boundaries explicit.** CLI/MCP initiate no listener. HTTP remains loopback-only with required bearer authentication until a separate remote-serving threat model and tests exist. A Rust process is not a network sandbox: use OS/container controls for network denial and resource isolation. Avoid shell execution, unbounded allocations/streams, raw backend errors, and secret-bearing logs.
6. **Be portability-minded.** Prefer OCI-compatible, non-root, read-only, ephemeral workflows with pinned versions and inspectable SBOMs. Don't introduce OS-specific assumptions without isolating and testing them.

## Practical usage scenarios

- **Local/desktop automation:** one-shot `anpdf inspect|merge|extract|split|remove-pages|compress` in a private, dedicated workspace; JSON result to stdout, canonical error to stderr. No background worker.
- **Coding/personal-agent tool use:** `anpdf operations` / `anpdf schema <operationId>` for discovery, then `anpdf mcp --root <workspace>` using stdio. Agent requests must be schema-valid, scoped to that root, and checked for side effects.
- **Local service integration:** `anpdf serve --root <workspace> --token-file <private-file>` on 127.0.0.1; keep credentials and client configuration outside the repository.
- **OCI and future cloud:** run against explicitly mounted inputs/outputs, as non-root with network disabled by default. Remote multi-user hosting is a **future security and architecture project**, not today's loopback listener.

## Verification required before merging

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
ANPDF_QPDF="$(command -v qpdf)" python3 tests/integration.py
```

The integration suite requires qpdf 12.4.2 on your PATH and Poppler on the test host. For Forge changes, also run the official pinned generation procedure and TypeScript/MCP/Cap'n Web qualification probe from `docs/FORGE.md`. For OCI changes, use `docs/BUILD.md` and actually execute the image and runtime smoke tests before claiming qualification. Add malformed/edge fixtures and compare complete response envelopes across CLI, HTTP and MCP; record input hashes and output validity. Never label a test “passed” without execution evidence.

## Public repository, attribution, and release discipline

**Assume every commit, branch, PR, fixture, generated artifact, log, and CI result is public forever.** Never commit personal names, emails, home/workspace paths, domains, account names, tokens, real customer documents, private scenarios, non-public prompts, or private Git history. Use only synthetic or appropriately licensed fixtures; do not include private PDFs even temporarily. Sanitize output and run `build/privacy.py` and the existing gitleaks checks before publication. The current privacy gate expects generic commit author/committer metadata (`anPDF Contributors <contributors@anpdf.invalid>`); do not weaken that gate to make an inconvenient tool pass.

Do not change third-party licenses or add AGPL/dual-licensed processing backends without an explicit compatibility review, notices, and distribution analysis. Do not publish a registry image, new release, package, non-loopback service, or a production-readiness claim without separate review of OCI/build provenance, licenses, vulnerabilities, privacy and platform qualification.

**When reporting work**, state what changed, which commands actually ran, the observed outcomes, known limitations, and the commit SHA. Keep planning and implementation distinct; prefer a verifiable small vertical slice to a feature checklist.
