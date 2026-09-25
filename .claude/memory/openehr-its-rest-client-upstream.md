---
name: openehr-its-rest-client-upstream
description: openehr-its ships the rest-client (0.0.71) and at 0.0.72 everything the bridge worked around; ferrobridge-openehr and the #293 workarounds are gone
metadata:
  type: project
---

On 2026-09-25 the owner started rubentalstra/FerroEHR#3485: a `rest` feature
exposing the ITS-REST DTOs without axum, and a `rest-client` feature emitted
by `emit-rest` from the same OpenAPI as the server traits.

**Why:** `openehr-its` 0.0.69 has no client, only DTOs and an axum server
behind `rest-server`, so FerroBRIDGE keeps a hand-written reqwest client in
`crates/ferrobridge-openehr`. That client restates id and audit types the
openEHR crates publish (FerroBRIDGE #276).

Shipped in openehr-its 0.0.71 on 2026-09-25 (FerroEHR #3486): features
`rest` (DTOs over serde and http, no axum), `rest-server`, `rest-client`
(`rest::generated::<group>::client` over `rest::client::{Transport,
ReqwestTransport, Credentials, RetryPolicy, ClientError}`).

openehr-its 0.0.72 (FerroEHR #3488, closing #3487) shipped the six items
the bridge had worked around: the commit headers as generated `*Params`
fields (`openehr_version`, `openehr_audit_details: Vec<String>`,
`openehr_template_id`), `body: ErrorBody` on every documented `4xx` outcome
variant, `validationErrors` defaulted so a `{"message": ...}` 400 decodes,
`:` and `@` literal in `path_segment`, `Credentials` over `SecretString`
with `basic`/`bearer` constructors, and `ClientError` carrying `body`.

**How to apply:** #276 cut the restated types; #285 deleted
`ferrobridge-openehr`; #293 removed every workaround in
`app/ferrobridge-server/src/cdr/` at 0.0.72. The transport there still adds
`X-Request-Id` and a read's `Prefer` (no generated parameter carries them)
and keeps the answer record, because the generated outcomes carry only the
response headers the OAS declares (the `adl2` `ETag`, a `201` contribution
`ETag` whose body fails to decode) and the prose error shape's `code` and
`errors` members. `[cdr]` credentials are the upstream `Credentials`, with no
bridge copy of the type. Take `openehr-its` with
`rest` for DTOs and `rest-client` for calls, never `rest-server`. Watch the openehr-its changelog on every dependency
sweep ([[deps-latest-sweep]], [[openehr-crates-are-the-model]]).
