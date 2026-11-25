# anPDF roadmap

## Current release

[v0.5.0](https://github.com/nextexcite/anPDF/releases/tag/v0.5.0) shipped on 2026-11-25 as a development milestone. Tag `v0.5.0` points at merge commit `76ae8214e7a0cd4f897372cc8306d139ed977a48`; release workflow run [45629808514](https://github.com/nextexcite/anPDF/actions/runs/45629808514) passed exact-tree qualification, publication, published-digest smoke tests, and asset upload. The release is still a prerelease/development milestone, not production qualification.

- **0.5.0 feature scope:** `reorderPages` writes every input page exactly once in a caller-specified order across CLI, HTTP, and MCP. Extract remains an ascending subset. Non-identity reordering of tagged PDFs fails closed because the logical reading-order tree cannot yet be rewritten safely. Top-level help groups the existing flat commands by task; nested action-family syntax is intentionally absent.
- **0.5.0 completed qualification scope:** deterministic filesystem-race rejection, MCP cancellation/EOF cleanup, raw HTTP disconnect cancellation, malformed-input and compression-bomb cases, macOS RSS-limit coverage, official TypeScript MCP client probing, pinned-backend structural goldens, and hosted native Linux arm64 qualification. Exact artifact checksums, digests, scan counts, and local post-publication verification are recorded in [QUALIFICATION.md](QUALIFICATION.md).
- **Accepted residuals:** formal legal sign-off/copyleft source-offer review, upstream Forge schema-fidelity breadth, vulnerability remediation beyond the fixable High/Critical gate, broader cross-version backend comparison, and any support claim beyond the exact recorded runner/backend matrix remain outside engineering evidence. Forms, visual/certificate signing, OCR, existing-text replacement, and remote multi-user hosting remain deferred.

## Recommended next work

1. **Let the owner choose the next feature slice.** The 0.5.0 evidence chain is closed; do not treat deferred forms, signing, OCR, or hosting as automatically committed work.
2. **Keep further compression work case-driven.** The full integration corpus already checks a traced-only page beside a mixed page with live text and a link. Do not repeat it as a new study. If scan legibility becomes a priority, use one synthetic degraded scan and its clean control; record size, elapsed time and small-glyph readability at the same render scale in the [case log](COMPRESSION-TECH-REVIEW.md#4-case-log). Finish with an adopt, reject, or defer decision rather than another model matrix.

## Deferred beyond 0.5.0

- **Compression:** degraded scans, small glyphs, and monochrome/CCITT inputs remain unmeasured classes. Use the [case template](COMPRESSION-TECH-REVIEW.md#5-case-template-copy-per-new-class) and [benchmark harness](BENCHMARK.md) when a case motivates a change. Compression capability discovery still does not predict which techniques will run on a document.
- **Qualification residuals:** formal legal sign-off/copyleft source-offer review, upstream Forge schema-fidelity breadth, and any support claim beyond the exact recorded runner/backend matrix remain outside engineering evidence. See [qualification](QUALIFICATION.md) before making stronger support claims.
- **Later features:** repair depth, form filling, visual and certificate signatures, OCR, existing-text replacement, and remote multi-user hosting each need a preservation or security design. None is promised by the current release.

Keep release evidence in `QUALIFICATION.md`, versioned changes in `CHANGELOG.md`, operation behavior in `PROTOCOLS.md`, and only current decisions and priorities here.
