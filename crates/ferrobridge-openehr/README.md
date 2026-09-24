<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# ferrobridge-openehr

The FerroBRIDGE openEHR ITS-REST 1.1.0 client with typed outcomes per status.

Part of [FerroBRIDGE](https://ferrobridge.eu), a pure-Rust bridge between
openEHR and two interoperability targets: HL7 FHIR through the FHIRconnect
specification, and the OMOP Common Data Model through the OMOCL specification.

## What it does

Every call answers with an outcome enum whose variants are the statuses the
governing ITS-REST 1.1.0 operation documents, and each non-success variant
carries the upstream body. Anything else is a typed error that keeps the
upstream status and body: a refused connection, a timeout, a `5xx`, a `401`
with its `WWW-Authenticate` challenge, a status the operation does not
document, and a body that fails to decode.

The surface is `POST /ehr`, `GET /ehr` by subject, `GET /ehr/{ehr_id}`, the
four COMPOSITION calls, `POST /ehr/{ehr_id}/contribution`, `POST /query/aql`
with a paging helper that yields rows page by page, and a template fetch that
tries `GET /definition/template/adl1.4/{template_id}` first and
`GET /definition/template/adl2/{template_id}` on a `404` or a `406`.

Four rules the specification text pins:

- `Prefer` is always sent explicitly, because the specification warns that its
  default may change.
- `If-Match` carries the bare quoted `version_uid`, with the `W/` an `ETag`
  carries stripped.
- A concurrency failure is `412` on `PUT` and `409` on `DELETE`, and each is
  its own outcome variant.
- A `204` on a composition read means the composition was deleted at the
  requested time, which is a `Deleted` outcome and never an absent value.

The model types come from the published `openehr-rm`, `openehr-its` and
`openehr-am` crates (`openehr-sdt` in the tests, for the FLAT codec); this crate models nothing the specification already
publishes.

## Status

The crate version is a placeholder while the crate line settles; the tracker
carries the build order.

openEHR is a registered trademark of the openEHR Foundation. This crate is not
endorsed by the openEHR Foundation.

## Licence

Business Source License 1.1 (`LICENSE`): free for every non-production use and
for non-commercial production use; a commercial licence for other production
use; Apache License 2.0 four years after each version.
