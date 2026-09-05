<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# CLAUDE.md

**FerroBRIDGE** is a pure-Rust, standalone bridge between openEHR and two
interoperability targets: HL7 FHIR, driven by the FHIRconnect specification
(model-mapping and contextual-mapping YAML validated against its published JSON
schemas), and the OMOP Common Data Model, driven by the OMOCL specification.
Mappings are specification-conformant YAML, never hand-coded per resource or
table. It runs as its own server beside any openEHR CDR reached over the
openEHR ITS-REST API (FerroEHR is the reference CDR, never a compile-time
dependency) and uses a FHIR terminology server (FerroTERM is the reference) for
code systems and value sets.

The name follows the Ferro family (FerroEHR, FerroTERM). FerroBRIDGE in prose,
`ferrobridge` in identifiers.

## The design is recorded in `docs/architecture.md`

Read it first. It is the output of the 2026-09-03 research pass over the primary
sources (issue #1) and records the decisions with their ground: FHIRconnect and
OMOCL are two languages sharing one header, so the bridge is one shared
foundation, two interpreters, two sinks; the pins are FHIRconnect v1.0.0, FHIR
R4, OMOCL v1.0.0, OMOP CDM v5.4 and openEHR ITS-REST 1.1.0; the FHIR side is a
facade over the CDR, the OMOP side a batch ETL over AQL into a CDM database; OMOP
concept resolution is SQL over the loaded OHDSI vocabulary, never a FHIR
terminology operation. Where a specification is silent (FHIR search, identity,
failure policy, extension ordering) the document names the decision as
FerroBRIDGE's own; keep that labelling in code and docs.

## The two layers

- **Generated where a machine-readable source exists:** the OMOP CDM v5.4 row
  types and DDL from the OHDSI `CommonDataModel` definitions. Every file marked
  `// @generated` is off-limits: change the generator and regenerate, never
  hand-edit generated code. The FHIR model is a DEPENDENCY, not a generator
  (`docs/architecture.md` §7); the openEHR model comes from the published
  `openehr-*` crates.
- **Hand-written and the product:** the shared mapping foundation, the
  FHIRconnect and OMOCL parsers, validators and interpreters, the ITS-REST
  client, the terminology client, the FHIR facade and the ETL runner. Modern
  idiomatic Rust of our own design, with the FHIRconnect, FHIR, OMOCL, OMOP CDM
  and openEHR specifications as the authority.

The Cargo workspace lands with the first milestone (one verbatim round trip per
target, `docs/architecture.md` §9), never before its issues are filed.

## Repo map

There is no Cargo workspace yet. What exists:

- `.claude/`: the working discipline. `rules/` (the path-scoped and standing
  rules), `hooks/`, `skills/`, `agents/`, `memory/`.
- `scripts/gh/`: the tracker helpers (`rel.sh`, `project.sh`, `labels.sh`).
- `scripts/checks/`: the committed guards (`comment-style.sh`, and
  `versions.sh`, which fails when a file disagrees with the `docs/VERSIONS.md`
  pin matrix or claims a licence other than `BUSL-1.1`).
- `.github/`: issue and pull-request templates, CODEOWNERS, Dependabot, and
  five workflows that work on a repository with no code (CI, Docs,
  Scorecard, CodeQL, SonarQube Cloud). `ci.yml` runs its workflow, shell, container and
  guard tier now and keeps the Rust tier gated behind a `Cargo.toml` detection
  job; its `conclusion` job is the single required check on `main`
  (`docs/ci-cd.md`). There is no release lane, because there is nothing to
  release.
- Root markdown: this file, `README.md`, and the community and governance set.

When the workspace lands, each crate carries its own `CLAUDE.md` with
crate-local discipline, and this section becomes the crate map.

## Issue workflow (the loop)

The tracker is GitHub Issues; the open issue list is the worklist
(`.claude/rules/issue-workflow.md`). One type label per issue
(bug/enhancement/documentation/chore/refactor/perf/ci), one priority label
(P0 to P3), and domain labels as needed (`spec:FHIR`, `spec:FHIRconnect`,
`spec:openEHR`, `spec:OMOP`, `spec:OMOCL`). Milestones are releases. Record progress on the issue (tick
criteria, comment); a PR declares `Closes #N`. New work found while working an
issue is filed and fixed before the next unit starts (the fix-first cadence).
Native sub-issue and dependency edges are set only with `scripts/gh/rel.sh`;
the roadmap board is a view over the tracker, written only with
`scripts/gh/project.sh` (`.claude/rules/issue-relationships.md`,
`.claude/rules/project-board.md`). The SessionStart hook prints the open issue
list.

## Model orchestration (workflows and subagents)

**When the session runs on Fable 5 (effort `high`), Fable is the orchestrator,
not the implementer.** Fable plans, coordinates, reviews, and does the taste-
and intelligence-heavy work itself; it fans implementation out to subagents via
the `Agent` tool (`model: 'opus'`, which resolves to the newest Opus, currently
Claude Opus 5: 1M-token context, 128K output; the `.claude/agents/*` defs with
`model: opus` pick it up automatically) or a `Workflow` (per-agent `model`). The
main loop does not auto-delegate, so this section is the standing instruction
that it should.

Why this split: the win is context isolation, parallelism, and sparing the
orchestrator's capacity. Keep Fable's context clean and let Opus workers grind
through file-heavy implementation in parallel. It is an intelligence downgrade
per worker (see the table: Fable outranks Opus on both intelligence and cost),
so delegate by the nature of the work, never reflexively.

Rankings, higher = better. `cost` is relative spend, `intelligence` is how hard
a problem the model can be handed unsupervised, `taste` covers code quality,
API design, and clarity. Models are the ones the `Agent`/`Workflow` `model`
parameter accepts.

| model     | cost | intelligence | taste |
|-----------|------|--------------|-------|
| fable-5   | 2    | 9            | 9     |
| opus-5    | 4    | 8            | 8     |

Only these two models are used: Fable 5 orchestrates (and takes the rare
top-intelligence delegation), Opus 5 is the worker tier for everything else.
Sonnet and Haiku are not used in this project; never pass them to
`Agent`/`Workflow`, even for a mechanical pass.

How to apply (defaults, not limits; override when the output misses the bar.
Intelligence beats taste beats cost when the axes conflict for anything that
ships):

- **Orchestrator (Fable 5, high):** owns the issue loop, architecture and
  design decisions, spec-conformance judgement, and the hard bespoke logic (the
  mapping semantics of each specification, the target-side document
  construction, the terminology binding). Keeps these in context rather than delegating; they
  need top intelligence and taste and are the project's critical path.
- **Delegate to Opus 5 subagents:** bulk or parallelizable implementation on a
  clear spec (wiring handlers, DTO impls, client plumbing, test scaffolding),
  file-heavy investigation, and codebase analysis, which would otherwise burn
  the orchestrator's context. Fan out several concurrently in one message (max
  2 implementation workers, an owner cap). Opus 5's 1M-token window means one
  worker can hold a whole subsystem, so prefer one worker with the full spec up
  front over splitting a coherent task. Prompt-tune for Opus 5: it self-verifies,
  so drop "double-check your work" scaffolding from worker prompts; it also
  delegates onward readily, so tell workers NOT to spawn their own subagents.
