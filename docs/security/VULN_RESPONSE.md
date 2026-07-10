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
| RUSTSEC-2026-0104 | `rustls-webpki` 0.101.7 | Reachable panic in CRL parsing (`from_der`). The advisory states applications that do not use CRLs are unaffected; Reaper uses the AWS SDK purely as an HTTPS client and never parses CRLs → **unreachable**. | maintainers | AWS SDK → rustls 0.23 |
| RUSTSEC-2026-0098, RUSTSEC-2026-0099 | `rustls-webpki` 0.101.7 | Certificate **name-constraint** validation bugs (URI-name constraints ignored; constraints wrongly accepted for wildcard names). Unlike 0104 these are in the cert-verification path the AWS TLS client uses, so *not* "cannot affect us". **Residual risk is low**: exploiting them requires a trusted, name-constrained CA to mis-issue a cert outside its constraints (post-signature-verification) plus a MITM of the AWS endpoint. Accepted as low risk. | maintainers | AWS SDK → rustls 0.23 |

**Common context for the three `rustls-webpki 0.101.7` items above:** all share one root cause — the crate is pulled only transitively via `aws-smithy-http-client` (the AWS Rust SDK's HTTPS connector, still on the legacy hyper-0.14 + rustls-0.21 stack at its latest version) and only behind reaper-management's **optional** `storage-s3`/`storage-dynamodb` features. A default build (filesystem/Postgres storage) never compiles this code. No downstream fix exists — the AWS SDK controls the rustls version and feature unification prevents swapping the connector; verified that even the latest `aws-smithy-http-client` (1.2.0) still pulls rustls 0.21. Removal is a one-line `cargo update` once the AWS SDK adopts rustls 0.23; the weekly scheduled audit keeps these surfaced until then.

## Reporting a vulnerability

See the repository [`SECURITY.md`](../../SECURITY.md) for private disclosure
instructions. Do not open a public issue for an unfixed vulnerability.
