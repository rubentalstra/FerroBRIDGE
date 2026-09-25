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

**How to apply:** cut the restated types now (#276), never reshape the
transport half, and delete `ferrobridge-openehr` in one step when the upstream
feature is on crates.io. Watch the openehr-its changelog on every dependency
sweep ([[deps-latest-sweep]], [[openehr-crates-are-the-model]]).
