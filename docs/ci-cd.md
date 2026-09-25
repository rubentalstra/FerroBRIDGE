<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# CI: the two tiers and the one required check

No specification governs this; it is FerroBRIDGE's own design, grounded in the
OWASP GitHub Actions Security Cheat Sheet, OpenSSF Scorecard, and the zizmor
audit set. The enforceable discipline is `.claude/rules/ci-cd.md`, which this
document does not repeat. What follows is the design of
`.github/workflows/ci.yml` and the reason it is shaped this way. The release
lane is three files of its own and is documented in `docs/release.md`; what it
shares with this one is the pinning discipline and the tool pins below.

## The problem

The repository is in its design phase. There is no Cargo workspace, so a
conventional CI workflow would have nothing to run, and a workflow added later
would arrive after the files it is meant to guard. The workflows themselves
hold tokens, and the shell scripts under `scripts/` are the tracker helpers and
the committed guards, so both are live code from day one and both need a gate.

## The two tiers

`ci.yml` splits on whether a check needs Rust.

**Tier 1 runs today, on a tree with no code.**

| Job | Runs |
|---|---|
| `zizmor` | `zizmor --min-severity=low .github/`, with `GH_TOKEN` so the online audits (`impostor-commit`) work |
| `actionlint` | the official digest-pinned image, with `SHELLCHECK_OPTS='-e SC2016'` |
| `shellcheck` | `--severity=style` over every tracked `*.sh` and every tracked extensionless file with a shell shebang |
| `hadolint` | every tracked Dockerfile under `.hadolint.yaml` (`failure-threshold: warning`) |
| `comment-style` | `scripts/checks/comment-style.sh --all` |
| `versions` | `scripts/checks/versions.sh` |
| `favicon-sync` | `scripts/checks/favicon-sync.sh` |

`hadolint` runs against a real recipe since #22: `docker/Dockerfile`, the one
tracked Dockerfile, whose digest-pinned `FROM` the `versions` job checks
against `docs/VERSIONS.md` in the same tier. `versions` still skips the checks
whose subject file is absent, reporting each skip with its reason, so it gains
teeth as files appear.

