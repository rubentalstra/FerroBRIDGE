---
paths: ["**/*.rs", "**/tests/**"]
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Testing discipline

Test discipline is non-negotiable (a standing hard rule; see `CLAUDE.md`). It
applies to every crate, generated and hand-written alike.

## The hard rule

- **Never** silently weaken, skip, or delete an existing test to make a build
  pass.
- **Never** edit a test to route around a runtime bug it exposes. If a test
  fails and the fix is unclear, leave it failing and record a
  `// TODO(#NNNN):` naming its issue; do not touch the test to make it green.
- Conformance tests assert the **FHIR, FHIRconnect, OMOP, OMOCL, and openEHR
  specifications**: cite the clause a test encodes, and never adjust an
  expectation to match an implementation bug. A fixture defect is ADJUDICATED
  with a first-hand spec citation (an expected-rejection entry in the owning
  test), never routed around by editing the case.

## Tooling

- **Runner:** `cargo-nextest` (`cargo nextest run --workspace`), not
  `cargo test`.
- **Snapshots:** `insta` pins wire output against golden vectors, the key tool
  for a bridge whose product is a document or a row set on the far side: what
  each target receives from openEHR content, and what openEHR receives back
  where the direction exists. Redact volatile fields before snapshotting.
  Review intentional changes with `cargo insta review`; never accept a
  snapshot change you have not read.
- **Properties:** `proptest` for round-trips (map forward, map back, compare)
  and for invariants a mapping must preserve.
- **HTTP mocking:** `wiremock` for both upstreams. A bridge talks to an openEHR
  CDR and a terminology server, so the unit and integration layers stub them
  and only the end-to-end layer runs real servers.
- **Benches:** `criterion` or `divan`, kept separate from correctness tests.

## Oracles

The acceptance instrument is not yet chosen; the research on issue #1 decides
it, and this section records what the oracles are regardless of the harness
that runs them.

- **The mapping specifications** (FHIRconnect for FHIR, OMOCL for OMOP) and
  their published schemas are the authority for what a mapping file may
  contain. Schema validation of a mapping is a test rather than a runtime
  nicety: an invalid mapping is refused, and the refusal is asserted.
- **The HL7 FHIR specification** for the version in play is the authority for
  every FHIR-side shape and for the FHIR REST interactions the bridge exposes
  or consumes.
- **The OMOP Common Data Model** is the authority for every OMOP-side shape.
- **The openEHR ITS-REST specification** is the authority for every call the
  bridge makes into a CDR, including the status codes and headers it must
  handle.
- **FerroEHR is the reference CDR and FerroTERM the reference terminology
  server** for end-to-end runs. Both are prior art and reference deployments,
  never the oracle: where a reference server and the specification disagree,
  the specification wins and the divergence is recorded.
- **openFHIR, Eos, and EHRbase FHIR Bridge are prior art.** A difference from
  one of them is not by itself a defect; a spec citation is.
- Prefer a specification-published example over a hand-written fixture. A test
  that encodes a spec rule cites the section it asserts
  (`spec-adherence.md`).
- **No patient data, ever.** Fixtures are synthetic content invented for the
  test. Never paste a real clinical document, a real patient identifier, or an
  extract from a production system into the repository.

## Where tests live

Unit tests live beside the code they test (`#[cfg(test)] mod tests` in the same
file), and ONLY there: **dedicated test FILES under `src/` are banned**. A test
that drives the public API belongs in the owning crate's `tests/` directory; a
test of private internals stays a small inline module. If an internals test
grows large, that is a design signal to test through the public seam, not to
split into a src file.

**One integration-test binary per crate**: the `tests/` directory is
`tests/it/main.rs` plus one `mod` per topic file, not one top-level `.rs` per
topic. Cargo compiles and links every top-level `tests/*.rs` as its own crate
("each integration test results in a separate executable binary … this can be
inefficient",
<https://doc.rust-lang.org/cargo/reference/cargo-targets.html>); nextest still
runs each test as its own process, so isolation is unchanged. Shared helpers
live in a plain module under `tests/it/`.

**A binary-only crate is untestable by construction** (Book ch11.3): its
`main.rs` cannot be imported from `tests/`. A server binary therefore keeps a
thin `main.rs` over a testable `lib.rs` run path (Book ch12.3), and its
integration tests import the lib.

## Test shapes (the Book ch11 doctrine)

- **`Result`-returning tests are the preferred shape**: `fn t() -> Result<(),
  E>` with `?` instead of unwrap chains
  (<https://doc.rust-lang.org/book/ch11-01-writing-tests.html>). Plumbing
  failures propagate with `?`, not `.unwrap()`. `clippy::panic_in_result_fn`
  fires on a Result-returning test that also asserts, and clippy offers no
  `allow-…-in-tests` knob for it, so such a test carries the lint in the same
  scoped relaxation its file uses for `panic`/`unwrap`/`expect`
  (`#![allow(…, reason = "test assertions")]` at the test-file root, or a
  `#[expect(…, reason)]` on the single test). Never relaxed at the workspace
  level and never in a non-test module.
- **`#[should_panic]` always carries `expected = "…"`:** bare `should_panic`
  passes when the code panics for the WRONG reason (Book ch11.1).
  `should_panic` is illegal on Result-returning tests; assert `value.is_err()`
  there instead.
- **Assertions**: `assert_eq!` and `assert_ne!` over bare `assert!` for
  comparisons (they print both values); a production-code assert carries a
  message.
- **Doctests are copy-paste templates**: `?` via a hidden `# Ok::<(), E>(())`
  tail or a hidden `fn main`, never `unwrap` (C-QUESTION-MARK; enforced by
  `#![doc(test(attr(deny(warnings))))]` on library roots). `no_run` for an
  example that would open a socket, `text` for non-code, never `ignore`. A
  generated crate keeps `doctest = false` deliberately (generated doc text is
  not a curated example).

## Coverage is a mandate, not just pass rate

A green suite over a thin set of cases proves almost nothing. The bar is
COVERAGE of what the specifications define on the wire:

- Every construct a mapping file may use, per mapping specification, each as
  its own small, ISOLATED case so a failure localizes to one behaviour.
- Every direction of every mapping the project ships. Each direction of each
  target is its own behaviour and gets its own cases.
- Every error family: an invalid mapping file, input a mapping does not cover,
  a CDR that refuses a call, a terminology lookup that fails, an unreachable
  upstream.
- **A spec-defined behaviour with no case is a COVERAGE GAP, never an
  acceptable omission.** Close it (a new spec-cited case) or record the honest
  boundary. Silence is not coverage.
- **Coverage only ratchets up.** Cases are added, never removed to go green.
- **One behaviour per case:** many small isolated cases beat one broad case.

## Target

Compiling, clippy-clean, tested increments at all times; a green suite is the
standing bar and every change preserves it. Green comes ONLY from fixing the
defect after spec-adjudicated attribution (`spec-adherence.md`), never from
bending a test or a fixture to match the implementation.
