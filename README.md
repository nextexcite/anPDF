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

**Version:** `0.2.0` — early development milestone (**not production-qualified**).

Versioning convention:
- **`< 0.5.0`** (`0.2.0`, `0.3.0`, …): development milestones with no SemVer prerelease suffix (`v0.2.0`).
- **`>= 0.5.0`** (`0.5.0-alpha.1`, `1.0.0-rc.1`, …): pre-releases carrying standard prerelease tags ahead of `1.0.0`.

| Operation | CLI | HTTP | MCP |
| --- | --- | --- | --- |
| Inspect / structurally validate a PDF | `inspect` / `validate` | `POST /v1/inspect` | `inspectPdf` |
| Merge PDFs in input order | `merge` | `POST /v1/merge` | `mergePdf` |
| Extract selected pages into a new PDF | `extract` | `POST /v1/extract` | `extractPdf` |
| Split a PDF into multiple page-subset outputs | `split` | `POST /v1/split` | `splitPdf` |
| Remove selected pages and keep the rest | `remove-pages` | `POST /v1/remove-pages` | `removePages` |
| Compress streams / optimize images | `compress` | `POST /v1/compress` | `compressPdf` |

All three surfaces use the same validated operation contract. The Rust runtime uses **qpdf 12.4.2** for PDF processing; pinned **Forge 0.2.0** is a build-time generation and qualification dependency, not a product runtime. Release builds publish multi-arch **`linux/amd64` + `linux/arm64` OCI images** (GHCR) and **`macos/arm64` native binaries** (GitHub Releases + Homebrew tap from this repository).