- **Fable 5 subagents:** use when a delegated task still needs top intelligence
  or taste (a tricky algorithm, an API-shape decision) but you want it off the
  main context.
- **Reviews:** an independent read before committing a subsystem, especially
  spec and wire conformance. Spec questions and requirements extraction go to
  `spec-researcher`; bounded implementation to `implementer`. Both are defined
  in `.claude/agents/` and both are handed the governing spec sections in the
  prompt.
- Effort: keep Fable on `high`. Use `effort: 'low'` for cheap mechanical worker
  stages, higher tiers only for the hardest verify or judge stages.

Discipline is unchanged for subagents: they obey the hard rules below (never
hand-edit a generated file, no test-weakening, conventional-type branches, no
AI attribution). Delegate with a tight spec and verify the result.

## IMPORTANT hard rules

- **The specification text is the oracle.** The conformance authority is the
  HL7 FHIR specification for the version in play, the FHIRconnect
  specification, the OMOP Common Data Model, the OMOCL specification, and the
  openEHR ITS-REST specification, never memory and never another
  implementation's behaviour. Read the governing section before implementing or
  reviewing any spec-facing behaviour, and cite it (spec, page or section) for
  conformance-relevant decisions. Full policy:
  `.claude/rules/spec-adherence.md`.
- **Cite only durable references:** the FHIRconnect and OMOCL specifications
  (including published JSON schemas), the HL7 FHIR specification, the OMOP
  Common Data Model documentation, the openEHR specifications, or official
  external documentation (the Rust book and
  reference, the docs.rs page of a pinned crate). Never cite an internal
  markdown file as a design authority; internal plan documents are deleted in
  the PR that implements them. Where no specification governs a decision
  (storage mechanics, the process model, infrastructure), flag it: "no
  specification governs this: our own design".
- **Never hand-edit a `// @generated` file.** Change the generator and
  regenerate (`.claude/rules/codegen.md`).
- **Comments follow RFC 505 and RFC 1574 with hard budgets:** line comments
  only, pending work is `// TODO(#NNNN):` naming its issue, a settled decision
  is `// NOTE:` as a citation and one sentence. No essays in code; the record
  lives on the issue or PR (`.claude/rules/comments.md`, enforced by
  `scripts/checks/comment-style.sh`).
