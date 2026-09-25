<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Checks and gates

CI runs in two tiers. The first tier gates today, on a repository with no Rust
code: it covers the workflow files, the shell scripts, any Dockerfile, and the
committed guards. The second tier is the Rust set, written and gated behind a
job that looks for a root `Cargo.toml`, so it activates by itself when the
workspace lands. One `conclusion` job reads every result and is the single
required status check on `main`.

<!-- toc -->

## Tier 1, running now

| Check | Command | What it protects |
|---|---|---|
| Workflow security | `zizmor --min-severity=low .github/` | every `uses:` pinned to a commit SHA, no credential-persisting checkout, no context spliced into a shell, and the Dependabot cooldown window |
| Workflow correctness | `actionlint` | expressions that cannot evaluate, a `needs:` naming no job, unknown runner labels |
| Shell | `shellcheck --severity=style` over every tracked shell program | shellcheck's lowest floor, so every finding gates |
| Containers | `hadolint` over every tracked Dockerfile | container-recipe defects; no Dockerfile exists yet, so it reports that and passes |
| Comment style | `scripts/checks/comment-style.sh --all` | line comments only, `// TODO(#NNNN):` naming its issue, `// NOTE:` as a citation plus one sentence |
| Version drift | `scripts/checks/versions.sh` | every file that repeats a pin agrees with `docs/VERSIONS.md`, and no first-party file claims a licence other than BUSL-1.1 |
| Favicon sync | `scripts/checks/favicon-sync.sh` | the book theme favicons stay byte-identical to the brand mark they are copies of |
| The book | `mdbook build website/book` | a page that does not build fails the pull request |

Run the same commands locally before you push. A finding costs a local run
rather than a CI round trip.

## Tier 2, gated on the workspace

`cargo fmt --all --check`, `cargo clippy --workspace --all-targets
--all-features -- -D warnings`, `cargo nextest run --workspace --locked` with
`cargo test --doc --locked` (CI runs both one package at a time through
`cargo hack`, so the generated giants never compile side by side on the
hosted runner), `cargo doc` under `RUSTDOCFLAGS=-D warnings`,
`cargo deny check`, an MSRV check, and dependency review on pull requests.
Every cargo lane runs `--locked`, so CI fails on lockfile drift rather than on
registry drift.

## The end-to-end lane, and running it locally

One more job, `e2e`, starts real servers in containers: a PostgreSQL for the
OMOP CDM catalogue test and the reference openEHR CDR with its own database for
the ITS-REST client test. It is gated on the environment variable
`FERROBRIDGE_E2E`, and every container-backed test returns without touching
Docker unless that variable is `1`, so your ordinary test run stays offline.
To run the lane yourself, start Docker and set the variable:

```bash
FERROBRIDGE_E2E=1 cargo nextest run --locked -p omop-cdm -p ferrobridge-openehr -p ferrobridge-testkit
```

The first run pulls the images, which are pinned by digest in
`docs/VERSIONS.md`; later runs start in a few seconds. Each test owns its
containers and stops them when it ends.

## The conformance pass lists

Neither FHIRconnect nor OMOCL has an external conformance suite, so the corpus
tests are the instrument. Four corpora are measured, and each keeps a committed
pass list under `conformance/<corpus>/pass-list.txt`: one passing case id per
line, then a `total` line with the corpus size. The README badges read the
counts from `conformance/badges/`.

| Corpus | A case passes when |
|---|---|
| FHIRconnect mapping library | the file parses; it validates against the published schemas (`modules/ROOT/attachments/`, `schema/schema.adoc`) or is in the pinned rejection set; it validates against FerroBRIDGE's strict schemas; it loads into the library set with no refusal naming it; and a program the suites compile against a template carrying its archetype reaches it (`types-of-mapping-files/context-mappings.adoc`) |
| OMOCL mapping library | the file parses, validates against the authored schema, passes the rules of one file, and loads into the library set with every `Include` resolved |
| FHIR round-trip laws | `PutGet` and `GetPut` both run on the chain and each declares exactly the set its reviewed snapshot pins |
| FHIRconnect REST API (draft) | the wire contract reads every `in` parameter and part the FSH operation definition declares, and the operation answers only the `out` parameters it declares, each `min = 1` one present (`rest-api.adoc`, draft) |

