<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Security Policy

FerroBRIDGE is a bridge between openEHR and two interoperability targets, HL7
FHIR and the OMOP Common Data Model. This document says which versions receive
security fixes and how to report a vulnerability privately.

## Supported versions

FerroBRIDGE is in its design phase and has no release. Once releases begin,
security fixes apply to the latest released version only; there is no back-port
line.

| Version             | Supported          |
| ------------------- | ------------------ |
| Latest release      | :white_check_mark: |
| Older releases      | :x:                |
| `main` (unreleased) | best effort        |

Once the project reaches 1.0 this table will name a supported minor line.

## Reporting a vulnerability

**Do not open a public issue for a security vulnerability.** Report it
privately through GitHub's private vulnerability reporting:

1. Open the private advisory form:
   <https://github.com/rubentalstra/FerroBRIDGE/security/advisories/new>. You
   can also reach it from the repository's **Security** tab under **Report a
   vulnerability**.
2. Describe the issue, the affected version or commit, and a reproduction if
   you have one.

This opens a private advisory visible only to you and the maintainer. If you
cannot use GitHub's form, contact the maintainer through their GitHub profile
and ask for a private channel before sending any details.

Please include, where you can:

- the affected version, tag, or commit;
- a description of the impact (what an attacker can do);
- steps to reproduce, a proof of concept, or a failing request;
- any suggested remediation.

**Never include patient data in a report.** A bridge sits in the path of
clinical data, so a reproduction is easy to build from real content by
accident. Redact it, or synthesize an equivalent case.

## What to expect

This is a solo-maintained project, so timelines are best effort rather than a
contractual service-level agreement:

- **Acknowledgement** of your report within about 5 business days.
- **An initial assessment** (is it valid, how severe) within about 10 business
  days.
- **A fix or a mitigation plan** for a confirmed vulnerability as a priority,
  released as a new patch version. A published release is immutable, so a fix
  ships forward as a new version rather than by re-tagging.

You will be kept informed through the private advisory and, with your consent,
credited when the advisory is published. Please give a reasonable window to
release a fix before any public disclosure (coordinated disclosure).

## Scope

In scope: the FerroBRIDGE server and libraries in this repository, its mapping
handling, its build and release pipeline, and its published artifacts.

Out of scope: a vulnerability in a third-party dependency with no
FerroBRIDGE-specific impact (report those upstream; Dependabot and `cargo deny`
track advisories here), a vulnerability in the openEHR CDR or the terminology
server a deployment points the bridge at (report those to that project), and an
issue that requires an already-compromised host or a misconfiguration outside
FerroBRIDGE's control.

## How releases will be verified

There is no release yet. When releases begin, binaries are built in an isolated
reusable workflow (SLSA Build Level 3) and published with a SHA-256 checksum, a
Sigstore build-provenance bundle, and a CycloneDX dependency SBOM, verifiable
with `gh attestation verify` (`.claude/rules/ci-cd.md`). This section gains the
exact command with the first release, and claims nothing before then.
