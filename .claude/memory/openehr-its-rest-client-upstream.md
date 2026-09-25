---
name: openehr-its-rest-client-upstream
description: FerroEHR is building a rest-client feature in openehr-its (FerroEHR#3485, v4.3.2); ferrobridge-openehr is deleted when it ships
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

**How to apply:** #276 cut the restated types; #285 deleted
`ferrobridge-openehr`; the generated client lives behind
`app/ferrobridge-server/src/cdr/`, whose workarounds go with FerroEHR#3487. Take `openehr-its` with
`rest` for DTOs and `rest-client` for calls, never `rest-server`. Watch the openehr-its changelog on every dependency
sweep ([[deps-latest-sweep]], [[openehr-crates-are-the-model]]).
