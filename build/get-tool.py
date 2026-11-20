"""Pinned public qualification tools (Linux amd64/arm64). No credentials."""
import hashlib, io, pathlib, platform, sys, tarfile, urllib.request

ARCH = "arm64" if platform.machine().lower() in ("aarch64", "arm64") else "amd64"
pins = {
    ("gitleaks", "amd64"): ("gitleaks/gitleaks", "v8.30.1", "gitleaks_8.30.1_linux_x64.tar.gz", "551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb"),
    ("syft", "amd64"): ("anchore/syft", "v1.57.2", "syft_1.57.2_linux_amd64.tar.gz", "ad92bd3ec1b94a3df01d4d0aec199afa9c42b72e86da436ff5f837914d32706b"),
    ("syft", "arm64"): ("anchore/syft", "v1.57.2", "syft_1.57.2_linux_arm64.tar.gz", "8efe605309c98798e1ee0cffdcf8c05ca26819b4cd44c0b03650f1e963e9bdb6"),
    ("grype", "amd64"): ("anchore/grype", "v0.122.1", "grype_0.122.1_linux_amd64.tar.gz", "f1ba7e2917ee1395618de20a98da0aa7c0fb1788927797651047c50502a37538"),
    ("grype", "arm64"): ("anchore/grype", "v0.122.1", "grype_0.122.1_linux_arm64.tar.gz", "819e5be3d9516432fedb7b6b56df7306c3a16607606dfc05826eefbb00e471da"),
}
repo, version, name, digest = pins[(sys.argv[1], ARCH)]
out = pathlib.Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)
data = urllib.request.urlopen(f"https://github.com/{repo}/releases/download/{version}/{name}", timeout=120).read()
assert hashlib.sha256(data).hexdigest() == digest
with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
    archive.extractall(out, filter="data")
