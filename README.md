# anPDF

**Agent-native PDF tools, without the heavyweight editor.**

anPDF is an open-source, local-first toolkit for reliable PDF operations through a **CLI, HTTP API, and MCP tools**. It is designed for developers, desktop automation, self-hosted services, and AI agents that need to work with documents programmatically—not drive a GUI or hand files to a hosted PDF service.

The goal is to **own the interface and the workflow**, while reusing proven PDF engines behind replaceable adapters. anPDF is an independent project, not a fork or wrapper around another PDF suite.

## Design principles

- **Agent-first, contract-first.** A versioned OpenAPI contract defines operations, inputs, outputs, errors, and capability discovery across CLI/API/MCP. Structured JSON is the default.
- **Local by default.** One-shot commands and stdio MCP need no daemon or account. Remote hosting is a deployment option, not a dependency.
- **Portable, composable, open.** A small Rust runtime, replaceable backends, and a path toward macOS/Linux, OCI containers, desktop integrations, and cloud workloads.
- **Safe before clever.** Preserve inputs, fail explicitly on unsupported documents, and test output fidelity. No silent rasterization, overwrite, or claims of signature validity.
- **Build with the ecosystem.** Adopt [Cloudflare Forge](https://github.com/cloudflare/forge) for contract-based generation as it matures, while qualifying generated output rather than trusting it blindly.

## What works today

**Version:** `0.1.0-alpha.1` — preliminary, **not production-qualified**.

| Operation | CLI | HTTP | MCP |
| --- | --- | --- | --- |
| Inspect / structurally validate a PDF | `inspect` / `validate` | `POST /v1/inspect` | `inspectPdf` |
| Merge PDFs in input order | `merge` | `POST /v1/merge` | `mergePdf` |
| Extract selected pages into a new PDF | `extract` | `POST /v1/extract` | `extractPdf` |
| Split a PDF into multiple page-subset outputs | `split` | `POST /v1/split` | `splitPdf` |

All three surfaces use the same validated operation contract. The Rust runtime uses **qpdf 12.4.2** for PDF processing; pinned **Forge 0.2.0** is a build-time generation and qualification dependency, not a product runtime. Hosted CI qualifies `ubuntu-26.04` (Linux amd64) and `macos-26` (macOS arm64); preliminary publication targets the `linux/amd64` OCI image (GHCR) and the native `macos/arm64` binary (GitHub prerelease). See [qualification and open gates](docs/QUALIFICATION.md) and [OCI/build](docs/BUILD.md).

Transforms currently **reject** encrypted, signed, form/XFA-bearing, or preservation-unknown PDFs. Inspection is structural—not visual, cryptographic, PDF/A, or malware certification.

## Try it locally

Requires Rust 1.100.0, Python 3, and qpdf 12.4.2 (auto-discovered from standard paths or trusted `PATH` directories, or overridden via `ANPDF_QPDF` / `--qpdf`). On macOS with Homebrew: `brew install qpdf`.

```sh
cargo build --locked
python3 fixtures/generate.py build/work/fixtures

./target/debug/anpdf operations
./target/debug/anpdf inspect --root build/work/fixtures --request-json '{"input":{"path":"three.pdf"}}'
./target/debug/anpdf merge --root build/work/fixtures --request-json '{"inputs":[{"path":"three.pdf"},{"path":"one.pdf"}],"output":{"path":"merged.pdf"}}'
```

Use `./target/debug/anpdf mcp --root build/work/fixtures` for local stdio MCP. HTTP is **optional, loopback-only, and bearer-authenticated**; see [protocols and setup](docs/PROTOCOLS.md). Paths in requests are relative to the configured workspace root. Commands emit machine-readable JSON; errors have stable codes.

## Roadmap

1. **Foundation and portability:** qualify reproducible Forge generation, multi-platform binaries, OCI builds, security limits, and cross-surface contract tests.
2. **Everyday PDF operations:** page deletion/reordering, better structural repair, and **quality-aware compression** with measurable fidelity.
3. **Documents and signatures:** inspect/fill AcroForms, add visual signatures, and separately support certificate-based digital signing/validation with preservation checks.
4. **Richer capabilities and integration:** OCR, extraction, and cautious existing-text editing where fidelity can be demonstrated; optional backends and agent/desktop/cloud integration.

These are **intentions, not shipped features**. The guiding rule is to deliver a small set of trustworthy operations before expanding the surface area.

## Project docs

[Architecture](docs/ADR-002.md) · [Forge integration](docs/FORGE.md) · [Protocols](docs/PROTOCOLS.md) · [Qualification and open gates](docs/QUALIFICATION.md) · [Security](SECURITY.md) · [OCI/build](docs/BUILD.md) · [Agent and contributor guidance](AGENTS.md)

Licensed under [Apache-2.0](LICENSE). Third-party notices and dependency licenses are tracked separately.
