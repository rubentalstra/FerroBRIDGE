<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the worked examples of the FHIRconnect REST API chapter

Each file is one `[source,json]` block of the draft chapter, copied verbatim
from the vendored text at its pinned commit. They are the wire fixtures the
contract tests round trip, so a change to the chapter shows up as a failing
test rather than as silent drift. The chapter is an **unmerged draft**, pull
request #93 of the FHIRconnect specification, so these examples are not part of
any release.

- Source: `docs/specs/fhirconnect/draft-rest-api/modules/ROOT/pages/engine/rest-api.adoc`
- Upstream: <https://github.com/SevKohler/FHIRconnect-spec/pull/93>
- Pin: pull request #93 at head commit `2bf2a2fe91bae2ae659cda1665826567ea81b4af`
- Extracted: 2026-09-15
- Upstream licence: Apache License 2.0, the `LICENSE` file of the same
  repository, vendored at `../../../../../docs/specs/fhirconnect/LICENSE`

| File | Block in `rest-api.adoc` | sha256 |
|---|---|---|
| `tofhir-canonical.request.json` | §$tofhir Input, `.Canonical Composition` | `d62c5512034ba50a1cfea1f7a8e8850e3b9b38051409c895b24381cd6573d3f8` |
| `tofhir-flat.request.json` | §$tofhir Input, `.Flat Composition` | `94c38e9e05f8a69f5d24ca28201be18600132dfb96237a0d9412c8215b5f6d66` |
| `tofhir-direct-canonical.composition.json` | §Direct payload invocation, `.Canonical Composition, sent directly` | `16806ca95516d82b401ffedd9641f5dabf540386a679f081e6e89dbd4ff5b4cb` |
| `tofhir-direct-flat.composition.json` | §Direct payload invocation, `.Flat Composition, sent directly` | `89816b22f1a0a1d0ff49377c7ff08046ab59abed69c4d39cc747799ad389906f` |
| `toopenehr-canonical.response.json` | §$toopenehr Output, ``.`format=canonical` (default)`` | `22912a40eaca339c6dc56563119235b046cdf6f8c63dd696ee4e67359625ed23` |
| `toopenehr-flat.response.json` | §$toopenehr Output, ``.`format=flat`` `` | `4d3592f669122866eac20503477e83d33cdb7e4a54a8d09fa83cc0fa1a07ddbc` |
| `toopenehr-outcome.response.json` | §Reporting partial results with `OperationOutcome` | `29098922b016ca84802d81b8a7b805c33db52deed8381345e3c6641e6803b54f` |
