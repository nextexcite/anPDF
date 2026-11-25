"""Audit OCI SPDX SBOM and Grype vulnerability output against license and CVE policy."""
import argparse, collections, json, pathlib, re, sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
DISALLOWED_LICENSE_RE = re.compile(r"\bAGPL(?:-[0-9.]+)?\b", re.IGNORECASE)
EXPECTED_DEB = {"ca-certificates", "libjpeg62-turbo", "libssl3t64", "zlib1g"}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sbom", required=True)
    parser.add_argument("--grype", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    sbom = json.loads(pathlib.Path(args.sbom).read_text())
    grype = json.loads(pathlib.Path(args.grype).read_text())
    manifest = json.loads((ROOT / "license-manifest.json").read_text())
    manifest_crates = {(c["component"], c["version"]) for c in manifest["components"]}

    packages = sbom.get("packages", [])
    sbom_crates = set()
    deb_names = set()
    disallowed = []

    for pkg in packages:
        name = pkg.get("name", "")
        ver = pkg.get("versionInfo", "")
        for field in ("licenseConcluded", "licenseDeclared"):
            val = pkg.get(field) or ""
            if DISALLOWED_LICENSE_RE.search(val):
                disallowed.append({"package": name, "version": ver, "field": field, "license": val})
        for ref in pkg.get("externalRefs", []):
            loc = ref.get("referenceLocator", "")
            if loc.startswith("pkg:cargo/") and name != "anpdf":
                sbom_crates.add((name, ver))
            elif loc.startswith("pkg:deb/"):
                deb_names.add(name)

    for ext in sbom.get("hasExtractedLicensingInfos", []):
        for field in ("licenseId", "name"):
            val = ext.get(field) or ""
            if DISALLOWED_LICENSE_RE.search(val):
                disallowed.append({"extracted": ext.get("licenseId"), "field": field, "license": val})

    license_counts = collections.Counter()
    for pkg in packages:
        if not any(ref.get("referenceLocator", "").startswith("pkg:deb/") for ref in pkg.get("externalRefs", [])):
            continue
        concluded = pkg.get("licenseConcluded")
        declared = pkg.get("licenseDeclared")
        license_name = next(
            (value for value in (concluded, declared) if value and value != "NOASSERTION"),
            "unrecorded",
        )
        license_counts[license_name] += 1
    missing_deb = sorted(EXPECTED_DEB - deb_names)
    unexpected_crates = sorted(sbom_crates - manifest_crates)

    severity_counts = collections.Counter()
    state_severity_counts = collections.Counter()
    fixable_high_critical = []
    for m in grype.get("matches", []):
        vuln = m.get("vulnerability", {})
        art = m.get("artifact", {})
        sev = vuln.get("severity", "Unknown")
        fix = vuln.get("fix", {})
        state = fix.get("state", "unknown")
        severity_counts[sev] += 1
        state_severity_counts[(state, sev)] += 1
        if state == "fixed" and sev in ("Critical", "High"):
            fixable_high_critical.append({
                "id": vuln.get("id"),
                "severity": sev,
                "package": art.get("name"),
                "version": art.get("version"),
                "fixVersions": fix.get("versions", []),
            })

    report = {
        "sbomPackageCount": len(packages),
        "debianPackageCount": len(deb_names),
        "debianLicenseCounts": dict(sorted(license_counts.items())),
        "rustCrateCount": len(sbom_crates),
        "expectedDebianPackagesMissing": missing_deb,
        "unexpectedRustCrates": [{"name": n, "version": v} for n, v in unexpected_crates],
        "disallowedLicenses": disallowed,
        "vulnerabilitySeverityCounts": dict(sorted(severity_counts.items())),
        "vulnerabilityFixStateSeverityCounts": {
            f"{state}/{sev}": count
            for (state, sev), count in sorted(state_severity_counts.items())
        },
        "fixableHighOrCritical": fixable_high_critical,
    }
    pathlib.Path(args.out).write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))

    failures = []
    if missing_deb:
        failures.append(f"missing expected debian packages: {missing_deb}")
    if unexpected_crates:
        failures.append(f"uninventoried rust crates in binary SBOM: {unexpected_crates}")
    if disallowed:
        failures.append(f"disallowed AGPL licenses detected: {disallowed}")
    if fixable_high_critical:
        failures.append(f"fixable High/Critical vulnerabilities detected: {len(fixable_high_critical)}")
    if failures:
        print("OCI AUDIT FAILED:", "; ".join(failures), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
