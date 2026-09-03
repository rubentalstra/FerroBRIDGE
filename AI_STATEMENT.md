<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# FerroBRIDGE AI Statement

| | |
|---|---|
| Version | 1.0.0 |
| Effective date | 2026-09-03 |
| Status | Active |
| Author and owner | Ruben Talstra, maintainer |
| Canonical location | `AI_STATEMENT.md` at the repository root |
| Licence | Apache 2.0, like the rest of the project's own text |
| Review | at every major or minor release, and on any trigger in §13 |

**Abstract.** This document discloses how artificial-intelligence tools are
used to develop FerroBRIDGE, an open-source pure-Rust bridge between openEHR
and two interoperability targets, HL7 FHIR and the OMOP Common Data Model. It
states what the tools do and do not touch, who is accountable, which controls
bound the work and how each is enforced, the licensing and data posture, the
rules for contributors, the uses that are prohibited, and the limitations that
survive all of it. It is a self-declaration by the maintainer, written for
evaluators and regulated adopters performing supplier due diligence, and it
changes in the same pull request that changes the practice it describes.

The key words **shall**, **should**, and **may** carry the meaning ISO/IEC
Directives Part 2 gives them: requirement, recommendation, permission.

## 1. Scope

This document covers the use of AI tools in developing everything in this
repository: the bridge, its mappings handling, tests, tooling, infrastructure,
documentation, and this document itself.

It does not cover an AI system in the product, because there is none.
**FerroBRIDGE ships no AI.** No model is trained, embedded, or called at
request time, and no inference happens anywhere in the served software. The
bridge transforms content according to mapping files that a human wrote and a
schema validated. AI is used to *build* the software, in the same sense
compilers and linters are used to build it.

The project is in its **design phase**, so most of the work disclosed here is
research and the repository's own working configuration rather than product
code. That does not change any rule below.

## 2. Which frameworks apply here, and which do not

Stated plainly, because borrowed authority is worse than none:

- **The EU AI Act imposes no obligation on this project.** The Act binds
  providers and deployers of AI *systems* (Articles 2 and 3(1)); FerroBRIDGE is
  neither, because it contains no AI system. Article 50's marking duties bind
  the AI tool's provider rather than the tool's user, and the European
  Commission's Article 50 FAQ places source code outside the content-marking
  obligation. This document is voluntary.
- **Medical-device regulation does not classify this project as a device.**
  Under MDCG 2019-11 Rev. 1, software is classified by its intended medical
  purpose, and software that retrieves, transforms, or communicates information
  without interpreting it for an individual patient does not qualify as a
  medical device. A format bridge is that kind of software. A downstream
  integrator who gives their product a medical purpose may bring *their*
  product into scope; that classification is theirs to make, and this document
  exists partly so they can answer their own supplier questions.
- **ISO/IEC 42001 and the NIST AI RMF are used as vocabulary, not claimed as
  conformity.** No certification is claimed, no audit has occurred, and the
  words "certified", "audited", and "validated" appear in this document only
  inside this sentence, to say they do not apply.

## 3. Terms

This document reuses the W3C AI Content Disclosure vocabulary rather than
inventing one: **none** (entirely human-authored), **ai-assisted**
(human-authored; AI edited, refined, or filled in boilerplate),
**ai-generated** (AI-generated with human prompting and review), **autonomous**
(AI-generated without meaningful human oversight). An **agentic tool** is one
that plans and executes multi-step work, editing files and running builds and
tests, under a human's direction, as opposed to inline completion.

## 4. Accountability

One named human, the maintainer listed in [MAINTAINERS.md](MAINTAINERS.md), is
the author of and accountable for every change in this repository, whatever
tool produced the bytes. A tool **shall not** be named as an author,
co-author, or signer of anything here, because a tool cannot be responsible for
accuracy, integrity, or originality; responsibility that cannot be borne cannot
be assigned. There is no AI-issued sign-off of any kind.

