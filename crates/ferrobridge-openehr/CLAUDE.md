# ferrobridge-openehr

The hand-written ITS-REST 1.1.0 client transport, and nothing else: the
reqwest calls, the retry budget, credentials and TLS, `Prefer` and `ETag`
handling, the commit headers, and one outcome enum per call. The oracle is the
openEHR ITS-REST specification: the three vendored OpenAPI documents under
`docs/specs/its-rest/computable/OAS/` for every path, parameter, status and
body shape, and the published overview prose for the header rules the OpenAPI
does not carry. The crate is deleted in one step when `openehr-its` ships its
generated `rest-client` feature (FerroEHR#3485), so the transport half is not
reshaped in the meantime.

- **The crate owns no openEHR model type.** The version, version container,
  template and `UID_BASED_ID` identifiers are the `openehr-base` BASE 1.3
  types the `openehr-its` DTOs use (`ObjectVersionId`, `HierObjectId`,
  `TemplateId`, `UidBasedId`); the audit a commit states is the ITS-REST
  `UpdateAuditData` over the RM `DvCodedText`, `DvText` and `PartyProxy`; the
  ADL 2 HRID is the `openehr-am` `ArchetypeHrid`. `ids.rs` holds only the thin
  readers the wire needs over them (`entity_tag`, `version_from_etag`,
  `versioned_object_uid`, `template_id`) and the handles no BASE type fits
  (`EhrId`, `ContributionUid`, `SubjectId`, `SubjectNamespace`, `RequestId`),
  each with its reason beside it. Consumers import the upstream types from
  their own crates; this crate re-exports none.

- **A documented status is an outcome, never an error.** Each endpoint has one
  outcome enum with a variant per status its operation documents, and every
  non-success variant carries the upstream body. Everything else is a typed
  error in `error.rs` that keeps the upstream status and body. Nothing is ever
  flattened into `None` or a default.
- **Read the `responses` of the operation before adding or changing a
  variant**, and cite the file and the operation id in the doc comment.
- **The commit metadata headers live in the prose only.** `openehr-version`,
  `openehr-audit-details` and `openehr-template-id` are absent from the three
  OpenAPI documents; the client sends the 1.1.0 value-carrying form and never
  the deprecated 1.0.3 spelling, whose attribute path sat in the header name.
- **The model comes from the published crates**: `openehr-rm` for the RM,
  `openehr-its` for the ITS-REST transport DTOs and the codecs, `openehr-sdt`
  for the Simplified Data Template formats, `openehr-am`
  for the AOM2 operational template. Never hand-write a type one of them
  already defines.
- **Retry is bounded and idempotent-only.** `GET`, `PUT` under `If-Match` and
  `DELETE` of a version may be retried on a connect failure, a timeout or a
  `5xx`; a `POST` is sent once. The predicate is `Error::is_retryable` and it
  is tested.
- **Tests are `wiremock` contract tests**, one case per documented status, plus
  the request-side assertions for `Prefer`, `If-Match`, the commit headers and
  `Accept`. Every body is synthetic: no clinical content, no real identifier.
- The client logs identifiers and statuses through `tracing`, never a body.
