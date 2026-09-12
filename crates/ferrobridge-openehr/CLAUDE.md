# ferrobridge-openehr

The hand-written ITS-REST 1.1.0 client. The oracle is the openEHR ITS-REST
specification: the three vendored OpenAPI documents under
`docs/specs/its-rest/computable/OAS/` for every path, parameter, status and
body shape, and the published overview prose for the header rules the OpenAPI
does not carry.

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
  `openehr-its` for the ITS-REST transport DTOs and the codecs, `openehr-am`
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