## 5. Where AI is used, and at what level

The tooling is agentic AI coding assistance (currently Claude Code, by
Anthropic), operated in sessions the maintainer directs, reviews, and merges.
Levels below use the §3 vocabulary, per development activity. Deliberately, no
percentage appears anywhere in this document: no defensible method exists for
measuring one.

| Activity | Level | Notes |
|---|---|---|
| Research and specification reading | ai-generated | conducted against the published specifications and the prior art, with citations; the conclusions are the maintainer's |
| Bridge, tooling, and infrastructure code | ai-generated | written in directed sessions against the specifications this project answers to; reviewed and merged by the maintainer |
| Tests | ai-generated | held to the same authority as the code they test: expectations cite the specification, and the attribution discipline below governs failures |
| Documentation and this statement | ai-generated | held to the repository's own prose rules; the research grounding this document's structure is recorded on the project tracker |
| Specification adjudications, owner rulings, release decisions | none | decided by the maintainer and recorded with reasoning on the issue tracker |
| Contribution and review verdicts on others' work | none | prohibited use; see §11 |

**autonomous** appears in no row, and that is the point of the next section.

## 6. Human oversight

The maintainer directs the work, reads the result, and merges every change.
Nothing lands on its own authority, and no merge is automated. Where the tools
run multi-step sessions, the session's decisions with consequences (what a
specification silence means, what a mapping must do, what ships in a release)
are the maintainer's, recorded on the tracker
([GOVERNANCE.md](GOVERNANCE.md) § Where decisions are recorded). A decision
that exists only inside a tool session is not a decision this project made.

## 7. Quality controls, and what each one proves

AI-produced work is not a shortcut around engineering process. Every change,
whoever or whatever wrote it, passes the same machine-enforced gates. Each
control below names its enforcement, because a control without a failing check
is a wish. That principle is itself written down, in
[`.claude/rules/reliability.md`](.claude/rules/reliability.md), together with
the full enforcement register.

- **Specification authority.** Five sources are the oracle for every
  spec-facing behaviour: the HL7 FHIR specification for the version in play,
  FHIRconnect, the OMOP Common Data Model, OMOCL, and openEHR ITS-REST.
  Spec-facing decisions cite the section
  ([`.claude/rules/spec-adherence.md`](.claude/rules/spec-adherence.md)).
  Enforced by review against the cited text and by the tests.
- **Executed conformance.** Mapping files validate against the published
  schemas, and the transformations are tested per direction and per target
  ([`.claude/rules/testing.md`](.claude/rules/testing.md)). This is the control
  that catches a plausible-but-wrong implementation regardless of who wrote it.
  The acceptance instrument is an output of the research program, and this
  bullet gains its name when that closes; claiming one now would be a claim
  with nothing behind it.
- **Failure attribution.** A failing test is adjudicated against the
  specification first, and the implementation is never presumed correct because
  it was written carefully. A reference implementation is prior art, so a bug
  in openFHIR, Eos, or EHRbase FHIR Bridge is not a requirement. Tests,
  fixtures, and expectations **shall not** be weakened to make a build pass; a
  fixture defect is adjudicated with a first-hand citation, never routed around
  ([`.claude/rules/testing.md`](.claude/rules/testing.md)).
- **Generated layers.** Where a specification model is generated from
  machine-readable input, a drift check regenerates in CI and fails on any
  diff, and a `// @generated` file is never hand-edited
  ([`.claude/rules/codegen.md`](.claude/rules/codegen.md)).
- **Static and supply-chain gates.** No `unsafe` (compiler-`forbid`),
  deny-tier lints on panicking shortcuts, typed errors, machine-checked comment
  style ([`.claude/rules/comments.md`](.claude/rules/comments.md)), `cargo
  deny` policy, `actionlint`, `zizmor`, `shellcheck`, and CodeQL over the
  workflows ([`.claude/rules/ci-cd.md`](.claude/rules/ci-cd.md)). The Rust half
  activates with the Cargo workspace; the workflow and shell half runs today.
