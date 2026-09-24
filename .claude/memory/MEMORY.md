<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Memory index

- [Product scope](product-scope.md): the owner's product statement is the
  ceiling on what this repository may claim; standalone server and
  mapping-driven are decided, everything else is research on issue #1;
  FerroEHR's in-tree FHIR extension is expected to retire in favour of the
  bridge
- [OMOP target](omop-target.md): the owner confirmed OMOP as a first-class
  target beside FHIR on 2026-09-03; FHIRconnect covers FHIR (openFHIR is its
  reference implementation) and OMOCL covers OMOP (Eos is its reference
  implementation); origin FerroEHR #2652, now FerroBRIDGE #2; claim nothing
  about OMOP beyond the product statement
- [Owner work style](owner-work-style.md): research-first and evidence-based,
  from first principles; confirm foundational decisions before scaffolding; no
  code while the design is open
- [Licence: BUSL 1.1](license-busl.md): owner decision 2026-09-04 (#12), the
  Business Source License 1.1 on FerroEHR's and FerroTERM's terms, replacing
  the 2026-09-03 Apache 2.0 choice; non-commercial production free, commercial
  production needs a licence, Apache 2.0 four years after each version;
  inbound equals outbound, no contributor licence agreement
- [Sibling projects](sibling-projects.md): FerroEHR at `../ferroehr` is the
  reference CDR and FerroTERM at `../FerroTERM` the reference terminology
  server; both are read-only prior art from here and never edited from this
  repository; a tracker issue in a sibling may be filed from here when the
  owner asks (2026-09-05, FerroTERM #300)
- [Domain ferrobridge.eu](domain-ferrobridge-eu.md): the public domain is a
  Pages setting mirroring ferroterm.eu, never a `CNAME` file; owner 2026-09-04
- [Milestones 0.0.x](milestones-0-0-x.md): milestones start at v0.0.1 and step
  by a patch number, never a v0.1.0 opener; owner 2026-09-04
- [PR auto-merge](pr-auto-merge.md): enable auto-merge on every pull request
  the moment it is opened (`gh pr merge <n> --auto --squash --delete-branch`);
  owner 2026-09-04
- [Memory lives in the repo](memory-lives-in-repo.md): every learning is a
  tracked file in `.claude/memory/`, never a per-user note; owner 2026-09-04
- [Standalone product](standalone-product.md): public documents never name
  FerroEHR or FerroTERM; any CDR, any terminology server, crates by crate name;
  the licence framing stands on its own; owner 2026-09-04
- [Crates published](crates-published.md): owner decision 2026-09-05, the
  library crates are published to crates.io on the sibling model (`crates/*`
  published, `app/*` and `tools/*` never), so the release lane carries a
  crates.io leg and every `pub` surface is designed as API from the start
- [Subagent reports go to a file](subagent-reports-to-file.md): a long
  agent report is written to the scratchpad by the agent, because a truncated
  result is lost and a finished agent cannot be resumed; 2026-09-12
- [Mermaid diagrams](mermaid-diagrams.md): the architecture carries mermaid
  diagrams; render every fence with mermaid-cli against the installed Chrome
  before a pull request; a semicolon ends a sequence message; 2026-09-12
- [CI runner memory](ci-runner-memory.md): a workspace-wide check or clippy
  is killed with exit 143 when two generated giants compile together; lint
  per package with cargo hack, never the all-features union of fhir-types;
  2026-09-12
- [PostgreSQL 18](postgresql-18.md): every PostgreSQL the project tests
  against or documents is the latest release (18.6 on 2026-09-12), never 16;
  owner ruling 2026-09-12
- [End-to-end gate](e2e-gate.md): container tests run only with
  `FERROBRIDGE_E2E=1` through the testkit harness; images pinned by digest and
  checked by the versions guard; 2026-09-12
- [Merge queue with signed commits](merge-queue-signed.md): main requires signed, up-to-date branches; rebase locally, merge one at a time, never gh pr update-branch --rebase
- [Release tag is mine](release-tag-is-mine.md): the session pushes every release and pre-release tag; never hand the tag to the owner
- [Upstream reports carry no milestone](upstream-reports-no-milestone.md): an upstream-report issue is never in a milestone; the in-repo decision it forces is a separate, milestoned issue
- [Upstream reports stay here](upstream-reports-stay-here.md): the issue is the record; nothing is filed on external trackers and no owner-action issue for filing is created
- [Dependency sweep to latest](deps-latest-sweep.md): every session compares the workspace pins with crates.io and the sibling's crate list (openehr-sdt split off openehr-its on 2026-09-24); the family lands together by hand; owner 2026-09-24
