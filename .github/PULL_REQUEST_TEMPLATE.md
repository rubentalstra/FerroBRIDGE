<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

## What changed and why

<!-- Describe the change and the reason for it. Keep it to what a reviewer needs. -->

Closes #NNN

## Checklist

- [ ] Spec-facing decisions cite the governing specification (HL7 FHIR, FHIRconnect, the OMOP Common Data Model, OMOCL, or openEHR ITS-REST), not memory and not another implementation.
- [ ] Shell and workflow files are clean: `shellcheck --severity=style`, `actionlint`, `zizmor --min-severity=low .github/workflows`.
- [ ] Rust gates pass, once a workspace exists: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo nextest run --workspace --locked`, `cargo test --doc --locked`, `cargo doc` with `RUSTDOCFLAGS=-D warnings`, and `cargo deny check`.
- [ ] `CHANGELOG.md` has an `[Unreleased]` entry, if the change is user-visible.
- [ ] Docs are updated, if behaviour changed.
- [ ] Every commit is signed.
- [ ] No AI or assistant attribution anywhere in the commits or this PR.

Contributions are inbound equals outbound under the Business Source License
1.1, including its Change License; there is no contributor licence agreement. See
[CONTRIBUTING.md](../CONTRIBUTING.md) for the full contribution guide.
