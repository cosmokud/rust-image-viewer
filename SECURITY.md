# Security Policy

## Supported versions

This project is still pre-`1.0.0`.

That matters for security expectations:

- Versions under `v1.0.0` can be buggy, unstable, incomplete, and subject to breaking internal changes.
- Pre-`1.0.0` releases should not be treated as hard security boundaries or as long-term supported branches.
- If a security fix is made before `1.0.0`, it will usually be fixed only in the latest release and current development line, not backported broadly.

Current support stance:

| Version line                  | Status                     |
| ----------------------------- | -------------------------- |
| Latest release before `1.0.0` | Best-effort security fixes |
| Older releases before `1.0.0` | Not supported              |
| `1.0.0` and later             | Not applicable yet         |

If you report an issue against an older pre-`1.0.0` build, expect to be asked to reproduce it on the latest version first.

## Reporting a vulnerability

Please avoid opening a public GitHub issue for an undisclosed security problem.

Preferred reporting path:

1. Use GitHub's private vulnerability reporting flow for this repository if it is enabled.
2. If that is not available, contact the maintainer privately through GitHub.
3. Include enough detail to reproduce and triage the issue safely.

Useful information to include:

- affected version or commit
- Windows version
- whether the app was built from source or downloaded as a release
- GStreamer runtime version, if video is involved
- whether the issue requires a crafted media file
- minimal reproduction steps
- impact assessment, if you have one

If a sample media file is required, share it privately when possible. Do not post sensitive or weaponized samples publicly during initial triage.

## Scope and expectations

This application opens complex untrusted media formats through multiple third-party decoders and a GStreamer runtime. That means:

- keeping the app updated matters
- keeping GStreamer updated matters
- security posture depends partly on upstream codec and multimedia components

Important limitations for the current project state:

- This is not a sandboxed viewer.
- This is not a hardened parser isolation environment.
- This is not yet a `1.0.0` stability line.

If you handle highly untrusted media in a hostile environment, use operating-system isolation, a VM, or another containment boundary around the app.

## Disclosure guidance

Please allow reasonable time for investigation and a fix before public disclosure.

When a report is confirmed, the likely remediation path before `1.0.0` is:

- fix on the current development line
- publish a new release
- recommend upgrading rather than supporting many historical builds

## What is usually in scope

Examples of issues that are worth reporting privately:

- memory safety problems triggered through supported media handling
- arbitrary file overwrite or unexpected file access caused by the viewer itself
- command execution paths that can be reached through normal app usage
- serious denial-of-service cases that are significantly worse than normal pre-`1.0.0` instability
- unsafe single-instance or IPC behavior that crosses expected trust boundaries

## What is usually out of scope

Examples that are usually not treated as security vulnerabilities by themselves:

- ordinary crashes in already-known unstable pre-`1.0.0` areas without a meaningful security impact
- missing hardening features that have never been claimed
- issues that only affect unsupported historical versions when the latest release is not affected
- vulnerabilities that exist solely in a stale or unsupported third-party runtime on the host machine
