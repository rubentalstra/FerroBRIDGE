<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Getting help

Three destinations, and they are not interchangeable. Picking the right one is
the difference between an answer and a thread nobody is paged for.

## I have a question

FerroBRIDGE is in its **design phase**: there is no code, no release, and no
documentation site yet, so there is little to configure and nothing to operate.
[`README.md`](README.md) says what the project is,
[`CLAUDE.md`](CLAUDE.md) carries the working discipline, and the research
program on
[issue #1](https://github.com/rubentalstra/FerroBRIDGE/issues/1) is where the
design decisions appear with their citations.

If those do not answer it, **open a GitHub issue** through the
[issue chooser](https://github.com/rubentalstra/FerroBRIDGE/issues/new/choose):
whether an approach fits, what a mapping construct means in this
implementation, or why a design is the way it is.

There is no commercial support offering, no service-level agreement, and no
paid tier. Answers come when the maintainer is at a keyboard
([MAINTAINERS.md](MAINTAINERS.md) is honest about how many keyboards that is).

## I found a defect

**[Open an issue](https://github.com/rubentalstra/FerroBRIDGE/issues/new/choose)**
when something is wrong, missing, or contradicts one of the specifications this
project answers to: HL7 FHIR, FHIRconnect, the OMOP Common Data Model, OMOCL,
or openEHR ITS-REST.

The reports that get fixed fastest carry:

- the version (the release tag or commit), and how it is deployed, including
  which openEHR CDR and terminology server it talks to;
- the mapping file and the input, and the output it produced, verbatim;
- what the specification says should have happened, with the section if you
  have it. A citation turns a disagreement into a defect.

**Never include patient data.** Redact it, or synthesize an equivalent case.

**A specification-conformance report is the most valuable kind here.** The
implementation is never presumed correct because it was written to the
specification; the specification text is the authority and the implementation
is the usual culprit. openFHIR, Eos, and EHRbase FHIR Bridge are prior art, so
"another bridge does it differently" is not by itself a defect; a citation is.

## I found a vulnerability

**Do not open a public issue.** Follow [SECURITY.md](SECURITY.md): report
privately through
[GitHub private vulnerability reporting](https://github.com/rubentalstra/FerroBRIDGE/security/advisories/new).

That document also carries what you can expect in return: an acknowledgement
window, an assessment window, and coordinated disclosure with credit by
default.

**A vulnerability in a dependency, or in a service you deployed alongside
FerroBRIDGE, goes to that project**, not here, unless it has a
FerroBRIDGE-specific impact. SECURITY.md § *Scope* has the routing.

## I want to change something

[CONTRIBUTING.md](CONTRIBUTING.md) is the practical guide: what helps most in a
design phase, the gates every pull request must pass, and the hard rules.
[GOVERNANCE.md](GOVERNANCE.md) is how the decision gets made and how someone
becomes a maintainer.

## What you are entitled to

Nothing, and that is worth saying plainly. FerroBRIDGE is provided as-is under
the Business Source License 1.1, with no warranty. Read the [LICENSE](LICENSE),
which says exactly that in the language that binds. Everything above describes what
the project *intends* to do, and the intent is sincere; none of it is a
contractual commitment, and only the security-report windows in SECURITY.md are
stated as promises at all.

If your deployment needs a stronger guarantee than a one-person project can
give, the honest options are to fork and maintain, or to fund the capacity that
would change the answer.