`favicon-sync` covers a duplication the other guards cannot see. The book's
mdBook theme reads its favicon from `website/book/theme/favicon.svg` and
`favicon.png`, which are copies of `assets/brand/favicon.svg` and its 32-pixel
raster. The mark therefore exists twice, the regeneration block in
`assets/brand/README.md` writes both, and nothing checked that someone had run
it. The guard compares each copy with its source byte for byte and names the
file that differs (#52).

**Tier 2 is written now and gated off.** A `detect` job checks out and looks
for a root `Cargo.toml`, publishing a boolean output. Every Rust job carries
`needs: detect` and `if: needs.detect.outputs.cargo == 'true'`: rustfmt,
clippy at `-D warnings`, nextest plus doctests, rustdoc at `-D warnings`,
`cargo deny check`, MSRV through `cargo hack check --rust-version`, and
`dependency-review-action` on pull requests. Each lane mirrors the local
command in `.claude/rules/ci-cd.md` verbatim. The workspace pull request
therefore changes nothing in CI; the lanes activate by themselves.

## The end-to-end lane and its gate

`e2e` is the one job that starts real servers. The container harness in
`tools/ferrobridge-testkit` reads the environment variable `FERROBRIDGE_E2E`,
and every container-backed test returns before it touches Docker unless that
variable is exactly `1`. The ordinary `test` job therefore stays offline and
fast, and `e2e` sets the variable and runs the three packages that own those
tests: `cargo nextest run --locked -p omop-cdm -p ferrobridge-openehr -p
ferrobridge-testkit --no-tests=pass`.

Three properties of the job are deliberate. It names packages rather than
`--workspace`, because a workspace-wide compile schedules the generated crates
side by side and the runner runs out of memory. It uses no `services:` block:
`testcontainers` talks to the runner's own Docker daemon, and the harness
creates the network, starts PostgreSQL and the reference CDR, and tears both
down when the test's value drops. It needs no registry credential, because
every image is a public package, pinned by tag and by digest in
`docs/VERSIONS.md` and in the harness constants the versions guard compares.

The trade the gate makes: a gated-off test is reported by nextest as passed,
not as skipped, because it returns `Ok(())` rather than being `#[ignore]`d. An
`#[ignore]` would show honestly as skipped but would also need a second
invocation flag to run at all, which puts the lane one forgotten flag away from
never running. The gate variable is the single switch instead, and this job is
where it is on.

## The conformance gate

`conformance` runs `scripts/checks/conformance.sh --check` (#24). There is no
external conformance suite for FHIRconnect or OMOCL, so the corpus tests are
the instrument (`docs/architecture.md` §11). Four tests, one per corpus, give
each case a verdict and write it to `target/conformance/<corpus>.json` through
`ferrobridge_testkit::conformance`:

| Corpus | Case | Test |
|---|---|---|
| `fhirconnect-mapping-lib` | a file of `docs/specs/fhirconnect-mapping-lib/` | `crates/fhirconnect/tests/it/corpus.rs` |
| `omocl` | a file of `docs/specs/omocl/` | `crates/omocl/tests/it/corpus.rs` |
| `roundtrip` | a round-trip chain | `crates/fhirconnect/tests/it/roundtrip.rs` |
| `draft-rest-api` | an FSH operation definition of the draft chapter | `crates/fhirconnect/tests/it/operations.rs` |

Each test compares its verdicts with the committed
`conformance/<corpus>/pass-list.txt` and fails when a listed case no longer
passes, so a regression fails the ordinary `test` job too. The script adds the
rest under `--check`: a passing case the list does not record, a corpus whose
size moved from the list's `total` line, a badge under `conformance/badges/`
that disagrees with its list, and a README conformance badge that names no
committed badge file. `scripts/checks/conformance.sh --update` rewrites the
lists and the badges from one run. The script runs one package per `cargo
nextest` invocation, for the same memory reason the `test` job uses
`cargo hack`, and it needs `jq`, which the hosted runner carries.

The README shows each badge through
`https://img.shields.io/endpoint?url=<raw URL of the badge JSON on main>`, the
shields.io endpoint schema (<https://shields.io/badges/endpoint-badge>):
`schemaVersion` 1, a label, the message `k / n`, and a colour by share. No
specification governs the gate: our own design, following the sibling
terminology server's pass lists.

## The sqlx query metadata

`omop-cdm` checks its SQL at compile time with `sqlx`, from the query metadata
committed under `crates/omop-cdm/.sqlx/`, so no build needs a database.
`sqlx-offline` keeps that metadata honest. It runs the PostgreSQL image the
matrix pins as a `services:` container that trusts local connections, and
runs `scripts/checks/sqlx-offline.sh --check` with `DATABASE_URL` pointing at
it. The script applies the vendored OHDSI DDL into a `cdm` schema through
`psql` and runs `cargo sqlx prepare --check`, which fails when the committed
metadata no longer describes the queries. Without `DATABASE_URL` the same
script starts the pinned image in Docker, which is how you regenerate the
metadata locally. The versions guard compares the service image and the
`sqlx-cli` pin with `docs/VERSIONS.md`.

`fhir-types-features` runs `cargo hack check -p fhir-types --each-feature
--locked` plus one wide combination, because the generated crate
holds the union of its declared root sets and its features select inside it
(`crates/fhir-types/README.md`).

Three tier-2 jobs guard what is published rather than what compiles.
`codegen-drift` runs `cargo run --locked -p fhir-codegen -- emit --check`, so
the committed `crates/fhir-types` tree always matches what the emitter produces
from the vendored HL7 packages. `publish-dry-run` runs `cargo publish
--workspace --dry-run --locked` with no cache, so a packaging failure surfaces
on the pull request instead of in a release. `crate-version-guard` runs on pull
requests only and fails a change that alters a crate's packaged content without
moving that crate's version, which a published version's immutability makes a
hard rule (`.claude/rules/crates-publishing.md`); the `no-crate-bump` label is
its escape.

`hashFiles()` cannot do this work. It is evaluated before checkout, when the
workspace is empty, so a `if: hashFiles('Cargo.toml') != ''` gate never sees
the file. A detection job that checks out and tests for the file is the
correct primitive, and `codeql.yml` already uses the same one for its Rust
analysis.

## Why a mostly-skipped pipeline is still enforceable

A branch ruleset requires status checks by name, and a job that GitHub reports
as `skipped` satisfies a required check without having verified anything. Ten
skipped Rust lanes named individually in the ruleset would read as ten green
checks over a tree nobody compiled, and re-listing the checks would be an owner
action every time a job is added or renamed.

The `conclusion` job removes both problems. It `needs` every other job, runs
under `if: always()` so it reports even when its dependencies were skipped, and
reads `join(needs.*.result, ' ')` through `env:`. It fails when any result is
`failure` or `cancelled`, and passes when every result is `success` or
`skipped`.

**`conclusion` is the single required status check on `main`.** That gives one
name in the ruleset that never changes as jobs come and go, and it collapses
the whole matrix into one honest verdict: a skip is a skip because the gate
decided the lane had nothing to check, while a real failure anywhere still
fails the required check. A job added to `ci.yml` must be added to the
`conclusion` job's `needs` list in the same change, or its result is not
counted.

Reading the results through `env:` rather than splicing them into the `run:`
block is the same template-injection rule every other workflow follows
(`.claude/rules/ci-cd.md`).

## What zizmor audits, and the cooldown decision

The zizmor lane audits `.github/`, not `.github/workflows/`. zizmor reads more
than workflow files: it audits `dependabot.yml` and the composite actions under
`.github/actions/`, and a composite action runs with the calling workflow's
permissions, so it is the same class of token-holding code. Auditing only the
workflows left both invisible.

Widening the path surfaced one finding, `dependabot-cooldown` at medium
severity and high confidence: `default-days: 3` on the `github-actions` entry,
below the 7-day floor the audit enforces. The three-day value had a recorded
defence, that an action runs in CI rather than shipping in the product, that
every action is digest-pinned so a bump is a reviewed change of digest, and
that Dependabot applies no cooldown to advisory-driven bumps. Only the third
leg survives inspection. CI is where the release-signing identity lives, so
"runs in CI" understates the consequence rather than reducing it, and a
maintainer approving a digest bump does not read the action's diff, so
digest pinning records what changed without reviewing it.

The cooldown was raised to 7 days on `github-actions` and `docker`, matching
the 7-day minor and 14-day major values already on `cargo`. The cost is a week
of delay on convenience bumps that arrive on a weekly schedule anyway, and
security updates are exempt from cooldown by design, so an advisory still
arrives immediately. The benefit is four more days of community detection
window on a compromised release, which is the attack this control exists for.
No suppression was recorded and the audit path was not narrowed
(`.claude/rules/ai-code-review.md`).

## Every pin, and what watches it

A pin nothing watches goes stale silently, so each class of pin names its
mechanism here.

| Pin | Watched by |
|---|---|
| `uses:` references in `.github/workflows/**` and `.github/actions/**` | Dependabot, `github-actions` ecosystem |
| the workspace dependency table in the root `Cargo.toml` | Dependabot, `cargo` ecosystem |
| the digest-pinned `FROM` of `docker/Dockerfile` | Dependabot, `docker` ecosystem at `/docker` |
| the analyzer versions in `ci.yml` and the documentation toolchain in `.github/actions/docs-toolchain` | `pin-freshness.yml`, weekly |
| the container image tags in `compose.yaml` and the harness constants | `scripts/checks/versions.sh` against `docs/VERSIONS.md` |

All three Dependabot ecosystems resolve something today. The `cargo` and
`docker` entries were inert until their manifests landed, and inert meant
failing: each weekly job ended `dependency_file_not_found`, which is the noise
#41 was filed against. The root `Cargo.toml` landed with #107 and
`docker/Dockerfile` with #22, and the `docker` entry now names the real path
rather than covering two candidates, so no expected failure remains. Dependabot
update runs are not exposed by the REST API; read them under Insights,
Dependency graph, Dependabot.

**Dependabot does not cover an analyzer version**, and that is the gap
`pin-freshness.yml` fills (#35). The `github-actions` ecosystem reads a `uses:`
reference. It does not read `rhysd/actionlint:1.7.12@sha256:…` inside a `run:`
block, it does not read `tool: zizmor@1.29.0` passed to an installer as an
input, and it does not read a commit a vendor script fetches an asset from.
`taiki-e/install-action` carries neither actionlint nor hadolint in its
manifest list, and neither is a crate, so its `cargo-binstall` fallback cannot
reach them either; both stay on their official images, pinned by tag and by
digest.

`scripts/checks/pin-freshness.sh` reads each of those pins from
`docs/VERSIONS.md` and compares it with the newest release of the upstream
project, which is the tag the image and the installer both carry. The workflow
runs it every Monday and on dispatch. When a pin is behind it opens one issue
carrying the report, and when an open issue already carries that report it adds
nothing. A pin it could not read fails the job, so a network failure never
reads as a fresh pin.

It opens an issue rather than failing red. A weekly red job on a lint version
teaches a maintainer to ignore red jobs, which is the habit #41 identified as
the actual risk.

## Why the GitHub licence field reads NOASSERTION

`gh api repos/rubentalstra/FerroBRIDGE --jq .license` returns `NOASSERTION`
("Other"), and it will keep returning it. This is not a defect in `LICENSE`
(#45).

GitHub detects a repository licence with
[licensee](https://github.com/licensee/licensee), which matches the licence
file against the crowdsourced set on
[choosealicense.com](https://github.com/github/choosealicense.com/tree/gh-pages/_licenses).
That set holds 46 licences and BUSL-1.1 is not one of them; the nearest
identifier in it, `bsl-1.0`, is the Boost Software License. With no BUSL-1.1
entry to match, no layout of the file can produce a match, and a layout change
would be a change to the licence text for a detector rather than for a reader.
The same call returns `NOASSERTION` for `hashicorp/terraform` and
`hashicorp/vault`, which carry the canonical BUSL-1.1 template in full, and for
FerroEHR and FerroTERM.

What the licence field cannot say, other channels do: `LICENSE` and `NOTICE`
carry the terms, every first-party file carries an
`SPDX-License-Identifier: BUSL-1.1` header that `scripts/checks/versions.sh`
enforces, and each published crate carries `license = "BUSL-1.1"` in its
manifest, which is what crates.io and `cargo deny` read. Do not change a term
to satisfy a detector.

## Scorecard alerts: the decisions

OpenSSF Scorecard reports seven checks against this repository. Each is decided
rather than left open (#48). The dismissals in the Security tab are the owner's
to apply; this table is the reasoning behind each.

| Check | Decision |
|---|---|
| `BranchProtectionID` | accepted as is. The four warnings (settings do not apply to administrators, no required approvers, no required CODEOWNERS review, last-push approval off) each cost a single maintainer the ability to merge their own work. The enforcement that does hold is the `conclusion` required check, the pull-request requirement, signed commits, and the `release-tags` ruleset. Revisit when the maintainer set grows |
| `CodeReviewID` | accepted as is, the same root cause: a solo maintainer approves no changesets |
| `MaintainedID` | clears with time. It scores 0 only because the repository is under 90 days old |
| `CIIBestPracticesID` | registered on 2026-09-13 as bestpractices.dev project 14612; the check reads the registration and clears on the next run |
| `FuzzingID` | `cargo fuzz` targets over the YAML mapping loader and the openEHR mapping-path parser run weekly from `fuzz.yml` (#155); Scorecard detects OSS-Fuzz and ClusterFuzzLite registrations, so the check scores only once the project registers there, a second step worth taking when the targets prove useful |
| `SASTID` | already satisfied. CodeQL runs on every pull request and every push to `main`; the score lagged because its Rust job was gated off until the workspace landed |
| `SecurityPolicyID` | fixed in #47, which gave `SECURITY.md` the link the check looks for |

## Configuration this workflow reads

- `.github/actionlint.yaml`: no self-hosted runner labels, and the
  configuration-variables check disabled.
- `.hadolint.yaml`: `failure-threshold: warning` plus the trusted registries.
- `.dockerignore`: denies everything but a staged `dist/` tree, so no source
  or build output enters a container build context. `docker/Dockerfile` copies
  exactly one file out of it, `dist/<os>_<arch>/ferrobridge`.

Each tier-2 job installs the toolchain through the composite
`./.github/actions/setup-rust` action, which wraps a digest-pinned
`actions-rust-lang/setup-rust-toolchain` step, reads the channel from
`rust-toolchain.toml`, and takes the `components` a job asks for (`rustfmt`,
`clippy`), so the pin lives in one file.

Every cargo invocation in the workflow runs with `CARGO_BUILD_JOBS=2` (a
workflow-level `env:`), and the `clippy` job lints one package at a time
through `cargo hack clippy`, the way the `msrv` job compiles: a workspace-wide
`cargo check` or `cargo clippy` schedules the two generated giants
(`fhir-types` with its four versions, `openehr-am` behind the client) side by
side, the hosted runner's seven gigabytes run out, and the runner kills the
job with exit 143 and no diagnostic; per package the two never meet. The
`clippy-fhir-types` matrix lints `fhir-types` per version with `resources`,
which is what a consumer builds; the all-features union (four versions, every
resource, one crate) is no consumer's surface and is not linted in CI. No
specification governs this: our own design.

## The release lane beside this one

`release.yml` is dormant until a `v*` tag is pushed, and it calls two reusable
workflows that this one never touches. They are named here because they share
this file's pinning rules and because Dependabot's `github-actions` ecosystem
bumps their actions along with `ci.yml`'s: all three live under
`.github/workflows/`, which is the directory Dependabot and the zizmor and
actionlint lanes read.

| Workflow | Runs | Tools it pins |
|---|---|---|
| `release.yml` | on a `v*` tag: plan, draft, the two calls, publish, crates.io | none of its own |
| `release-build.yml` | called once per target, on a runner of that architecture | `cargo-auditable`, `cargo-cyclonedx`, `syft` |
| `release-image.yml` | called once the musl binaries exist | `syft` |

Those three versions are rows of `docs/VERSIONS.md`, and
`scripts/checks/versions.sh` reads every `tool:` line of the two release
workflows back against them, the same way it reads `ci.yml`'s four analyzers.
They decide what a consumer can prove about a published binary, so a floating
version would change the contents of a release with no reviewed change.

The composite `./.github/actions/setup-rust` is shared with `ci.yml` and takes
a `target` and a `cache` input for this reason: a publishing lane passes
`cache: "false"`, because a cache an untrusted run could poison must never feed
a release.

## The fuzz lane beside this one

`.github/workflows/fuzz.yml` runs the two `cargo fuzz` targets of the `fuzz/`
crate (the YAML mapping loader and the openEHR mapping-path parser of
`openehr-mapping-core`) every Wednesday and on dispatch, five minutes per
target by default. It is a time-boxed search rather than a pass-or-fail check,
so it is never a pull-request gate and never a `conclusion` input. A panic, an
abort or a hang is a defect: the run fails and uploads the reproducing input as
an artifact for 90 days, and the finding becomes a `bug` issue with the input
attached. An `Err` from the parser is the correct answer and is never a
finding. The lane is the one job on a nightly toolchain, through the
`toolchain` input of the `setup-rust` composite, because cargo-fuzz needs
sanitizer flags stable does not carry; the `fuzz/` crate is excluded from the
workspace so the product stays on the pinned stable toolchain. Its
`cargo-fuzz` pin is a `docs/VERSIONS.md` row the versions guard checks.

## Triggers and concurrency

`push` to `main`, `pull_request` against `main`, `merge_group`, and
`workflow_dispatch`. `cancel-in-progress` is true for pull requests only: a
push to `main` and a merge-group run each verify a commit that must keep its
own result, while a superseded pull-request run verifies a commit nobody will
merge.

## Owner actions (one-time, not scriptable)

These are repository settings only the owner can change. Issue #19 is the
checklist and the record; this table is the same list for the next reader, with
the state on 2026-09-05.

| Setting | State |
|---|---|
| `main` ruleset: requires a pull request, signed commits, and the `conclusion` status check with the strict up-to-date policy; deletions and non-fast-forward pushes blocked | done |
| Code scanning in advanced setup, with the CodeQL default setup off so `codeql.yml` is the analysis path | done |
| Secret scanning with push protection, Dependabot alerts, and Dependabot security updates | done |
| Artifact attestations, which the release lane's provenance and SBOM bundles are stored against | done |
| The `SONAR_TOKEN` secret, with SonarQube Cloud's Automatic Analysis off (`.claude/rules/ai-code-review.md`) | done |
| Pages publishes from GitHub Actions and serves `ferrobridge.eu` with HTTPS enforced; the apex A records point at the four GitHub Pages addresses, `www` is a CNAME to `rubentalstra.github.io`, and the domain is verified for the account | done |
| The roadmap board and the label bootstrap (`scripts/gh/labels.sh`) | done |
| Registration at bestpractices.dev | done 2026-09-13: project 14612 (<https://www.bestpractices.dev/en/projects/14612>). The badge stays out of the README while the level reads "in progress"; it joins when the self-assessment is filled in from the draft on #19 and the level is worth showing (#62). The passing level is out of reach under BUSL-1.1, since `floss_license` is a MUST |
| Dismissing the Scorecard alerts decided above as accepted trade-offs, in the Security tab | open: the decisions are recorded here; only the owner can dismiss an alert |
| Immutable releases, the repository setting that stops a published release's notes and assets from being edited | done: enabled by the owner. It is not reported by the REST API, so read it in Settings rather than from `gh api` (`docs/release.md`) |
| A `crates-io` environment with a required reviewer, and crates.io Trusted Publishing entries per crate for `release.yml` and `publish-crates.yml` | open: both lanes exist and call `scripts/release/publish-crates.sh` (#72), so what is left is the owner's side. The first version of each crate (0.0.0, the name reservation) was published locally by the owner on 2026-09-05 (#107), since a crate's first release cannot use OIDC; `fhir-types` already exists on crates.io and its Trusted Publisher entries move here from the sibling terminology server |

`conclusion` is the contract for the required-checks list. Add no other CI check
to it: a job added to `ci.yml` joins the `conclusion` job's `needs` list
instead, which keeps the ruleset stable as the pipeline grows.

## Sources

- GitHub Actions security hardening:
  <https://docs.github.com/en/actions/security-for-github-actions/security-hardening-for-github-actions>
- OWASP GitHub Actions Security Cheat Sheet:
  <https://cheatsheetseries.owasp.org/cheatsheets/GitHub_Actions_Security_Cheat_Sheet.html>
- zizmor: <https://docs.zizmor.sh/>
- actionlint: <https://github.com/rhysd/actionlint>
- hadolint: <https://github.com/hadolint/hadolint>
- Dependabot options reference:
  <https://docs.github.com/en/code-security/dependabot/working-with-dependabot/dependabot-options-reference>
- Licensing a repository, and what licensee looks at:
  <https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/licensing-a-repository>
  and
  <https://github.com/licensee/licensee/blob/main/docs/what-we-look-at.md>
- Scorecard checks:
  <https://github.com/ossf/scorecard/blob/main/docs/checks.md>
- Required status checks and rulesets:
  <https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets>
