# Preliminary qualification evidence

Observed locally and on hosted GitHub Actions runners (`ubuntu-26.04` amd64 and
`macos-26` arm64). These checks establish a preliminary `0.1.0-alpha.1` vertical
slice and artifact publication; they do not establish a production V0 release.

| Check | Observed result |
|---|---|
| Canonical OpenAPI 3.0.3 | Independent openapi-spec-validator passed |
| Rust | Build, unit safety/schema/inspect/discovery tests, format and Clippy with warnings denied passed on Linux amd64 and macOS arm64 |
| Shared corpus | 49 cross-surface cases (inspect, merge, extract, split, remove-pages, compress) through CLI/HTTP/MCP passed with complete domain envelope equality on `ubuntu-26.04` and `macos-26` |
| Preservation/order | Every corpus input hash preserved; independent Poppler text order passed |
| HTTP | Loopback binding, bearer, Host/Origin, discovery and JSON error checks passed |
| MCP | Initialization, discovery, calls, error wrapping and stdout framing passed |
| qpdf | Exact 12.4.2 official archive matched published checksum; actual structural/transform commands ran on Linux amd64 and macOS arm64 |
| Forge | Official multi-platform 0.2.0 package (`b8d74e2`), three native generators, deterministic regeneration, and TS compilation passed; fidelity gaps documented |
| Edge fixtures | Embedded CJK text survived extraction; identical rendered pixels; synthetic signing and tampering independently verified; both refused by product |
| Offline | Native inspection passed in an isolated Linux network namespace |
| Process/filesystem | Timeout, capture overflow, sparse-file limit, symlink/alias rejection, atomic competing outputs and cleanup tests passed (`RLIMIT_AS` is Linux-only; XNU rejects it with `EINVAL`) |
| Native artifact | Auditable release binary and actual Syft SPDX SBOM produced; `macos-arm64` release tarball packaged and smoke-tested on `macos-26` |
| GitHub workflows | Hosted `ci.yml` (`ubuntu-26.04`, `macos-26`, `forge-output`, `privacy`) and `oci.yml` passed |
| OCI (`linux/amd64`) | Built on `ubuntu-26.04`, passed non-root/read-only/`--network=none` smoke test, exported OCI archive + minimal provenance + Syft SPDX SBOM, and published to GHCR |

Synthetic recipes contain no personal data or external documents. CJK testing uses
an exact checksum/revision-pinned OFL-1.1 font; the ephemeral test certificate/key
is generated in memory and never stored in source. A synthetic marker fixture is
clearly distinguished from real signing fixtures. Generated PDF manifests include
checksums, source recipes, licensing and expected counts. Visual checks are test
evidence, not a rendering/signing product feature.

Remaining release gates:

- Multi-arch `linux/arm64` OCI build/runtime verification, complete image vulnerability
  scanning, and formal OS/backend license sign-off.
- macOS memory-limit semantics (`RLIMIT_AS` is not enforced on XNU; `RLIMIT_CPU`,
  `RLIMIT_FSIZE`, and `RLIMIT_NOFILE` remain active).
- Reproducible redistributable Forge artifact and wider generator/schema regression
  qualification; generated schema fidelity remains insufficient for validation.
- Extended malformed/object/stream bombs, namespace mutation/race stress, HTTP
  disconnect cancellation, MCP cancellation negotiation with external clients,
  complex widget appearances and incremental/expired-signature variants.
- Cross-version golden PDFs and additional visual/page semantics; independent
  structural success does not establish every PDF's visual correctness.
- Full dependency vulnerability review and target-specific OS/backend license review.
  The native Rust license manifest inventories 128 components and retains legal notices;
  OS/backend/build-tool artifacts require separate review.

Check scripts remain in the repository. Raw execution logs and sensitive scan
particulars are not public-bound.
