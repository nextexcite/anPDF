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

**Version:** `0.4.1` staged on `main` — early development milestone (**not production-qualified**).

Latest published build: [v0.4.0](https://github.com/nextexcite/anPDF/releases/tag/v0.4.0). See [CHANGELOG.md](CHANGELOG.md), the
[release checklist](docs/BUILD.md#release-checklist), and the [project handoff](docs/HANDOFF.md) for status, roadmap and next actions.

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
| Extract embedded text with PDF-space spans | `extract-text` | `POST /v1/extract-text` | `extractText` |
| Render a page as PNG | `render-page` | `POST /v1/render-page` | `renderPage` |
| Password-protect a PDF (AES-256, revision 6) | `encrypt` | `POST /v1/encrypt` | `encryptPdf` |
| Remove a PDF's password | `decrypt` | `POST /v1/decrypt` | `decryptPdf` |

All three surfaces use the same validated operation contract. The Rust runtime accepts **qpdf 12.x >=12.4.2**, with **12.4.2** the tested version; other accepted versions emit `BACKEND_VERSION_UNTESTED` once per response. `ANPDF_QPDF_EXACT=1` or `--qpdf-exact` requires exactly 12.4.2 (enabled in CI and OCI). Pinned **Forge 0.2.0** is a build-time generation and qualification dependency, not a product runtime. Release builds publish multi-arch **`linux/amd64` + `linux/arm64` OCI images** (GHCR) and **`macos/arm64` native binaries** (GitHub Releases + the [nextexcite/homebrew-anpdf](https://github.com/nextexcite/homebrew-anpdf) tap).

The development contract also exposes [embedded text extraction and PNG rendering](docs/PROTOCOLS.md#text-and-rendering-engine). Standard releases use bundled PDFium for these operations; qpdf remains the structural engine. A `--no-default-features` build reports content operations as unavailable.

The capability registry selects registered providers consistently across all three
surfaces. The [synthetic corpus and benchmark harness](docs/BENCHMARK.md) compare
external engines; benchmark scores are not product or production qualification.

Page transforms currently **reject** encrypted, signed, form/XFA-bearing, or preservation-unknown PDFs. `encrypt` and `decrypt` are the exceptions: they rewrite only protection — encrypt accepts form/XFA-bearing inputs, signatures stay rejected everywhere. Inspection is structural—not visual, cryptographic, PDF/A, or malware certification.

Passwords never travel through command arguments, environment variables, logs or error envelopes. On a terminal `encrypt`/`decrypt` prompt with echo disabled; scripts pass `--password-file <f>` / `--owner-password-file <f>` (a `chmod 600` file), `--request-file`, or piped stdin carrying `password` / `ownerPassword`. `--request-json` refuses literal password fields — argv is exposed to the OS at launch, so that refusal is a guardrail toward the safe channels, not retroactive secrecy — and HTTP/MCP requests carry them only in the body (the contract has no secret file-path fields). Encryption is standard AES-256 revision 6: the user password opens the document; the optional owner password (generated randomly when omitted) is the permissions password. An intentionally empty user password gives an owner-only document that opens without one, warned via `EMPTY_OPEN_PASSWORD`; an omitted or cancelled password fails closed.

Merge, extract, split and remove-pages retain the first input as the primary document. Each output reports `preservation` for `metadata`, `bookmarks`, `annotations` and `attachments`: `preserved`, `omitted` (including uncertainty), or `absent`. Optional `preserve` booleans require those aspects when present; an unmet requirement fails with `PRESERVATION_UNMET` before any output is published. Defaults are best effort, with one `PRESERVATION_OMITTED` warning naming omissions. Secondary merge metadata can be omitted, and secondary bookmarks/attachments are not merged. Inspection includes optional per-page `annotationCount`. See [preservation evidence and limitations](docs/PROTOCOLS.md#preservation-and-compression).

Compression keeps the validated original bytes if compressed output is not smaller, unless `allowLargerOutput:true` is requested. Required `outputSource` identifies `original` or `compressed`; reported hashes, sizes and savings describe the bytes actually written. `NO_SIZE_REDUCTION` names which was kept.

`compress` also accepts `maxOutputBytes`, `compressionLevel` (1–3) and `onTargetMiss` (`bestEffort` default, or `error`). A byte target selects from a DPI/JPEG-quality ladder — `image-downsample` decodes each placed image once, measures every rung, and applies the gentlest rung fitting the target through a sandboxed worker — bounded by the level's floor (2 ≈ 200dpi, 3 ≈ 120dpi; a bare target may descend to ~72dpi; level 1 stays lossless). Results report `techniques`, `targetBytes` and `targetMet`; an unmet target warns `SIZE_TARGET_UNMET` or fails 422.

## Basic usage

### macOS (Apple Silicon)

Install via Homebrew directly from this repository (installs `qpdf` and shell completions for `bash`, `zsh`, and `fish` automatically):

```sh
brew tap nextexcite/anpdf
brew trust --formula nextexcite/anpdf/anpdf
brew install anpdf
anpdf --version
```

Current Homebrew requires `brew trust --formula` for non-official tap formulae before install.

> [!NOTE]
> If you previously tapped this repository directly (`brew tap nextexcite/anpdf https://github.com/nextexcite/anPDF`), migrate once: `brew untap nextexcite/anpdf && brew tap nextexcite/anpdf` — installed `anpdf` stays installed.

Or download `anpdf-0.4.0-macos-arm64.tar.gz` from [GitHub Releases](https://github.com/nextexcite/anPDF/releases), verify against `SHA256SUMS-macos-arm64.txt`, and install `qpdf` (`brew install qpdf`).

### Linux (OCI container, `linux/amd64` & `linux/arm64`)

The container image bundles `anpdf`, `qpdf 12.4.2` and PDFium on a minimal Debian 13 runtime and runs as non-root UID `65532`. The v0.4.0 example applies after publication:

```sh
docker pull ghcr.io/nextexcite/anpdf:0.4.0
mkdir -p out
docker run --rm --read-only --network=none \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  -v "$PWD:/data/in:ro" -v "$PWD/out:/data/out" \
  ghcr.io/nextexcite/anpdf:0.4.0 \
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

Requires **Rust 1.100.0**, **Python 3**, and **qpdf 12.x >=12.4.2** on a standard/trusted path (or `ANPDF_QPDF` / `--qpdf`). Qualification uses exactly **12.4.2**. The integration test suite also requires **Poppler** (`pdftotext`) and `build/test-requirements.txt`.

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
docker build -f containers/Dockerfile -t anpdf:0.4.1-local .
sh containers/smoke.sh anpdf:0.4.1-local
```

When modifying `contracts/v1/openapi.json`, regenerate `generated/` and run the Forge schema/TS checks documented in [docs/FORGE.md](docs/FORGE.md).

## Default PDFium text and rendering engine

`cargo build --locked` includes the `pdfium-render 0.9.5` adapter for
`extract-text` and `render-page`. Standard native, Homebrew, and OCI releases bundle
the checksum-pinned PDFium `chromium/8165` dynamic library and its notices.
qpdf remains required for structural inspection and preservation gates.
PDFOxide ships separately under its own preview tag, not in standard releases.

Library precedence is `--pdfium-lib` > `ANPDF_PDFIUM_LIB` >
`<canonical-executable-dir>/../lib/anpdf/libpdfium.{so,dylib}` >
`<canonical-executable-dir>/libpdfium.{so,dylib}`. Only regular files are registered;
automatic discovery uses trusted directories. Explicit paths must be absolute.
No library means `BACKEND_UNAVAILABLE`; there is no fallback engine.
`anpdf operations` lists the registered engine; omit `engine` in requests to use it.
For source builds, run `python3 build/get-pdfium.py build/work/pdfium` and set
`ANPDF_PDFIUM_LIB` to the printed path, or copy the library beside the binary.
Build without PDFium using `cargo build --locked --no-default-features`.

```sh
anpdf extract-text --root ./docs --request-json '{"input":{"path":"sample.pdf"},"pages":[1],"engine":"pdfium"}'
anpdf render-page --root ./docs --request-json '{"input":{"path":"sample.pdf"},"page":1,"dpi":144,"output":{"path":"page.png"},"engine":"pdfium"}'
```

Native decoding runs only in a hidden subprocess with the qpdf process resource,
capture, cancellation and deadline controls. This is not an OS network sandbox.
Use only trusted library paths; the library itself is executable code.
Text runs use PDF-space bounds and PDFium reading order; this is not OCR.
Pixel dimensions round up from points × dpi / 72 and are checked before rendering.
The fetcher supports Linux x64/arm64 and macOS arm64. Upstream notices are retained
in every bundled artifact. Development releases remain subject to the documented
qualification gates; making PDFium the default is not a production-readiness claim.
Page reordering, forms, signatures, OCR and existing-text replacement remain unimplemented.

For the optional CJK integration case, fetch `build/get-test-font.py build/work/font`,
install `build/test-requirements.txt`, and set `ANPDF_TEST_FONT` to the absolute
`build/work/font/NotoSansSC.ttf` path alongside `ANPDF_PDFIUM_LIB`.

## Release & quality gates

Every commit and tagged release (`v*`) passes automated checks in [`.github/workflows/ci.yml`](.github/workflows/ci.yml) and [`.github/workflows/oci.yml`](.github/workflows/oci.yml):

- **Cross-surface contract parity:** OpenAPI 3.0.3 schema validation, Forge TS/MCP/Cap'n Web regeneration & probe checks, Rust safety/resource-guard unit tests, and 55+ CLI/HTTP/MCP envelope & Poppler text-order tests.
- **SBOM & license policy (`syft` + [`build/oci-audit.py`](build/oci-audit.py)):** Generates a per-arch SPDX JSON SBOM (`anpdf-linux-{amd64,arm64}.spdx.json`) from the `cargo-auditable` binary and runtime packages, reconciles every Rust crate against [`license-manifest.json`](license-manifest.json), and fails on any `AGPL` license or unexpected Debian package.
- **Vulnerability gate (`grype` + [`build/oci-audit.py`](build/oci-audit.py)):** Scans each arch OCI archive before registry push (`anpdf-linux-{amd64,arm64}.grype.json`) and blocks any **fixable High or Critical CVE**.
- **Container & binary smoke tests:** Runs [`containers/smoke.sh`](containers/smoke.sh) as non-root UID `65532` with `--read-only`, `--network=none`, and dropped capabilities both before pushing each architecture digest and after assembling the multi-arch index; verifies the `macos/arm64` tarball with pinned `qpdf 12.4.2`.
- **Privacy & secret hygiene:** `gitleaks` and [`build/privacy.py`](build/privacy.py) inspect Git history and tracked files for secrets, tokens, or personal data.

Remaining gaps before `>= 0.5.0` prereleases and `1.0.0` (extended malformed/fuzz stress, external MCP/HTTP client conformance, formal OS/backend license sign-off, Forge schema fidelity fixes) are tracked in [docs/QUALIFICATION.md](docs/QUALIFICATION.md).

## Roadmap

The working roadmap and prioritized queue live in [docs/HANDOFF.md](docs/HANDOFF.md). In short: consolidate the 0.4.x line, close qualification debt, then everyday operations (page reordering, repair depth), documents and signatures, then richer capabilities and integrations.

These are **intentions, not shipped features**. The guiding rule is to deliver a small set of trustworthy operations before expanding the surface area.

## Project docs

[Handoff — status, roadmap, next actions](docs/HANDOFF.md) · [Architecture](docs/ADR-002.md) · [Forge integration](docs/FORGE.md) · [Protocols](docs/PROTOCOLS.md) · [Qualification and open gates](docs/QUALIFICATION.md) · [Security](SECURITY.md) · [OCI/build](docs/BUILD.md) · [Compression technical review](docs/COMPRESSION-TECH-REVIEW.md) · [Agent and contributor guidance](AGENTS.md)

Licensed under [Apache-2.0](LICENSE). Third-party notices and dependency licenses are tracked separately.