- **A second machine opinion.** SonarQube Cloud runs on every pull request and
  every push to `main`; its findings are advisory, never outrank the
  specifications or the gates, and it commits nothing
  ([`.claude/rules/ai-code-review.md`](.claude/rules/ai-code-review.md)).

What these controls do **not** prove is stated in §12.

## 8. Licensing and provenance of AI output

The project is Apache 2.0 licensed. The position taken here follows the Apache
Software Foundation's and LLVM's published reasoning rather than wishful
shortcuts: an AI tool's output does not launder anyone's copyright, the full
provenance of generated text is generally not knowable, and prompting alone is
not treated as authorship. In practice: a contribution of substantially copied
third-party material is refused however it was produced; generated code is held
to the same originality expectations as human code, under the same review; and
if identifiable third-party material is found in the tree, it is removed or
licensed properly, exactly as it would be for a human-introduced copy. The
tools are used under terms that do not restrict the output's use in Apache 2.0
licensed software.

## 9. Data

No patient data, no personally identifiable health information, and no customer
data exists anywhere in this project. It is not in the repository, the test
fixtures, or the telemetry, and therefore not in any prompt. This matters more
for a bridge than for most software, because a bridge sits in the path of
clinical content and a reproduction is easy to build from real data by
accident: fixtures are synthetic content invented for the test, and a report
that carries real content is redacted rather than absorbed
([SECURITY.md](SECURITY.md), [SUPPORT.md](SUPPORT.md)). This is a structural
property a reader can check against the tree rather than a promise about tool
behaviour. Vendor-side data handling is governed by the tool vendor's terms;
this document makes no claim on the vendor's behalf, because such claims go
stale silently.

## 10. Rules for contributors

Contributors **may** use AI tools. A contribution with **ai-generated** content
per §3 **shall** say so in the pull-request description: which tool, and what
it did. Disclosure lives in the pull-request description and never in a commit
trailer, for two reasons stated openly. This repository's standing rule keeps
tool attribution out of commit metadata (one maintained disclosure beats ten
thousand trailer lines, and this document is that disclosure), and the wider
ecosystem has no agreed trailer anyway; the same trailers some communities
recommend, others forbid. The contributor remains responsible for their
submission in full, under the same [CONTRIBUTING.md](CONTRIBUTING.md) bar as
any other work: understood, explained on request, tested, and honest.

## 11. Prohibited uses

In this project, AI **shall not**: merge anything; adjudicate, score, or answer
reviews of contributions (the machine reviewer of §7 is advisory input to the
maintainer rather than a verdict); sign anything; decide a spec-facing question
(a silence is adjudicated by the maintainer and recorded); or weaken a test, an
expectation, or a gate to make something pass, the last being a standing hard
rule for humans and tools alike.

## 12. Limitations and residual risks

This section exists because a disclosure without one is marketing.

- **The gates prove what they test, not correctness.** The tests demonstrate
  the behaviours their cases cover; coverage is meant to ratchet upward and is
  itself reviewed, and it is still a boundary.
- **There is no acceptance instrument yet.** The project is in its design
  phase; the conformance harness is an output of the research program, so
  today's controls are the rule set, the workflow gates, and review.
- **End-to-end verification needs two other servers.** A bridge is only fully
  exercised against a real openEHR CDR and a real terminology server, so those
  runs are heavier than a unit test and will be a periodic gate rather than a
  per-pull-request one.
- **Review depth is one person's.** The project has a single maintainer;
  machine gates stand in for the review capacity a larger team would have
  ([GOVERNANCE.md](GOVERNANCE.md) says this plainly). "The maintainer
  understands and can explain every merged change" is the honest claim; "every
  line was independently re-derived" would not be.
