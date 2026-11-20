# Preliminary builds and workflows

CI workflows run on `ubuntu-26.04` (Linux amd64) and `macos-26` (Apple Silicon arm64).
Release builds (`.github/workflows/oci.yml`) additionally build OCI natively on `ubuntu-26.04`
(`linux/amd64`) and `ubuntu-26.04-arm` (`linux/arm64`). `.github/workflows/oci.yml` triggers on `v*`
tag pushes except PDFOxide preview tags, and on `workflow_dispatch`, reading the authoritative version from `Cargo.toml` (and asserting
tag alignment on tag pushes), so untrusted PRs never receive package or release write permissions.

Versioning convention: `< 0.5.0` tags use clean milestone versions (`0.2.0`, `v0.2.0`); SemVer prerelease identifiers (`-alpha.N`, `-rc.N`) start at `>= 0.5.0`.

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

## PDFOxide preview releases

After the preview workflow is merged, tag the intended commit on `main` as
`v<Cargo.toml version>-pdfoxide-preview.<number>` (for example, `v0.2.0-pdfoxide-preview.1`)
and push that tag. Only `.github/workflows/pdfoxide-preview.yml` publishes for these tags.
The workflow builds Linux amd64 with `--features pdfoxide`, smoke-tests extraction and rendering
under the same restricted runtime settings as the main build, and audits the image before publishing.

The preview uses `ghcr.io/<owner>/anpdf-pdfoxide:<tag>` with no `latest` tag and creates
a separate GitHub prerelease. It includes `anpdf-<tag>-linux-amd64.image.tar.gz`, an SPDX
SBOM, a Grype report, a license/crate audit, and checksums. The image carries the feature
dependency inventory and notices. No main image tags, native artifacts, or Homebrew
formula are updated by the preview workflow.

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
