# anPDF maintainer handoff

## What shipped

[v0.4.4](https://github.com/nextexcite/anPDF/releases/tag/v0.4.4) shipped on 2026-11-24 as a development milestone. All ten [operations](../README.md#try-it-on-a-document) share the Rust dispatcher across CLI, MCP stdio, and authenticated loopback HTTP. Standard packages ship PDFium for embedded text and PNG output; qpdf owns structural operations. PDFOxide remains a separate preview.

- **Compression:** explicit lossless mode remains lossless even with a target. A strict miss reports the measured size without publishing. The legacy target-only request can still permit loss. See [compression semantics](PROTOCOLS.md#preservation-and-compression).
- **Agent use:** the CLI has shorthand and JSON interfaces; MCP can restrict destination, writer, and bytes. Its tool list reflects writer policy, not a backend health check. The [agent skill](../.agents/skills/processing-pdfs-with-anpdf/SKILL.md) is for using these tools; [AGENTS.md](../AGENTS.md) is for developing them.
- **Evidence:** the installed Mac password round-trip and OCI package gates passed. Both Linux architectures, the published Mac tarball, and the Homebrew checksum were verified; see [release evidence](QUALIFICATION.md#044). The [wording pilot](AGENT-EVALS.md#compare-size-miss-wording) is complete. It measures compression decisions, not general agent reliability. Neither item is an outstanding release task.

## Recommended next work

1. **Propose page reordering as the next user-facing operation.** Extract keeps an ascending subset; it cannot move the last page to the front. Start with one input, one distinct output, and a full permutation of its pages. Acceptance: `3,1,2` produces those page markers in order across CLI/HTTP/MCP; duplicates, missing pages, or unmet preservation requirements leave the destination untouched. Keep extract's contract unchanged. This is proposed, not implemented; stay on the 0.4.x line unless the owner approves a version-line change.
2. **Keep further compression work case-driven.** The full integration corpus already checks a traced-only page beside a mixed page with live text and a link. Do not repeat it as a new study. If scan legibility becomes a priority, use one synthetic degraded scan and its clean control; record size, elapsed time and small-glyph readability at the same render scale in the [case log](COMPRESSION-TECH-REVIEW.md#4-case-log). Finish with an adopt, reject, or defer decision rather than another model matrix.

## Deferred, not blockers for the shipped milestone

- **Compression:** degraded scans, small glyphs, and monochrome/CCITT inputs remain unmeasured classes. Use the [case template](COMPRESSION-TECH-REVIEW.md#5-case-template-copy-per-new-class) and [benchmark harness](BENCHMARK.md) when a case motivates a change. Compression capability discovery still does not predict which techniques will run on a document.
- **Qualification:** full native Linux arm64 corpus (distinct from the passing arm64 image smoke), macOS resource-limit coverage, malformed-input stress, external MCP lifecycle/cancellation and HTTP conformance, [Forge fidelity and redistribution](FORGE.md), and formal dependency/OS license review remain open. See [qualification](QUALIFICATION.md) before making stronger support claims.
- **Later features:** repair depth, form filling, visual and certificate signatures, OCR, existing-text replacement, and remote multi-user hosting each need a preservation or security design. None is promised by the current release.

Keep release evidence in `QUALIFICATION.md`, versioned changes in `CHANGELOG.md`, operation behavior in `PROTOCOLS.md`, and only current decisions and priorities here.