A case the list records that no longer passes fails the corpus test itself, so
a regression fails CI. When your change makes a case pass, or the corpus
changes size, the `conformance` job fails until you record it:

```bash
scripts/checks/conformance.sh --update
```

Commit the rewritten lists and badges with the change. Without a flag the
script compares and reports without failing on new passes; `--check` is what
CI runs. It needs `cargo-nextest` and `jq`. Never edit a list by hand, and
never remove a case from one to make CI green.

## Vendored inputs and the build-time fetch

Every external corpus enters through a committed script under
`scripts/vendor/`, which reads its pin from `docs/VERSIONS.md` and writes a
`PROVENANCE.md` beside what it fetches. The HL7 FHIR packages, the v2-to-FHIR
implementation guide among them, go to `tools/fhir-codegen/vendor/<package>/`
through `scripts/vendor/fhir-packages.sh`, and the specification corpora go to
`docs/specs/`. `scripts/checks/versions.sh` reads every provenance file back
against its pin.

The HL7 v2 definitions are the one input whose terms do not permit
redistribution, so they are never committed. `scripts/vendor/v2ig.sh` fetches
them at build time into `tools/fhir-codegen/vendor/hl7-v2ig/`, which
`.gitignore` refuses except for its committed `PROVENANCE.md`. The script
checks the file count and tree digest the pin records on every run. Run it
before the codegen drift check, as the `codegen-drift` job does:

```bash
scripts/vendor/v2ig.sh
cargo run --locked -p fhir-codegen -- emit --check
```

To move the pin, change the commit in `docs/VERSIONS.md`, run
`scripts/vendor/v2ig.sh --stamp`, and copy the file count and digest it prints
into the same row.

## The sqlx query metadata

`omop-cdm` checks its SQL at compile time from the metadata committed under
`crates/omop-cdm/.sqlx/`, and the `sqlx-offline` job fails when that metadata
is stale. After you change a query in `omop-cdm`, start Docker, install
`sqlx-cli` at the version `docs/VERSIONS.md` pins, and regenerate:

```bash
scripts/checks/sqlx-offline.sh
```

Commit the rewritten `.sqlx/` files with the query change. With
`DATABASE_URL` set in your environment the `sqlx` macros connect to that
database instead of reading the metadata, so leave it unset for an ordinary
build.

## Workflow security rules

Every workflow follows the same four rules, and the analysers above check them:

- Every `uses:` is pinned to a full commit SHA with a trailing version comment.
- `permissions: {}` at workflow level, with the minimum granted per job.
- `persist-credentials: false` on every checkout that does not push with git.
- No `${{ }}` interpolation inside a `run:` block; context travels through
  `env:`.

## Advisory analysers

CodeQL scans the workflow files, because a workflow that holds a token is code.
SonarQube Cloud runs a multi-language sweep over shell, YAML, and JSON. OpenSSF
Scorecard scores the repository's security posture. All three are advisory:
they gate no merge, and a finding never outranks a specification citation or a
project rule. A wrong finding is recorded on the tracker rather than
suppressed quietly.

## Building this book

```console
$ cargo install mdbook mdbook-toc mdbook-mermaid
$ mdbook serve website/book
```

Use the versions pinned in `docs/VERSIONS.md`, which are the ones CI installs.
The site as GitHub Pages serves it, the landing page at the root and the book
under `/docs/`, is assembled by `scripts/site/assemble.sh _site`. That script
renders the roadmap block from the open milestones when `gh` is authenticated,
and leaves the block empty without failing when it is not.
