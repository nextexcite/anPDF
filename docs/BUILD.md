# Preliminary builds and workflows

CI workflows run on `ubuntu-26.04` (Linux amd64) and `macos-26` (Apple Silicon arm64).
Release builds (`.github/workflows/oci.yml`) additionally build OCI natively on `ubuntu-26.04`
(`linux/amd64`) and `ubuntu-26.04-arm` (`linux/arm64`). `.github/workflows/oci.yml` triggers on `v*`
tag pushes and `workflow_dispatch`, reading the authoritative version from `Cargo.toml` (and asserting
tag alignment on tag pushes), so untrusted PRs never receive package or release write permissions.

Versioning convention: `< 0.5.0` tags use clean milestone versions (`0.2.0`, `v0.2.0`); SemVer prerelease identifiers (`-alpha.N`, `-rc.N`) start at `>= 0.5.0`.

## Default content engine

Standard native, Homebrew, and OCI releases use PDFium for `extractText` and
`renderPage`; qpdf remains the structural engine. PDFOxide ships separately under
its own preview tag. `cargo build --locked` includes PDFium support;
`cargo build --locked --no-default-features` builds the structural-only variant.
Source builds need `python3 build/get-pdfium.py build/work/pdfium` and either
`ANPDF_PDFIUM_LIB` set to its printed path or the library installed beside the binary.

Library precedence is `--pdfium-lib` > `ANPDF_PDFIUM_LIB` >
`<canonical-executable-dir>/../lib/anpdf/libpdfium.{so,dylib}` >
`<canonical-executable-dir>/libpdfium.{so,dylib}`. Missing libraries leave content
operations unavailable. The native tarball contains `anpdf`, `libpdfium.dylib`,
and `PDFIUM_LICENSES/` (upstream LICENSE plus licenses directory). Homebrew installs
the dylib in `lib/anpdf/` and notices under its documentation directory. Its formula
URL and SHA-256 remain release-managed: bump both to the newly published tarball
before distributing this formula change; the old v0.2.0 tarball lacks PDFium.
OCI installs `/usr/local/lib/anpdf/libpdfium.so`, sets `ANPDF_PDFIUM_LIB`, and
carries `/usr/share/licenses/pdfium/`. Binary acquisition is SHA-256 pinned for
Linux x64/arm64 and macOS arm64; the dynamic library is not statically linked.

## Published development artifacts

Each release run of `.github/workflows/oci.yml` builds, scans, smoke-tests, and publishes:

1. **Multi-arch `linux/amd64` + `linux/arm64` OCI index (GHCR):** `ghcr.io/<owner>/anpdf` with tags
   `<version>-<sha>`, `<version>`, and `latest`, minimal BuildKit provenance, per-arch
   Syft SPDX image SBOMs (`anpdf-linux-{amd64,arm64}.spdx.json`), Grype vulnerability reports
   (`anpdf-linux-{amd64,arm64}.grype.json`), license/crate policy audits
   (`oci-audit-report-linux-{amd64,arm64}.json`), and recorded digest (`oci-digest.txt`). Each
   architecture passes the non-root, read-only, `--network=none` smoke test AND `build/oci-audit.py`
   (disallowing AGPL, checking uninventoried Rust crates, and blocking fixable High/Critical CVEs)
   before push by digest; `image-index` then creates and smoke-tests the unified OCI index (`DOCKER_BUILD_RECORD_UPLOAD=false`
   keeps build-record bundles off the release).
2. **`macos/arm64` native binary (GitHub Release + Homebrew formula `Formula/anpdf.rb`):**
   `anpdf-<version>-macos-arm64.tar.gz` and `SHA256SUMS-macos-arm64.txt`, built with
   pinned Rust `1.100.0` and `cargo-auditable 0.7.7`, smoke-tested against pinned qpdf `12.4.2`,
   and attached to the `v<version>` GitHub release alongside the OCI SBOMs, scan reports, and digests.

## OCI recipe and local reproduction

OCI builder/runtime images are pinned by index digest in `containers/Dockerfile`.
Both tags were resolved against the official Docker registry. Debian package
resolution uses the signed `20261118T000000Z` snapshot (`trixie`, `trixie-updates`, and
`trixie-security`). Exact qpdf 12.4.2 source is SHA-256 pinned and built with OpenSSL,
avoiding the extra crypto libraries bundled with upstream development/test binary archives.
Node, Forge, Rust and compilers are absent from the runtime stage. Rust dependency metadata
is embedded with pinned `cargo-auditable 0.7.7` for actual binary SBOM discovery. Runtime
images carry license notices and retain distribution package copyright records.

```sh
docker build -f containers/Dockerfile -t anpdf:0.2.0-local .
sh containers/smoke.sh anpdf:0.2.0-local
```

The smoke test uses numeric non-root UID `65532`, read-only rootfs, `--network=none`,
`no-new-privileges`, dropped capabilities, bounded tmpfs and resource limits, with separate
read-only input and writable temporary output mounts.

## CI matrix and pinned GitHub Actions

The native CI matrix (`.github/workflows/ci.yml`) runs:
- `ubuntu-26.04` (Linux amd64, required)
- `macos-26` (macOS arm64, experimental)
- `forge-output` (`ubuntu-26.04`) and `privacy` (`ubuntu-26.04`)

An additional Ubuntu native leg verifies `--no-default-features`. The PDFium
Ubuntu/macOS matrix tests default builds with bundled-library discovery and CJK
fixtures. `oci-smoke` builds and loads linux/amd64 without pushing, then exercises
merge, PDFium text, and PNG rendering with the restricted runtime flags.

Poppler and upstream qpdf binary archives are test tools, not runtime image dependencies.
OpenAPI validation and optional signed/CJK fixtures can use pinned Python packages from
`build/test-requirements.txt`.

Current action releases were checked through each official upstream latest release
and tag API on 2026-11-19. Full tag-object commit SHAs are pinned with version comments:

| Action | Verified release |
|---|---|
| [actions/checkout](https://github.com/actions/checkout/releases/tag/v7.0.1) | v7.0.1 |
| [actions/setup-node](https://github.com/actions/setup-node/releases/tag/v7.1.0) | v7.1.0 |
| [actions/upload-artifact](https://github.com/actions/upload-artifact/releases/tag/v7.0.2) | v7.0.2 |
| [actions/download-artifact](https://github.com/actions/download-artifact/releases/tag/v8.0.2) | v8.0.2 |
| [docker/setup-buildx-action](https://github.com/docker/setup-buildx-action/releases/tag/v4.4.1) | v4.4.1 |
| [docker/login-action](https://github.com/docker/login-action/releases/tag/v4.8.0) | v4.8.0 |
| [docker/build-push-action](https://github.com/docker/build-push-action/releases/tag/v7.4.0) | v7.4.0 |

Only minimum read permissions are requested at workflow top level; `packages: write` and
`contents: write` are scoped strictly to the manual publish jobs in `oci.yml`. Checkout
credentials are not retained. Generated-code checks use a locked Node build dependency
tree. Gitleaks and Syft qualification binaries are checksum pinned.
