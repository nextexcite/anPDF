# anPDF maintainer handoff

## Current release candidate

[v0.4.4](https://github.com/nextexcite/anPDF/releases/tag/v0.4.4) shipped on 2026-11-24 as a development milestone. The current working release line is **0.5.0**, owner-approved after the pragmatic qualification gates; it is not published until the release workflow and artifact verification complete.

- **0.5.0 feature scope:** `reorderPages` writes every input page exactly once in a caller-specified order across CLI, HTTP, and MCP. Extract remains an ascending subset. Non-identity reordering of tagged PDFs fails closed because the logical reading-order tree cannot yet be rewritten safely. Top-level help groups the existing flat commands by task; nested action-family syntax is intentionally absent.
- **0.5.0 qualification scope:** deterministic filesystem-race rejection, MCP cancellation/EOF cleanup, raw HTTP disconnect cancellation, malformed-input and compression-bomb cases, macOS RSS-limit coverage, official TypeScript MCP client probing, pinned-backend structural goldens, and native Linux arm64 qualification are engineering gates. Forms, visual/certificate signing, OCR, existing-text replacement, and remote multi-user hosting remain deferred.
- **Release boundary:** record exact workflow runs, runner labels, digests, checksums, license/advisory artifacts and remaining residuals in [QUALIFICATION.md](QUALIFICATION.md). Do not claim production qualification, vulnerability-free status, formal legal sign-off, or strict Forge redistribution beyond the documented evidence.

## Recommended next work

1. **Finish only the active 0.5.0 release evidence chain.** Do not start new feature work while release verification is open. Treat qualification artifacts as release-specific evidence, not generic support claims.
2. **Keep further compression work case-driven.** The full integration corpus already checks a traced-only page beside a mixed page with live text and a link. Do not repeat it as a new study. If scan legibility becomes a priority, use one synthetic degraded scan and its clean control; record size, elapsed time and small-glyph readability at the same render scale in the [case log](COMPRESSION-TECH-REVIEW.md#4-case-log). Finish with an adopt, reject, or defer decision rather than another model matrix.

## Deferred beyond 0.5.0

- **Compression:** degraded scans, small glyphs, and monochrome/CCITT inputs remain unmeasured classes. Use the [case template](COMPRESSION-TECH-REVIEW.md#5-case-template-copy-per-new-class) and [benchmark harness](BENCHMARK.md) when a case motivates a change. Compression capability discovery still does not predict which techniques will run on a document.
- **Qualification residuals:** formal legal sign-off, upstream Forge schema-fidelity breadth, and any support claim beyond the exact recorded runner/backend matrix remain outside engineering evidence. See [qualification](QUALIFICATION.md) before making stronger support claims.
- **Later features:** repair depth, form filling, visual and certificate signatures, OCR, existing-text replacement, and remote multi-user hosting each need a preservation or security design. None is promised by the current release.

Keep release evidence in `QUALIFICATION.md`, versioned changes in `CHANGELOG.md`, operation behavior in `PROTOCOLS.md`, and only current decisions and priorities here.
