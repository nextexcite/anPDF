# Preliminary builds and workflows

CI and release workflows run on GitHub-hosted runners (`ubuntu-26.04` for Linux amd64 and
`macos-26` for Apple Silicon arm64). The OCI and release publication workflow
(`.github/workflows/oci.yml`) is manual-only (`workflow_dispatch`) so untrusted PRs never
receive package or release write permissions.

## Published preliminary artifacts (until further notice)

Each manual run of `.github/workflows/oci.yml` builds, smoke-tests, and publishes:

1. **`linux/amd64` OCI image (GHCR):** `ghcr.io/<owner>/anpdf` with tags
   `0.1.0-alpha.1-<sha>`, `0.1.0-alpha.1`, and `latest`, minimal BuildKit provenance, and
   a Syft SPDX image SBOM (`anpdf-linux-amd64.spdx.json`) + recorded digest (`oci-digest.txt`).
   Push only happens after the non-root, read-only, `--network=none` smoke test passes.
2. **`macos/arm64` native binary (GitHub Prerelease + artifact):**
   `anpdf-0.1.0-alpha.1-macos-arm64.tar.gz` and `SHA256SUMS-macos-arm64.txt`, built with
   pinned Rust `1.100.0` and `cargo-auditable 0.7.7`, smoke-tested against pinned qpdf `12.4.2`,
   and attached to the `v0.1.0-alpha.1` GitHub prerelease alongside the OCI SBOM and digest.

`linux/arm64` OCI images are not built or published in this round.

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
docker build -f containers/Dockerfile -t anpdf:0.1.0-alpha.1-local .
sh containers/smoke.sh anpdf:0.1.0-alpha.1-local
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
