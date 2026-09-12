<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Maintenance rule: every pull request that changes user-visible behaviour adds
an entry under **[Unreleased]** in the same PR. Cutting a release renames
[Unreleased] to the version and date, and adds a fresh link reference.

FerroBRIDGE is in its design phase, and the architecture is the output of the
research program on
[issue #1](https://github.com/rubentalstra/FerroBRIDGE/issues/1). Releases on
the 0.0.x line carry the repository, its gates, and its documentation; there is
no binary to download yet.

## [Unreleased]

### Added

- The generated OMOP CDM v5.4 layer and its generator (#73): `omop-cdm` carries
  a row type and a column-metadata static for every one of the 39 tables the
  OHDSI definitions declare across the `CDM`, `VOCAB` and `RESULTS` schemas,
  432 columns in definition order, with the CDM datatypes mapped to `i32`,
  `f64`, a bounded `Varchar<N>`, and the lexical `CdmDate` and `CdmDatetime`;
  OHDSI's four rendered PostgreSQL files are embedded verbatim and their
  `@cdmDatabaseSchema` placeholder is substituted through a checked PostgreSQL
  identifier. `tools/omop-cdm-codegen` emits all of it from the vendored
  definitions at tag `v5.4.3`, byte-deterministically, and the
  `omop-cdm-codegen-drift` CI job fails on any difference. Tests assert the
  generated column set of every table against OHDSI's DDL.

- `crates/openehr-mapping-core` carries the half of the mapping foundation both
  languages share (#74): the header FHIRconnect and OMOCL standardize between
  them, with `GrammarVersion`, `MappingName`, `MappingVersion`, `MappingType`
  and `ArchetypeId` as distinct types and everything a language adds under
  `spec` kept as an opaque positioned node; a YAML loader over `serde-saphyr`
  that resolves anchors, aliases and merge keys into a value tree where every
  node carries its line and column, and refuses a duplicate key, an
  unresolvable alias and a tab-indented file with a positioned diagnostic; a
  registry keyed by mapping name and by archetype id that refuses a repeated
  name naming both files; one `Diagnostic` type carrying file, position,
  mapping name, model path, severity and code; and a `MappingPath` over the
  `openehr-rm` BASE path parser that resolves the languages' own `../` step
  against an anchor and refuses a step above its root. Both vendored corpora
  load through the public seam in the test suite, which pins the exact set of
  files this crate refuses and why.
- The specification corpora under `docs/specs/` (#71), each fetched by a
  committed `scripts/vendor/*.sh` from the commit `docs/VERSIONS.md` pins and
  stamped with a `PROVENANCE.md`: the FHIRconnect specification source with its
  two published schemas and the unmerged REST API chapter, the FHIRconnect
  mapping library, the OMOCL corpus, the OMOP CDM v5.4 definitions with OHDSI's
  rendered PostgreSQL DDL, and the three `STABLE` openEHR ITS-REST OpenAPI
  documents. An integration test reads every tree, and
  `scripts/checks/versions.sh` fails when a provenance stamp stops naming the
  pin the matrix records.
- `fhir-types` carries a feature table (#120): `r4`, `r4b`, `r5` and `r6` gate
  the version modules, `terminology` carries the terminology root set the crate
  has always held, and `resources` widens the declared root set to every
  concrete resource each package defines with the complete closure of the
  datatypes it references. The default set is every version plus `terminology`,
  so a dependant that names no feature gets what it had. One emitted tree holds
  the union and each item carries the `cfg` of the narrowest feature that
  selects it, so the drift gate still covers the whole tree; a new
  `fhir-types-features` CI job checks each feature on its own.
- `docs/architecture.md` §3 supports both template generations (owner
  requirement, 2026-09-12): ADL 1.4 fetched as OPT 1.4 XML and ADL 2 fetched
  as AOM2 canonical JSON from the `adl2` route, decoded into the two OPT types
  and built into one Web Template by the two `openehr-its` builders behind one
  seam; the resolution order, the ADL 2 `Accept` rule, the HRID identifier
  handling, the at-code versus id-code refusal and the term-binding delta are
  recorded as FerroBRIDGE's own; `openehr-am` joins the pins and
  `openehr-adl` stays out until a CDR serves ADL 2 as text alone.
- `docs/architecture.md` carries ten mermaid diagrams: the system picture, the
  two languages over one foundation, the openEHR path pipeline, the FHIRconnect
  pipeline, the inbound create and the `$tofhir` sequences, the OMOP ETL flow,
  the crate graph, the FHIR identity derivation and the build order; the book's
  architecture tour gains the FHIRconnect pipeline and the ETL flow.
- The generated FHIR model and its generator (#72), moved in from the sibling
  terminology server by the owner decision of 2026-09-05: `crates/fhir-types`
  at 0.1.98, the one first-party crate under Apache-2.0, with the per-version
  R4, R4B, R5 and R6 modules, the strict JSON and XML codecs and the
  terminology operation contracts; `tools/fhir-codegen`, which emits the crate
  from the five vendored HL7 FHIR packages (380 MB, each with its
  `PROVENANCE.md`); `scripts/vendor/fhir-packages.sh`, which refetches those
  packages against the `docs/VERSIONS.md` pins; the `codegen-drift` CI job,
  which regenerates in check mode and fails on any difference; and the
  `regen-codegen` skill. Features, the public element table and the
  `serde_json::Value` conversion follow in the next increments of #72.
- The crates.io lane the published crate line needs (#72): the `crates` leg of
  `release.yml`, gated by the `crates-io` environment and authenticated with
  Trusted Publishing; `.github/workflows/publish-crates.yml`, the dispatch dry
  run and recovery path; `scripts/release/publish-crates.sh`, which uploads
  each member in dependency order and reads the registry back at each crate's
  own manifest version; the `publish-dry-run` CI job; and
  `scripts/checks/crate-version-guard.sh`, which fails a change that alters a
  crate's packaged content without moving its version.
- The workspace root discipline is complete (#20): a composite
  `.github/actions/setup-rust` action reads `rust-toolchain.toml` for every
  Rust-tier job, and every workspace member carries one integration binary at
  `tests/it/main.rs` (the foundation crate asserts the four `openehr-*` rows of
  the pin matrix name one line; the server test drives the library run path;
  the testkit tests its own pin-matrix reader).
- The Cargo workspace skeleton (#107): the root manifest with every lint the
  reliability rule names, the release profile, `rust-toolchain.toml`,
  `rustfmt.toml`, `clippy.toml`, `deny.toml` and a committed `Cargo.lock`;
  six placeholder library crates at version 0.0.0 (`openehr-mapping-core`,
  `fhirconnect`, `omocl`, `omop-cdm`, `ferrobridge-openehr`,
  `ferrobridge-term`), published to reserve their names, each carrying its
  pinned specification version as a constant that an integration test checks
  against the pin matrix; a thin `ferrobridge` binary over a library that does
  nothing yet; and the `ferrobridge-testkit` tool crate with the pin-matrix
  reader. CI tier 2 is active from this change on.

### Changed

- `docs/architecture.md` is rewritten as the output of the third research
  pass (2026-09-12): the `fhir-types` move is re-baselined on the crate as it
  is today (no cargo features, a lexical-precision `Value` instead of
  `serde_json::Value`, an XML-schema element table without cardinality, a
  crate line at 0.1.97 still published by the sibling); the draft FHIRconnect
  REST API (`$tofhir`, `$toopenehr`, specification pull request #93) becomes
  the engine surface beside the facade and ships ahead of it; FHIR identity
  takes the entry `uid` first per the upstream revision (pull request #94)
  with the hash as the fallback; the reference engine's 3.0.x behaviour
  changes are adjudicated (plural condition keys with OR semantics, lexical
  date and time preservation, no invented `DV_PROPORTION` extension URLs,
  refusal instead of warn-and-continue); a carry-over register records every
  decided behaviour, test invariant and known defect of the reference CDR's
  retiring FHIR connector with a disposition; the EHDS priority categories
  and the HL7 Europe guides become the planned profile targets in order (Base
  and Core, Laboratory, Medication, then Patient Summary), authored here and
  never a conformance claim; the build order gains v0.0.8 (the EU targets)
  and v0.0.9 (the change-feed adapter).
- `docs/VERSIONS.md` moves the `openehr-*` crates to 0.0.64 and the
  `fhir-types` floor to the sibling's latest release, pins the draft REST API
  chapter by pull-request commit, and gains a profile-package section that
  fills as contexts are authored. `CLAUDE.md` states the move as pending.
- `docs/architecture.md` §7 collapses the published crate set from twelve to
  seven (one crate per language, with modules), after a duplication check
  against both sibling projects; the tracker issues keep the module work units.

- `docs/architecture.md` is rewritten as the output of the second research
  pass (2026-09-05): the FHIR model is generated in this repository (the
  `fhir-types` crate and its generator move here from the sibling terminology
  server, which then consumes the crate from crates.io); the FHIRconnect
  mapping files are validated by a hand-written model, the vendored published
  schemas exercised as evidence, a strict schema of FerroBRIDGE's own, and
  semantic checks; a context mapping compiles once into an immutable program
  with pinned extension ordering; one interpreter serves both directions over
  data-type lenses; the openEHR wire is canonical JSON with the Web Template
  built locally from the OPT; FHIR ids derive from the stable composition uid
  and OMOP ids from sequences with a bridge-owned natural-key table; the OMOP
  commit unit is one composition's record graph; the library crates are
  published; the binary is one with subcommands; and the build order splits the
  round trips into v0.0.3 (FHIR) and v0.0.4 (OMOP). Every decision names its
  ground and its rejected alternatives.
- `docs/VERSIONS.md` pins the corpora by commit, the four `openehr-*` crates,
  the five HL7 FHIR packages the generator reads, and the crate line; the
  `fhir-terminology` row is removed, since the client contracts live in
  `fhir-types`. `scripts/checks/versions.sh` checks the new crate rows.
- The book's Evaluate, Operate and Integrate pages, `CLAUDE.md`,
  `docs/ci-cd.md` and the rules cite the pinned ITS-REST release and follow
  the rewritten design.

## [0.0.1] - 2026-09-05

### Added

- `.github/workflows/release.yml`, the release lane, dormant until a `v*` tag
  is pushed and running on nothing else. It validates the tag shape, refuses a
  `workflow_dispatch` that is not dispatched at the tag it names, checks the
  tag against every file that declares the product version (`CITATION.cff` and
  the `docs/VERSIONS.md` product row today, the root `Cargo.toml` as soon as it
  exists), and takes the release notes from the `## [X.Y.Z]` section of this
  changelog, failing when that section is missing or empty. The release is
  created as a draft and published only once its expected asset set is
  complete. The binary build sits behind the same root-`Cargo.toml` detection
  `ci.yml` tier 2 uses, so it is skipped on a tree with no workspace and
  activates by itself when one lands (#63).
- `docs/release.md`, the cut checklist in order, the no-retag rule, and what a
  published release is protected against: the immutable-releases setting
  freezes its notes and assets, and the `release-tags` ruleset blocks tag
  deletion and non-fast-forward updates and requires a signature (#63).
- The FerroBRIDGE brand assets under `assets/brand/`: the mark and its
  dark-tile, monochrome, and favicon variants, the three lockups
  (light, dark, and one that follows `prefers-color-scheme`), the 1200x630
  social card, the raster favicon set, the "Indigo & Iron" palette as
  `--ferrobridge-*` custom properties in `tokens.css`, and the brand README
  with the usage rules and the raster regeneration commands. The landing page
  now carries the favicon, the social card as its `og:image` and Twitter card
  image, and the mark in its header; the book gets the same favicon through
  `website/book/theme/`; and the repository README shows the lockup (#32).
- `scripts/site/assemble.sh` copies `assets/brand/` and the repository-root
  `llms.txt` into the assembled site, so the brand URLs the landing page names
  and <https://ferrobridge.eu/llms.txt> both resolve on the live host (#32).
- `.github/release.yml`, so GitHub's auto-generated release notes group pull
  requests by the tracker's own label taxonomy (Features, Fixes, Security,
  Documentation, Dependencies, Maintenance, Other changes), with
  `no-changelog` as the exclusion. The hand-curated changelog stays the primary
  record (#17).
- Two `cargo` and `docker` entries in `.github/dependabot.yml` beside the
  existing `github-actions` one, each grouping minor and patch bumps into one
  weekly pull request while majors stay individual, with the conventional
  commit prefixes `ci`, `deps` and `build` and `open-pull-requests-limit: 10`
  on cargo. Both are inert until their manifests exist (#17).
- The Rust half of `.github/workflows/sonar.yml`, gated on a root `Cargo.toml`
  at step level: the pinned toolchain with `llvm-tools-preview`, the
  instrumented `cargo llvm-cov nextest` run that writes `lcov.info`, and the
  `sonar.projectVersion` derivation that anchors the New Code window to the
  workspace version. `sonar-project.properties` names the lcov file and
  excludes build output, vendored trees, and the generated OMOP CDM subtree
  (#17).
- The documentation site at <https://ferrobridge.eu/>: a hand-written landing
  page at the root and an mdBook under `/docs/`, organised by reader intent
  (Evaluate, Operate, Integrate, Contribute). `.github/workflows/docs.yml`
  builds it on every pull request and deploys from `main` through GitHub Pages,
  `.github/actions/docs-toolchain/action.yml` installs the pinned mdBook
  toolchain, and `scripts/site/assemble.sh` places both halves and renders the
  roadmap block from the open milestones. `llms.txt` and `CITATION.cff` land
  with it, the README badge block covers CI, CodeQL, Scorecard, the Sonar
  quality gate and coverage, the licence and the latest release, and
  `scripts/checks/versions.sh` now guards the citation version and the three
  documentation-toolchain pins (#18).
- `.github/workflows/ci.yml`, the CI gate, in two tiers. Tier 1 runs on a
  repository with no code: zizmor at `--min-severity=low` with the online
  audits enabled, actionlint through its digest-pinned official image,
  shellcheck at `--severity=style` over every tracked shell program, hadolint
  over every tracked Dockerfile, and the comment-style and versions guards.
  Tier 2 is the Rust set (rustfmt, clippy, nextest and doctests, rustdoc,
  `cargo deny`, MSRV, dependency review), gated behind a `detect` job that
  looks for a root `Cargo.toml`, so it activates by itself when the workspace
  lands. The `conclusion` job reads every job's result and is the single
  required status check on `main`. Housekeeping the lanes read lands with it:
  `.hadolint.yaml`, `.dockerignore`, `.github/actionlint.yaml`, and the
  `.gitattributes` `linguist-generated` line reserved for the generated OMOP
  CDM subtree. `docs/ci-cd.md` records the design, why a mostly-skipped
  pipeline is still enforceable, and the owner actions that cannot be scripted
  (#16).
- The CI tool pins (zizmor, actionlint, shellcheck, hadolint) as a section of
  `docs/VERSIONS.md`, with `scripts/checks/versions.sh` failing when
  `ci.yml` and the matrix disagree (#16).
- `docs/VERSIONS.md`, the pin matrix: one place that records every version pin
  (the five specification pins, the published `fhir-types` and
  `fhir-terminology` crate pins, the Rust toolchain,
  edition, resolver and MSRV, the product version, and the licence) with the
  files that repeat each one. `scripts/checks/versions.sh` fails on any
  disagreement and on a first-party file claiming a licence other than
  `BUSL-1.1`, and skips with a printed reason for each file that does not
  exist yet. A `PostToolUse` hook runs the guard when a pinned file changes
  (#15).

### Changed

- `SECURITY.md` and `MAINTAINERS.md` name both protections a published release
  has: the immutable-releases setting freezes its notes and assets, and the
  `release-tags` ruleset stops its tag being moved or deleted (#68).

- The README's mark uses an absolute URL, so it renders wherever the README
  travels rather than only on the repository page (#60).

- The landing page footer shows the mark beside the wordmark, so the mark
  brackets the page instead of appearing only in the header (#56).

- The FerroBRIDGE mark is now the fork the product draws: a filled circle for
  the openEHR source, a two-way arrow to HL7 FHIR, and a plain line into a
  table for the OMOP Common Data Model. Only the FHIR branch carries
  arrowheads, because that side is an exchange and the OMOP side writes rows.
  Every file in `assets/brand/` is redrawn from the new masters, along with the
  book theme favicon, the social card, the lockups and the raster favicon set.
  The palette is unchanged. `assets/brand/README.md` describes the new
  geometry and its two drawing rules, and the landing page image alt text names
  the new shapes (#54).


- The landing page runs the approved "Indigo & Iron" palette. It carried a
  pre-brand blue that twenty-one rules read, so every link, button and
  gradient disagreed with the mark beside them. Every accent now clears
  WCAG AA against its background in both light and dark (#51).
- The social card's specification line is larger and lighter, so it stays
  legible at the size a link preview actually renders (#51).

- `SECURITY.md` links the private advisory form directly, so a reporter can
  act on the policy without navigating by hand. The document carried no
  hyperlink at all, which is what Scorecard's Security-Policy check scores
  (#47).

- `SUPPORT.md` sends a reader to the documentation site first. It still said
  there was no site, which went false the day the site went live and
  contradicted `README.md` on the same tree (#44).

- The contributor page at `/docs/contribute/checks-and-gates.html` names the
  zizmor audit path `.github/`, matching what CI runs. It still named
  `.github/workflows/`, so following it missed the Dependabot configuration
  and everything under `.github/actions/` (#40).

- The zizmor lane in `.github/workflows/ci.yml` audits `.github/` rather than
  `.github/workflows/`, so `dependabot.yml` and every composite action under
  `.github/actions/` are covered. A composite action runs with the calling
  workflow's permissions, so it is the same class of token-holding code.
  `CONTRIBUTING.md` and the pull-request template name the wider path too, so a
  local run still matches CI (#34).
- The Dependabot cooldown on the `github-actions` and `docker` ecosystems is
  7 days, up from 3, which is the floor zizmor's `dependabot-cooldown` audit
  enforces and matches the 7-day minor value already on `cargo`. Security
  updates are exempt from cooldown, so an advisory still arrives immediately.
  `docs/ci-cd.md` records why the earlier defence of the 3-day value did not
  hold and why no suppression was recorded (#34).
- `CONTRIBUTING.md` lists all six tier-1 gates `ci.yml` runs, in the same order
  and with the same flags. It named four, so a contributor who ran the listed
  commands could still fail CI on hadolint or the versions guard (#36).
- The licence of the project's own code and text is the Business Source
  License 1.1 (`LICENSE`, `NOTICE`): free for non-production use and for non-commercial production use, a commercial
  licence for any other production use, and Apache License 2.0 four years
  after each version. Every header, the README badge and licensing section,
  and the community and governance documents name it (#12).

[Unreleased]: https://github.com/rubentalstra/FerroBRIDGE/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/rubentalstra/FerroBRIDGE/releases/tag/v0.0.1
