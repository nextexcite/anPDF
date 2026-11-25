# anPDF

Inspect, assemble, compress, and protect PDFs locally from a terminal, script, or AI agent. The CLI and MCP server need no account or background service. [v0.5.0](https://github.com/nextexcite/anPDF/releases/tag/v0.5.0) is a published development milestone, not a production qualification.

## Install

**Apple Silicon:** Homebrew installs anPDF, qpdf, and shell completions. If you already installed it, use `brew update && brew upgrade anpdf`.

```sh
brew tap nextexcite/anpdf
brew trust --formula nextexcite/anpdf/anpdf
brew install anpdf
anpdf --version
```

**Linux amd64 or arm64:** the OCI image includes qpdf and PDFium. Try discovery without mounting any files:

```sh
docker run --rm --network=none --read-only ghcr.io/nextexcite/anpdf:0.5.0 operations
```

For input/output mounts and non-root permissions, see the [container instructions](docs/BUILD.md). For a source build, use `cargo build --locked` with the [pinned Rust toolchain](rust-toolchain.toml), qpdf 12.x ≥12.4.2 (12.4.2 qualified), and [PDFium setup](docs/BUILD.md#default-content-engine) for text extraction and rendering.

## Try it on a document

From a private working directory containing your own `report.pdf` (at least three pages):

```sh
anpdf inspect report.pdf
anpdf extract report.pdf 1,3 -o selected.pdf
anpdf reorder report.pdf 3,1,2 -o ordered.pdf
anpdf compress report.pdf -o compact.pdf --mode lossless --max-bytes 3MB --on-target-miss error
```

The last command writes a new file **only if lossless compression meets the limit**. A strict miss reports the measured size and leaves the destination untouched. Without `--on-target-miss error`, the target is best effort: check `targetMet` and warnings. Choose the mode explicitly: a bare byte target can enable lossy processing for v1 compatibility. `--replace` allows replacing a **distinct output**, never the input; it does not change quality. See `anpdf compress --help` for lossy and layered modes.

| I want to… | CLI command | MCP operation |
| --- | --- | --- |
| Check structure; optionally decode streams | `inspect` (`validate` is an alias; add `--deep-check` for streams) | `inspectPdf` |
| Join documents in input order | `merge` (first input is primary) | `mergePdf` |
| Keep selected pages | `extract` | `extractPdf` |
| Write multiple named page subsets | `split` | `splitPdf` |
| Drop selected pages | `remove-pages` | `removePages` |
| Write every page in a new order | `reorder` | `reorderPages` |
| Reduce file size | `compress` | `compressPdf` |
| Read embedded text, not OCR | `extract-text` | `extractText` |
| Save a page as a PNG | `render-page` | `renderPage` |
| Write a password-protected copy | `encrypt` | `encryptPdf` |
| Write a plaintext copy with a known password | `decrypt` | `decryptPdf` |

Page selections are 1-based. Extract, remove-pages, extract-text, and each split part use ascending unique selections; split parts may overlap. Reorder is different: its list is the output order, a full permutation containing every source page exactly once, written to a distinct output. The ascending order is valid. For page operations, check the `preservation` report or require the aspects you need; for example, merging does not combine every input's bookmarks. For password protection, use the hidden terminal prompt or a private password file (`chmod 600`), never a literal password in arguments. An intentionally empty open password does **not** keep contents confidential. See [operation details and limitations](docs/PROTOCOLS.md).

**Not implemented:** form filling, signing, OCR, and existing-text replacement. Inspection is not a signature-trust, PDF/A, or malware verdict.

## Scripts and agents

Use `anpdf operations` to discover operation IDs and `anpdf schema compressPdf` for a specific request. Operation commands show a summary on a terminal; use `--json` for an envelope. JSON requests use paths relative to `--root`:

```sh
anpdf inspect --root . --request-json '{"input":{"path":"report.pdf"}}' --json
```

Start local **MCP over stdio** with `anpdf mcp --root .` in a dedicated workspace. To allow just one new compression output, use `anpdf mcp --root . --output-only compact.pdf --output-operation compressPdf --output-byte-ceiling 3000000`. This limits what **that server** can publish, not what other tools can write. Use `tools/list` for the current session; use the [anPDF skill](.agents/skills/processing-pdfs-with-anpdf/SKILL.md) when giving an agent document tasks.

For **authenticated loopback HTTP**, see [protocol setup](docs/PROTOCOLS.md) and the [OpenAPI contract](contracts/v1/openapi.json). Remote multi-user hosting is not supported.

## Development

[Contributor instructions](AGENTS.md) · [Current progress and next work](docs/HANDOFF.md) · [Release evidence](docs/QUALIFICATION.md) · [Build and release checklist](docs/BUILD.md) · [Changelog](CHANGELOG.md) · [Security](SECURITY.md)

Licensed under [Apache-2.0](LICENSE). Third-party notices are distributed separately.
