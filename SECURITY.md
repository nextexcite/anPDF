# Security scope

This is a preliminary structural PDF tool. Untrusted PDF metadata and embedded
actions remain data. qpdf does not execute embedded actions; anPDF does not fetch
URLs, attachments, or external services. Ordinary CLI/MCP use starts no listener.
HTTP is optional, loopback-only, bearer-authenticated, with Host/Origin enforcement.

Filesystem resolution uses open directory descriptors and no-follow component
opens. Regular-file input descriptors are hashed and copied to private staging;
the staged bytes are verified too. Paths are relative, without traversal or URL
syntax. Directories must be owned by the effective user or root and deny
other/group writes. The configured root and its namespace must be controlled by
the operator; this is not isolation from another process with the same privileges
that can mutate the entire workspace namespace.

Inputs are re-opened and identities/hashes rechecked before publication. Outputs
cannot alias inputs by device/inode. New outputs use an atomic no-clobber hardlink
commit of a synced destination-filesystem temporary file. Explicit replacement
atomically renames a complete file over a distinct output. Parent directory
handles remain pinned; no check-then-rename is used for exclusive creation.
Workspace roots require a local filesystem supporting these primitives. Cleanup
is attempted after failures/cancellation; abrupt SIGKILL/host loss can leave
private staging, so a bounded disposable temporary filesystem is recommended.

Limits: 100 MiB/input, 250 MiB total input and output, 20 files, 2,000 total pages,
two active jobs, 60 seconds/job, bounded JSON requests and subprocess capture.
qpdf runs without a shell, with cleared environment, private staged absolute
paths, a new process group, and CPU/file-size/open-file limits (plus in-kernel
`RLIMIT_AS` on Linux and a 768 MiB `proc_pidinfo` RSS watchdog on macOS),
alongside bounded 200,000-object / 2,000,000-node iterative JSON inspection.
Timeout/cancellation kills the process group. Directory/input hashing is bounded;
portable wall-time interruption of a blocked filesystem I/O call is not promised.
The temporary-filesystem quota and OS/container PID/memory constraints supplement
these limits. Linux and macOS unit/integration checks are active; macOS relies on
polling for the RSS watchdog rather than kernel `RLIMIT_AS`.

The Rust process is not an OS network sandbox. Inspection was tested inside a
network namespace with no interfaces/routes to external services. OCI recipes
use `--network=none`, read-only rootfs, numeric non-root UID, dropped capabilities,
no-new-privileges, PID/memory/CPU limits and bounded private tmpfs. OCI execution
is still unqualified in this environment; see the qualification report.

Encrypted inputs fail explicitly. Transformations refuse detected signatures,
AcroForms/XFA, or unknown preservation status. Signature presence detection is
conservative structural detection, not certificate trust evaluation. qpdf's warning
exit is retained as a canonical warning; fatal structural failures never commit
outputs. Do not interpret structural success as appearance, PDF/A, or malware
assurance. Baseline truncated, bad-header, bad-filter, cyclic-tree, and nested
Flate stream-bomb cases are covered in the integration corpus; wider multi-gigabyte
fuzzing and stress testing remain ongoing release gates.

Do not place sensitive examples, keys, credentials, logs, account identifiers, or
workstation paths in source, fixtures, commits or artifacts. Security issues should
be retained locally for maintainers until an approved reporting destination exists;
no fabricated contact address is provided.

The full-history privacy gate permits only individually reviewed historical
identity exceptions in `build/privacy-baseline.json`. Each entry requires review
and binds a full commit SHA to the SHA-256 of its exact
`git show -s --format='%an%n%ae%n%cn%n%ce'` output, including the final newline;
names and addresses must not appear in the baseline. Invalid baselines fail the
gate. The reviewed `webMergeIdentitySha256` list additionally permits an exact
identity hash only for commits with at least two parents. The approved identity
does not permit single-parent squash, rebase, or ordinary commits; other names,
addresses, and committers remain rejected. Neither allowance exempts blobs,
remotes, or any other privacy checks.

Blob checks still scan the history reachable from every ref, including `HEAD`.
Email-bearing license notice blobs must be sanctioned by the committed
`license-manifest.json` of every ref that reaches them, not just the checked-out
manifest. Missing manifests sanction nothing; malformed manifests are findings.
Each offending blob counts once, and path checks apply to all blobs regardless
of license sanctions.
