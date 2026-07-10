# Security Policy

## Reporting a vulnerability

If you believe you have found a security vulnerability in Reaper, please report
it **privately** — do not open a public issue, pull request, or discussion for
an unfixed vulnerability.

- Use GitHub's **[private vulnerability reporting](https://github.com/pgste/reaper/security/advisories/new)**
  ("Report a vulnerability" under the Security tab), or
- email the maintainers with details and, if possible, a reproduction.

Please include the affected component (agent, platform, management, engine,
CLI, SDK), the version/commit, and the impact you observed.

## What to expect

We triage and remediate on a defined schedule — see
[`docs/security/VULN_RESPONSE.md`](docs/security/VULN_RESPONSE.md) for the
severity model and response windows (Critical triaged within 1 business day and
fixed within 7 calendar days, etc.). We will acknowledge your report, keep you
updated through triage and fix, and credit you on disclosure unless you prefer
to remain anonymous.

## Supply-chain assurance

Every change is gated by a blocking supply-chain pipeline: `cargo deny`
(advisories, licenses, bans, sources), `cargo audit` (with a weekly scheduled
re-check of `main`), and a blocking Trivy image scan. A CycloneDX SBOM is
attached to every release. See
[`docs/security/VULN_RESPONSE.md`](docs/security/VULN_RESPONSE.md) and the CI
workflows for details.

## Supported versions

Reaper is pre-1.0; security fixes land on `main` and in the latest release.
