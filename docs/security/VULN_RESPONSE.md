# Vulnerability Response SLA

This document defines how Reaper triages and remediates security
vulnerabilities — both in Reaper's own code and in its third-party
dependencies. It is the commitment a third-party-risk reviewer (SOC 2 CC7.1/
CC7.4, DORA Art. 28) can hold us to.

## Scope

- **First-party**: vulnerabilities in Reaper source (engine, agent, platform,
  management, CLI, SDK).
- **Third-party**: advisories against crates in `Cargo.lock` (surfaced by
  `cargo deny` / `cargo audit`) and CVEs in container base images (surfaced by
  Trivy).

## Severity

Severity is the CVSS v3.1 base score of the finding, adjusted for Reaper's
exposure (a vulnerability only reachable behind an unused feature flag may be
downgraded, with the reasoning recorded on the tracking issue).

| Severity | CVSS | Examples |
|----------|------|----------|
| **Critical** | 9.0–10.0 | RCE on the agent, auth bypass, signing-key disclosure |
| **High** | 7.0–8.9 | Privilege escalation, DoS of the enforcement path, audit-trail tamper |
| **Medium** | 4.0–6.9 | Info leak, limited DoS, missing hardening |
| **Low** | 0.1–3.9 | Defense-in-depth gaps, low-impact issues |

## Response windows

Windows are measured from the moment the vulnerability is confirmed (triage
complete), in business days unless stated otherwise.

| Severity | Triage (assess + confirm) | Fix released |
|----------|---------------------------|--------------|
| **Critical** | ≤ 1 business day | ≤ 7 calendar days |
| **High** | ≤ 2 business days | ≤ 14 calendar days |
| **Medium** | ≤ 5 business days | ≤ 60 calendar days |
| **Low** | ≤ 10 business days | next scheduled release |

"Fix released" means a patched version is tagged and published (and, for
dependency issues, `Cargo.lock` is updated and the supply-chain gate is green
again). If a fix is not yet available upstream, the window covers the
mitigation (feature-flag off, config change, or documented workaround) plus a
tracked issue for the eventual upgrade.

## Ownership

- The maintainers own triage and remediation. Escalation for an overdue
  Critical/High goes to the repository owners.
- The **scheduled supply-chain watch** (`.github/workflows/supply-chain-nightly.yml`)
  re-checks `main` weekly against a fresh advisory DB. A red run is a page,
  not noise: it means a CVE was disclosed against an already-shipped commit and
  the clock above has started.

## Acknowledging an unactionable advisory

When an advisory has no available fix (e.g. an unmaintained transitive crate
with no successor yet), do **not** blanket-mute it. Acknowledge the specific
advisory:

- `cargo-deny`: add the RUSTSEC id to `advisories.ignore` in `deny.toml` with a
  dated one-line justification and an owner.
- `cargo-audit`: pass `--ignore RUSTSEC-XXXX-NNNN` in the workflow with the same
  justification in a comment.
- Trivy: add the CVE to a `.trivyignore` with a justification, or rely on
  `ignore-unfixed` for base-image CVEs with no upstream patch.

Each acknowledgement is a reviewed, dated decision and is revisited when a fix
becomes available.

### Known acknowledged items

| Advisory | Component | Reason | Owner | Review by |
|----------|-----------|--------|-------|-----------|
| RUSTSEC (unmaintained) | `serde_yaml` | Unmaintained but still-functioning YAML policy parser; no vulnerability, migration to an alternative tracked. Not muted — reported as a warning, does not fail the build. | maintainers | next dependency review |

## Reporting a vulnerability

See the repository [`SECURITY.md`](../../SECURITY.md) for private disclosure
instructions. Do not open a public issue for an unfixed vulnerability.