The development contract also exposes [embedded text extraction and PNG rendering](docs/PROTOCOLS.md#optional-text-and-rendering-engines). These operations require optional engine providers; the qpdf-only build reports them as unavailable.

Transforms currently **reject** encrypted, signed, form/XFA-bearing, or preservation-unknown PDFs. Inspection is structural—not visual, cryptographic, PDF/A, or malware certification.

## Basic usage

### macOS (Apple Silicon)

Install via Homebrew directly from this repository (installs `qpdf` and shell completions for `bash`, `zsh`, and `fish` automatically):

```sh
brew tap nextexcite/anpdf https://github.com/nextexcite/anPDF
brew install anpdf
anpdf --version
```

Or download `anpdf-0.2.0-macos-arm64.tar.gz` from [GitHub Releases](https://github.com/nextexcite/anPDF/releases), verify against `SHA256SUMS-macos-arm64.txt`, and install `qpdf` (`brew install qpdf`).

### Linux (OCI container, `linux/amd64` & `linux/arm64`)

The container image bundles the `anpdf` binary and `qpdf 12.4.2` on a minimal Debian 13 runtime and runs as non-root UID `65532`:

```sh
docker pull ghcr.io/nextexcite/anpdf:0.2.0
mkdir -p out
docker run --rm --read-only --network=none \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  -v "$PWD:/data/in:ro" -v "$PWD/out:/data/out" \
  ghcr.io/nextexcite/anpdf:0.2.0 \
  inspect --root /data --request-json '{"input":{"path":"in/sample.pdf"}}'
```

### Everyday commands, shell completions, MCP & HTTP

Paths in requests are always relative to `--root`. Commands emit JSON on stdout and canonical error envelopes on stderr:

```sh
anpdf operations
anpdf inspect --root ./docs --request-json '{"input":{"path":"a.pdf"}}'
anpdf merge --root ./docs --request-json '{"inputs":[{"path":"a.pdf"},{"path":"b.pdf"}],"output":{"path":"merged.pdf"}}'
anpdf extract --root ./docs --request-json '{"input":{"path":"a.pdf"},"pages":[1,3],"output":{"path":"subset.pdf"}}'
anpdf remove-pages --root ./docs --request-json '{"input":{"path":"a.pdf"},"pages":[2],"output":{"path":"trimmed.pdf"}}'
anpdf compress --root ./docs --request-json '{"input":{"path":"a.pdf"},"output":{"path":"compact.pdf"},"mode":"lossless"}'
```

- **Shell completions** (`bash`, `zsh`, `fish`):
  ```sh
  anpdf completions zsh > "${fpath[1]}/_anpdf"   # zsh
  anpdf completions bash > ~/.local/share/bash-completion/completions/anpdf
  anpdf completions fish > ~/.config/fish/completions/anpdf.fish
  ```
- **Local stdio MCP**: `anpdf mcp --root ./docs`
- **Loopback HTTP API** (`127.0.0.1` only, requires bearer token file with >= 32 ASCII graphic chars): `anpdf serve --root ./docs --token-file /path/to/token`

See [protocols and setup](docs/PROTOCOLS.md) for JSON schemas, error codes, and MCP initialization messages.

## Build from source

Requires **Rust 1.100.0**, **Python 3**, and **qpdf 12.4.2** on a standard/trusted path (or `ANPDF_QPDF` / `--qpdf`). The integration test suite also requires **Poppler** (`pdftotext`).

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked

# Generate synthetic PDFs and run the cross-surface suite (CLI, HTTP, MCP, Poppler)
ANPDF_QPDF="$(command -v qpdf)" python3 tests/integration.py
```

To build and verify the OCI image locally:

```sh
docker build -f containers/Dockerfile -t anpdf:0.2.0-local .
sh containers/smoke.sh anpdf:0.2.0-local
```

When modifying `contracts/v1/openapi.json`, regenerate `generated/` and run the Forge schema/TS checks documented in [docs/FORGE.md](docs/FORGE.md).

## Release & quality gates

Every commit and tagged release (`v*`) passes automated checks in [`.github/workflows/ci.yml`](.github/workflows/ci.yml) and [`.github/workflows/oci.yml`](.github/workflows/oci.yml):

- **Cross-surface contract parity:** OpenAPI 3.0.3 schema validation, Forge TS/MCP/Cap'n Web regeneration & probe checks, Rust safety/resource-guard unit tests, and 55+ CLI/HTTP/MCP envelope & Poppler text-order tests.
- **SBOM & license policy (`syft` + [`build/oci-audit.py`](build/oci-audit.py)):** Generates a per-arch SPDX JSON SBOM (`anpdf-linux-{amd64,arm64}.spdx.json`) from the `cargo-auditable` binary and runtime packages, reconciles every Rust crate against [`license-manifest.json`](license-manifest.json), and fails on any `AGPL` license or unexpected Debian package.
- **Vulnerability gate (`grype` + [`build/oci-audit.py`](build/oci-audit.py)):** Scans each arch OCI archive before registry push (`anpdf-linux-{amd64,arm64}.grype.json`) and blocks any **fixable High or Critical CVE**.
- **Container & binary smoke tests:** Runs [`containers/smoke.sh`](containers/smoke.sh) as non-root UID `65532` with `--read-only`, `--network=none`, and dropped capabilities both before pushing each architecture digest and after assembling the multi-arch index; verifies the `macos/arm64` tarball with pinned `qpdf 12.4.2`.
- **Privacy & secret hygiene:** `gitleaks` and [`build/privacy.py`](build/privacy.py) inspect Git history and tracked files for secrets, tokens, or personal data.

Remaining gaps before `>= 0.5.0` prereleases and `1.0.0` (extended malformed/fuzz stress, external MCP/HTTP client conformance, formal OS/backend license sign-off, Forge schema fidelity fixes) are tracked in [docs/QUALIFICATION.md](docs/QUALIFICATION.md).

## Roadmap

1. **Foundation and portability:** qualify reproducible Forge generation, multi-platform binaries, OCI builds, security limits, and cross-surface contract tests.
2. **Everyday PDF operations:** page reordering and better structural repair.
3. **Documents and signatures:** inspect/fill AcroForms, add visual signatures, and separately support certificate-based digital signing/validation with preservation checks.
4. **Richer capabilities and integration:** OCR, extraction, and cautious existing-text editing where fidelity can be demonstrated; optional backends and agent/desktop/cloud integration.

These are **intentions, not shipped features**. The guiding rule is to deliver a small set of trustworthy operations before expanding the surface area.

## Project docs

[Architecture](docs/ADR-002.md) · [Forge integration](docs/FORGE.md) · [Protocols](docs/PROTOCOLS.md) · [Qualification and open gates](docs/QUALIFICATION.md) · [Security](SECURITY.md) · [OCI/build](docs/BUILD.md) · [Agent and contributor guidance](AGENTS.md)

Licensed under [Apache-2.0](LICENSE). Third-party notices and dependency licenses are tracked separately.