- **Retroactivity.** Commits predating this statement carry no disclosure
  markers; this document describes the practice rather than a per-commit audit
  trail, and no such trail is claimed.
- **Provenance uncertainty survives.** Whether any generated fragment echoes
  unlicensed training material is not fully knowable with current tools; §8
  states the handling rather than a guarantee.
- **The legal ground is unsettled.** Copyright in AI output is an open question
  in most jurisdictions; this document records positions, and positions may
  have to change. §13 names the triggers.
- **This is a self-declaration.** No third party has audited it. The checkable
  artifacts in §7 are the counterweight: they can disagree with this document,
  and if they do, the document is wrong.

## 13. Review and change

This statement is reviewed at every major or minor release, and revised
off-cycle when any of these fires: the tooling changes materially, a tool
vendor's terms change in a way §8 or §9 relies on, a binding rule emerges (EU
AI Act guidance touching this use, a foundation policy this project follows, a
court decision on AI output and copyright), or a claim in this document stops
being true. The maintainer owns the review; the change lands as a pull request
like everything else, and the version and change log update in the same pull
request.

## 14. Reporting

A suspected provenance, licensing, or quality problem in this repository,
including a claim in this document that does not survive checking, is a report
this project wants. Open an issue and cite this file; for anything
security-sensitive, use the private route in [SECURITY.md](SECURITY.md). The
handling commitment is the same as for any defect: attributed, answered on the
tracker, and never silently absorbed.

## 15. References

**Normative for this project** (the documents that bind the practice described
here): the [Apache License 2.0](LICENSE); the HL7 FHIR, FHIRconnect, OMOP
Common Data Model, OMOCL, and openEHR ITS-REST specifications; the repository's
rule set (`.claude/rules/`, in particular `spec-adherence.md`, `testing.md`,
`reliability.md`, `codegen.md`, `comments.md`, `ai-code-review.md`,
`writing-style.md`, `ci-cd.md`); [GOVERNANCE.md](GOVERNANCE.md),
[MAINTAINERS.md](MAINTAINERS.md), [CONTRIBUTING.md](CONTRIBUTING.md),
[SECURITY.md](SECURITY.md).

**Informative** (the sources this document's structure and positions draw on,
inherited from the sibling projects' own survey): the W3C AI Content Disclosure
vocabulary; the ISO/IEC Directives Part 2 document conventions and verbal
forms; RFC 7322's required-considerations discipline; ICMJE's AI-authorship
position; the Apache Software Foundation's generative-tooling guidance; the
Linux Foundation's generative-AI policy; the Fedora Council's
AI-assisted-contributions policy; the Linux kernel, LLVM, Kubernetes, NumPy,
Mozilla, QEMU, curl, Gentoo, OpenInfra, Ghostty, Kyverno, and nf-core
positions; the OpenSSF security guidance for AI code assistants and the
OpenSSF/CNCF maintainer guide; NIST AI RMF and ISO/IEC 42001 as vocabulary; EU
AI Act Articles 2, 3, and 50 with the European Commission's Article 50 FAQ;
MDCG 2019-11 Rev. 1.

## Annex A. Change log

| Version | Date | Change |
|---|---|---|
| 1.0.0 | 2026-09-03 | First issue. |

## Annex B. Machine-readable summary

Levels per the W3C AI Content Disclosure vocabulary (§3); the prose above is
authoritative where the two could ever disagree.

```yaml
ai-statement:
  version: 1.0.0
  last-updated: 2026-09-03
  vocabulary: w3c-ai-content-disclosure
  disclosure-default: ai-generated
  tools:
    - name: Claude Code
      provider: Anthropic
  processes:
    research: ai-generated
    design: ai-assisted
    implementation: ai-generated
    testing: ai-generated
    documentation: ai-generated
    review: none
    adjudication: none
    release-decisions: none
  ships-ai-system: false
  autonomous-use: none
```
