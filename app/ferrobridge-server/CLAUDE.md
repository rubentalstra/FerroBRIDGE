# ferrobridge-server

The one binary. `main.rs` is thin over `lib.rs` so the integration tests drive
the real run path; a binary-only crate cannot be imported from `tests/`
(<https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-testing.html>).

- **`main.rs` never grows.** It calls `run(std::env::args())` and returns the
  `ExitCode`. Everything else lives in a module of the library.
- **Every configuration default lives inline in its struct's `Default` impl**,
  with container-level `#[serde(default, deny_unknown_fields)]`
  (`.claude/rules/rust-style.md` §Default values). The per-field
  `#[serde(default = "path")]` form is banned, and so is a `fn default_x()`.
- **A new configuration key is three changes in one edit**: the field with its
  default in `src/config/section.rs` (the file tree), the Operate page's
  variable table in `website/book/src/operate/`, and a case under
  `tests/it/config/`. A key with no documented variable name is a key an
  operator cannot set. `src/config/` holds the tree and its resolved settings
  in `mod.rs`, the environment overrides in `overrides`, each lane's
  resolution in `resolve` and the upstream credentials in `credentials`.
- **A secret is reachable through a `<key>_file` sibling, read once at boot**,
  and lives in a `SecretString` after that. A value and its `_file` together is
  a boot error, and so is an unknown key. The refusal names the key.
- **No clinical content in any log line.** The request log carries the matched
  route, never the raw URL, and a query value only from the allowlist. A body
  never reaches the log, a `Debug` rendering, or an error message. Adding a
  field to `request_log` needs the same judgement as adding one to a wire body.
- **An upstream failure is a typed error carrying the upstream status.** A
  readiness indicator reports a reason; it never reports up because a call
  failed quietly.
- **The panic catcher exists because the release profile pins
  `panic = "unwind"`.** Keep both: an `abort` regression would make the `500`
  untestable by construction
  (<https://doc.rust-lang.org/cargo/reference/profiles.html#panic>).
- **A status is a `StatusCode`**, compared as one. Every CDR answer becomes a
  facade answer through the one table in `facade/status.rs`, and a new row
  carries both citations (the ITS-REST operation, the R4 HTTP section) plus a
  wire test. A handler never picks a status of its own beside it.
- **Tests are one binary**, `tests/it/main.rs` plus one module per topic, and
  they drive the public seams: `run`, `Config::from_sources`, `router`,
  `with_middleware`, `serve_until`. A test that needs its own route builds one
  and hands it to `with_middleware`, so the stack under test is the shipped
  stack.
- The upstream stubs are `wiremock`; every fixture is synthetic.

## The CDR client (`src/cdr/`)

- **The generated client owns every operation.** The paths, the parameters,
  `Prefer`, `If-Match`, the outcome enum per call and the retry are the
  `openehr_its::rest::generated::<group>::client` operations over one
  `openehr_its::rest::client::Client<ReqwestTransport>`; the commit headers
  are fields of the generated parameters since 0.0.72 (`CommitHeaders`). The
  module adds only what they lack: the `X-Request-Id` echo, the answer kept
  beside each outcome for the `ETag` of an adl2 template or of a `201` whose
  body does not decode, the ids an `ETag` names, the two-route template fetch
  and the AQL paging. A gap in the generated client is a request to
  `openehr-its`, never a hand-written copy of the operation here. `mod.rs`
  holds the handle and the answer helpers; the `CdrClient` calls sit in
  `ehr`, `composition` and `contribution`, one child per resource group.
- **A documented status is an outcome, never an error**, and a refusal keeps
  the status and the `ErrorBody` the outcome or `ClientError` carries, so the
  facade's status table diagnoses it (`facade::status::diagnostics`).
- **The commit headers carry the 1.1.0 value form** (`openehr-version`,
  `openehr-audit-details`, `openehr-template-id`) and never the deprecated
  1.0.3 spelling.
- **Tests are `wiremock` contract tests under `tests/it/cdr/`**, one case per
  documented status plus the request-side assertions; every body is synthetic.

## The facade (`src/facade/`)

- **The facade never stores.** The CDR holds the clinical record; the facade
  holds identity and nothing else. A cache of resource content, a local copy of
  a composition, or a column with a mapped value in the identity store all
  break the premise, and a test greps the store file for a mapped marker to
  prove it stays out.
- **Two error vocabularies never mix.** The CDR speaks `{error, message,
  validationErrors}` and the wire speaks `OperationOutcome`. Everything the
  facade authors is an `OperationOutcome` with an `issue.code` from the R4
  value set; the openEHR body travels verbatim inside `issue.diagnostics` and
  never reaches the wire as its own document.
- **Ids are newtypes.** `EhrId` comes from the `cdr` module, the version
  container and the version are the `openehr-base` `HierObjectId` and
  `ObjectVersionId`; `FhirResourceId`, `ExternalResourceId` and
  `PersonId` are this side's. A function never takes a bare `String` where one
  of them belongs, so a swapped argument is a compile error.
- **The map wins once written.** A derived id is derived once. Changing the
  derivation must never rename a resource a client already holds, so the store
  is read before the derivation runs.
- **Clinical content reaches neither a log line nor the identity store.** A
  handler logs the matched route, the status and identifiers. A resource body,
  a composition, and a CDR error body stay out of `tracing`, out of a `Debug`
  rendering, and out of `redb`.
- **A request path never panics.** No indexing, no slicing, no `unwrap`: every
  refusal is a typed `Refusal` that renders as an `OperationOutcome`.
- **The handlers own HTTP; `facade::ingest` owns the write.** A handler checks
  the media types, parses the body, reads the `templateId` pin,
  `If-None-Exist`, `If-Match` and `Prefer`, and renders the answer. Mapping,
  EHR resolution, the commit and the identity record are the `Ingest`
  service's, which takes plain values and answers a `Refused` (status and
  issues, unrendered) that becomes a `Refusal` through `From`. Every face that
  writes into the CDR calls that one service: a second map-and-commit loop is
  the duplication it exists to prevent. The Bundle path's rule for an entry no
  program maps is explicit on the call (`UnmappedEntries`), and the
  transaction route passes `Refuse`. `src/facade/ingest/` is cut by path:
  `single` (the single create and update), `transaction` and `entries` (the
  Bundle), `reconcile` (a delivery an earlier one committed or consumed),
  `pairing` (a contribution's versions to the entries sent, with the content
  tie-break), `claim`, `binding` (the engine run and the identity record) and
  `refusal`; the public types stay in `mod.rs`.
- **The HL7 v2 face (`src/hl7v2/`) is a face, never a second pipeline.** It
  owns the MLLP listener, the message's span and the acknowledgment; the
  mapping is `ferrobridge-hl7v2`'s and the write is `Ingest::ingest_bundle`
  under the facade's programs, identity map and claims
  (`Facade::ingest_under` with the face's EHR policy). The only Bundle edits
  it makes before the hand-off are the configured `meta.profile` claims and
  the subject reference to the message's one `Patient`, both flagged as our
  own design. `hl7v2::answer` is the one place an outcome becomes `AA`, `AE`
  or `AR`: a status no content change clears is `AR`, every other refusal
  `AE`. A message byte never reaches a log; the spans carry MSH-10 and the
  message type.
- **A new interaction is four changes in one edit**: the route, the
  `CapabilityStatement` it is declared in, the wire test, and the Integrate
  page. An interaction the statement does not declare answers `404`, and one it
  declares and the router does not mount is a lie.