- **Prose follows `.claude/rules/writing-style.md`:** no em dashes, no
  "not X but Y", no decorative triads, no filler buzzwords.
- **Branches use conventional types** (`feat/`, `fix/`, `chore/`, `docs/`,
  `refactor/`, `perf/`, `test/`, `ci/`, `build/`, `release/`) as
  `<type>/<kebab-case-slug>`. Never force-push `main`.
- **NEVER add AI or Claude attribution** to a commit, PR, issue, comment, or
  code comment: no `Co-Authored-By`, no "Generated with", no bot trailer or
  footer, no emoji marker, ever. This is an absolute rule with no exceptions.
  A `PreToolUse` hook blocks a commit or PR command carrying one.
- **Keep the changelog.** `CHANGELOG.md` follows Keep a Changelog 1.1.0: every
  change with user-visible effect adds an entry under `[Unreleased]` in the
  same PR. Releases are cut from the changelog.
- **Never weaken, skip, or delete a test** to make a build pass, and never edit
  a test to route around a bug it exposes (`.claude/rules/testing.md`).
- **Record progress on the tracker and commit before ending a session.** Issues
  and git survive `/clear` and `/compact`; the session todo list does not.
- **Build compiling, tested increments** once code exists. Do not defer
  compilation; keep every crate you touch green.

## Licence

The project's own code and text are under the **Business Source License 1.1**
(`LICENSE`, `NOTICE`): free to read, build, modify, and redistribute, free for
every non-production use and for non-commercial production use, a commercial
licence from the Licensor for any other production use (always for a hosted,
managed, or embedded service and for for-fee distribution), and Apache License
2.0 four years after each version. Every first-party file carries
`SPDX-FileCopyrightText: Ruben Talstra` and `SPDX-License-Identifier: BUSL-1.1`
in its header. Contribution is inbound equals outbound under the same licence,
and there is no contributor licence agreement and no copyright assignment.
Vendored specifications and third-party material keep their upstream terms,
recorded in a `PROVENANCE.md` beside each vendored tree
(`.claude/rules/vendored-inputs.md`).

The decision and its history are recorded in
`.claude/memory/license-busl.md`. The one file that names Apache 2.0 as a
licence of its own is `LICENSE`, where it is the Change License.

## Working discipline (`.claude/`)

Path-scoped rules load on demand when files in their scope are read; the rest
apply always. Read the relevant one before working in that area.

- `.claude/rules/writing-style.md`: no AI tells in any prose. The top priority
  for docs and comments.
- `.claude/rules/rust-style.md`, `reliability.md`, `comments.md`, `testing.md`:
  the Rust engineering discipline (idiomatic style, safety posture, comment
  budgets, test discipline).
- `.claude/rules/spec-adherence.md`: FHIR, FHIRconnect, OMOP CDM, OMOCL, and
  openEHR ITS-REST as the oracles; strictness; cite the spec.
- `.claude/rules/codegen.md`: the generated-versus-hand-written rule, pending
  the research that fixes the boundary.
- `.claude/rules/vendored-inputs.md`: every external corpus is fetched by a
  committed `scripts/vendor/*.sh`, vendored verbatim, provenance-stamped.
- `.claude/rules/ci-cd.md`, `ai-code-review.md`: the workflow-security
  discipline and the advisory-analyzer policy (SonarQube Cloud, CodeQL).
- `.claude/rules/issue-workflow.md`, `issue-relationships.md`,
  `project-board.md`: the tracker work style.
- Skills: `/spec-lookup` (find the authoritative answer in oracle order),
  `/next-task`, `/phase-done`, `/phase-status` (the issue loop).
- Agents: `spec-researcher`, `implementer` (both on Opus 5).

## Sibling projects

`FerroEHR` (`../ferroehr`) is the reference openEHR CDR and `FerroTERM`
(`../FerroTERM`) the reference FHIR terminology server. Both are read-only
prior art from here: read their code, rules, and history freely, and never edit
either from this repository (`.claude/memory/sibling-projects.md`).

## References

- The FHIRconnect specification v1.0.0 (the FHIR mapping specification):
  <https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/index.html>
- OMOCL (the OMOP mapping specification): <https://github.com/SevKohler/OMOCL>
- The HL7 FHIR specification: <https://hl7.org/fhir/>
- The OMOP Common Data Model: <https://ohdsi.github.io/CommonDataModel/>
- The openEHR ITS-REST specification:
  <https://specifications.openehr.org/releases/ITS-REST/latest/>
- The tracker: `gh issue list --state open`. Issue #1 carries the research
  program that produces `docs/architecture.md`.
