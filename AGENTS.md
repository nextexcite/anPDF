# Developing anPDF

This guide applies to the repository. For **using** the installed tool on PDFs, follow the [user guide](README.md) or the [document-processing skill](.agents/skills/processing-pdfs-with-anpdf/SKILL.md). For **changing** anPDF, start with the [maintainer handoff](docs/HANDOFF.md); it owns current status and priorities.

## Change the contract once

- Before changing an interface or safety property, read the README, handoff, [architecture decision](docs/ADR-002.md), [protocols](docs/PROTOCOLS.md), [Forge notes](docs/FORGE.md), [qualification record](docs/QUALIFICATION.md), and [security boundaries](SECURITY.md).
- Define operation semantics in `contracts/v1/openapi.json` first. Implement them in the shared Rust dispatcher (`src/lib.rs`), validation (`src/contract.rs`), and backend planner (`src/planner.rs`). Keep the CLI, HTTP, and MCP adapters thin. CLI `--dry-run` is preflight, not an operation result. MCP discovery may hide writers disallowed by session policy; the dispatcher must still enforce that policy on direct calls.
- Use the pinned Forge generation procedure and executable probes; never hand-edit `generated/`. If the pinned archive is unavailable, report the blocker. Generated TS/MCP/Cap’n Web artifacts do not implement the running Rust adapters.
- Keep CLI and MCP usable without Node, an account, a listener, or a resident daemon. qpdf is the qualified structural backend, PDFium the standard content provider, and PDFOxide a separate preview. Keep unsupported operations explicit in discovery and docs.

## Preserve inputs, fidelity, and credentials

- Enforce workspace-relative requests, trusted directories, distinct destinations, and input-alias checks in `src/filesystem.rs`. Keep encrypted, signed, form/XFA and preservation-unknown transforms fail-closed except where the operation has a tested policy. Distinguish visual marks from certificate signatures.
- Treat quality, size, and replacement as separate choices. A byte target never overrides explicit lossless mode. Test strict misses against an existing distinct destination and verify it and the input are unchanged.
- Keep credentials out of argv, environment, logs and errors. Retain bounded subprocesses, private staging and cancellation; do not invoke a shell for backend work. HTTP is authenticated and loopback-only. Use OS/container isolation for network denial, not claims about the Rust process.
- Treat fixtures, examples, history and CI output as public. Use synthetic or properly licensed fixtures, never private PDFs, identifying filenames, personal paths, private prompts or customer workflows.

## Verify the changed behavior

For runtime changes, run these checks before merge. Use qpdf **12.4.2**, Poppler, PDFium and the Python test dependencies. In a prepared orb, source `build/work/orb-env.sh` to use its pinned paths.

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
python3 tests/integration.py
```

- For a shared behavior, compare CLI/HTTP/MCP envelopes and verify the artifact (or absence of a write), not just exit status. For documentation examples, exercise the documented command against a synthetic input. Live [agent evaluations](docs/AGENT-EVALS.md) answer specific usability questions; they are not release gates.
- Contract changes also need [Forge regeneration and probes](docs/FORGE.md). For packaging changes, follow [BUILD.md](docs/BUILD.md) and smoke-test the actual package on its target; distinguish local rehearsal from published artifacts.

## Prepare and record releases

- Keep the product version in Cargo distinct from the v1 contract version. After publication, update the README, changelog, handoff and qualification record from the release artifacts, not from a planned tag. Keep historical results dated.
- Keep generic author and committer metadata (`anPDF Contributors <contributors@anpdf.invalid>`); run the existing full-history privacy and gitleaks gates before publication. Do not weaken them or expose private metadata to satisfy local tooling.
- Preserve third-party licenses and notices. New AGPL or dual-licensed backends need explicit compatibility and distribution review. Review provenance, SBOMs, vulnerabilities, licenses and platform evidence before publishing. Report what actually ran, what remains unverified, and the commit if one exists. A development release is not production qualification.
