<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Contributing to FerroBRIDGE

FerroBRIDGE is a pure-Rust, standalone bridge between openEHR and two
interoperability targets: HL7 FHIR, driven by the FHIRconnect specification,
and the OMOP Common Data Model, driven by the OMOCL specification. It is in its
**design phase**: there is no code, and the architecture is the output of the
research program on
[issue #1](https://github.com/rubentalstra/FerroBRIDGE/issues/1), which
produces `docs/architecture.md`. The working discipline is
[`CLAUDE.md`](CLAUDE.md). Read it before making a change.

## What helps most right now

Evidence, not scaffolding. A specification citation that settles an open
question, a measurement, or first-hand experience running FHIRconnect, OMOCL,
or an openEHR CDR in production is worth more than a pull request that guesses
at a layout. Do not open a pull request that scaffolds a Cargo workspace or a
crate structure; that decision belongs to issue #1.

## The two layers, once code exists

- **A generated model layer**, expected on the FHIR side, from vendored
  machine-readable packages. Never hand-edit a file marked `// @generated`:
  change the generator and regenerate
  (`.claude/rules/codegen.md`).
- **The hand-written bridge engine**, which is the product: the mapping loader
  and validator, the mapping interpreter, the openEHR ITS-REST client, and the
  terminology client. Modern idiomatic Rust, with the specifications as the
  authority (`.claude/rules/rust-style.md`).

**Mappings are specification-conformant YAML**, validated against the published
schemas. A hand-coded converter per FHIR resource or per OMOP table is refused
in review.

## Build and test

Today the gates are the shell and workflow set, and they run on every change:

```
zizmor --min-severity=low .github/workflows/
actionlint
shellcheck --severity=style <tracked shell files>
hadolint --config .hadolint.yaml <tracked Dockerfiles>
scripts/checks/comment-style.sh --all
scripts/checks/versions.sh
```

These are the six tier-1 guards `ci.yml` runs, in the same order and with the
same flags, so a local pass means a CI pass. The `conclusion` job aggregates
them and is the single required check on `main` (`docs/ci-cd.md`).

Once the Cargo workspace exists, the local gates mirror CI exactly:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --workspace --locked
cargo test --doc --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items
cargo deny check
```

Run them scoped to a crate (`-p <crate>`) while iterating, and run the full
`--workspace` gates before opening a pull request. Every cargo invocation uses
`--locked`, and `Cargo.lock` is committed.

## Specifications are the authority

Five sources, one per surface: the HL7 FHIR specification for the version in
play, FHIRconnect, the OMOP Common Data Model, OMOCL, and openEHR ITS-REST.
Read the governing section before implementing spec-facing behaviour, and cite
it in the pull request. Never resolve a specification question from memory, and
never from another implementation's behaviour: openFHIR, Eos, and the EHRbase
FHIR Bridge are prior art, a running CDR or terminology server is a test
deployment, and none of them is an oracle (`.claude/rules/spec-adherence.md`).

Where the specifications are silent, say so in the text you write: "no
specification governs this: our own design".

## Branches and commits

- Branch names use conventional types: `feat/<slug>`, `fix/<slug>`,
  `chore/<slug>`, `docs/<slug>`, `refactor/<slug>`, `perf/<slug>`,
  `test/<slug>`, `ci/<slug>`, `build/<slug>`, `release/<slug>`. Never
  force-push `main`.
- Commit messages describe only the change. Do **not** add AI or assistant
  attribution, co-author trailers, or "generated with" lines anywhere.
- **Sign your commits.** Configure commit signing (GPG, SSH, or S/MIME) so
  every commit is verified.

## Pull requests

- Declare `Closes #N`, one keyword per issue.
- Keep changes compiling and tested at every step; do not defer compilation.
- Never weaken, skip, or delete a test to make a build pass.
- Add a `CHANGELOG.md` entry under `[Unreleased]` for any user-visible change.
- Every workflow `uses:` is pinned to a full commit SHA with a trailing version
  comment; keep it that way. `permissions:` is `{}` at the workflow level with
  the minimum granted per job, and no untrusted context is interpolated into a
  `run:` block; pass it through `env:`.
- Write prose to `.claude/rules/writing-style.md`: no em dashes, no
  "not X but Y", no decorative triads, no filler buzzwords.

## Using AI tools

You may. If a contribution has AI-generated content, say so in the pull-request
description: which tool, and what it did. Disclosure lives in the description
and never in a commit trailer. You remain responsible for the submission in
full: understood, explained on request, tested, and honest. See
[`AI_STATEMENT.md`](AI_STATEMENT.md).

## Security

Report vulnerabilities privately. See [`SECURITY.md`](SECURITY.md). Do not open
a public issue for a security problem.

## Licensing

By contributing you agree that your contributions are licensed under the
project's Business Source License 1.1 (see [`LICENSE`](LICENSE)), including
its Change License, so that each version becomes Apache License 2.0 on its
Change Date. Inbound equals outbound. There is no contributor licence
agreement and no copyright assignment; you keep your copyright.
