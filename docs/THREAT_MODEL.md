# Filesystem boundary threat model

Scan is a local, read-only library. It is not a sandbox, secret redactor, or
atomic repository snapshot.

## Ordinary changing worktree

The default backend is appropriate for a local index of a developer worktree
that changes underfoot. Discovery, `File::open`, and post-read version or hash
checks are separate operations. A file may grow, shrink, or be replaced
between those steps. Concurrent-modification evidence and content hashes
detect many of those races after bytes have already been read. They do not
prove that bytes from outside the intended path were never opened.

A sequential walk is not an atomic snapshot of the tree. Per-file version
evidence binds one opened file, not a single repository-wide moment.

## Adversarial concurrent mutation

A check/open window exists: symlink and canonical-path checks are not atomic
with the later `File::open`. This analysis does not claim a proven exploit.
Callers that index untrusted trees while another party can mutate parents,
roots, or directory entries should treat the default backend as a detector,
not a preventer.

An optional handle-relative hardened backend remains out of the default path
until its cost is measured on the supported operating systems. Confirm
parent-directory, root, and symlink substitution on each target OS before
relying on any such mode.

## Portable reports

`ScanReport::to_portable` removes host paths, identities, timestamps, and
free-form diagnostics. Relative names remain. That is a trust-boundary
encoding, not secret redaction.

## Cooperative cancellation

Cancellation and deadlines are checked between read chunks. A read that is
already blocked in the kernel is not interrupted.
