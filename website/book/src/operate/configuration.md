<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Configuring the server

FerroBRIDGE reads an optional TOML file, then environment variables over it.
Nothing else configures the process. No specification governs this; it is
FerroBRIDGE's own design.

<!-- toc -->

## Where the file comes from

`ferrobridge serve --config /etc/ferrobridge/config.toml` names the file. With
no flag, `FERROBRIDGE_CONFIG` names it. With neither, the defaults below stand
and the server serves with no upstream configured.

## How an environment variable maps to a key

Prefix `FERROBRIDGE__`, then the dotted key with two underscores between
levels. Case does not matter; single underscores inside a key name stay as
they are.

```text
FERROBRIDGE__SERVER__LISTEN            -> [server] listen
FERROBRIDGE__CDR__BASE_URL             -> [cdr] base_url
FERROBRIDGE__CDR__RETRY__MAX_ATTEMPTS  -> [cdr.retry] max_attempts
```

A value is read as TOML syntax when it is valid TOML, and as plain text
otherwise. `5` is the number 5, `true` is a boolean, `["a","b"]` is an array,
and `http://cdr.example/v1` is a string. Quote a text value that would
otherwise read as a number: `FERROBRIDGE__CDR__CREDENTIALS__BEARER_TOKEN='"0123"'`.

## The variables

Every key below is settable in the file and as `FERROBRIDGE__<SECTION>__<KEY>`.
A key marked secret also accepts a `<key>_file` sibling naming a file the
server reads once at boot, with surrounding whitespace trimmed.

### `[server]`

| Key | Default | Meaning |
|---|---|---|
| `listen` | `127.0.0.1:8080` | The socket address to bind |
| `request_timeout_ms` | `30000` | How long one request may take before the server answers `408` |
| `shutdown_timeout_ms` | `10000` | How long the drain may take after `SIGTERM` or `SIGINT` |
| `body_limit_bytes` | `1048576` | The largest request body read before the server answers `413` |

### `[telemetry]`

| Key | Default | Meaning |
|---|---|---|
| `format` | `auto` | `auto`, `json` or `pretty`; `auto` is `pretty` on a terminal and `json` otherwise |
| `filter` | `info,hyper=warn,tower=warn,h2=warn` | A `RUST_LOG`-style directive; a directive that does not parse falls back to this default and says so once |
| `logged_query_parameters` | `[]` | The query parameter names whose values may reach the request log, each value cut at 64 characters |

Nothing else about a request is logged. Add a name to
`logged_query_parameters` only for a parameter you know carries no identifying
content.

### `[cdr]`

The openEHR CDR lane. Absent, the bridge holds no CDR client and readiness
reports no CDR indicator. This section reaches identifiable data, and the
server says so once at start-up, naming the section and never a value.

| Key | Default | Secret | Meaning |
|---|---|---|---|
| `base_url` | none, required | | The openEHR REST API root, usually ending in `/v1` |
| `timeout_ms` | `30000` | | How long one call may take, connection included |

### `[cdr.retry]` and `[terminology.retry]`

| Key | Default | Meaning |
|---|---|---|
| `max_attempts` | `3` | How many times a call is sent at most, the first try included; `1` disables retry |
| `initial_backoff_ms` | `200` | The delay before the second attempt |
| `max_backoff_ms` | `5000` | The ceiling every later delay is clamped to |

### `[cdr.credentials]` and `[terminology.credentials]`

Set one scheme. A bearer token and a user together is a boot error, and so is
an inline value beside its `_file` sibling.

| Key | Default | Secret | Meaning |
|---|---|---|---|
| `bearer_token` | none | yes | An RFC 6750 bearer token |
| `user` | none | | The user name of RFC 7617 basic authentication |
| `password` | none | yes | The password of RFC 7617 basic authentication |

### `[terminology]`

The FHIR terminology lane. Absent, the bridge holds no terminology client, and
the engines fail closed on anything that would need one.

| Key | Default | Meaning |
|---|---|---|
| `base_url` | none, required | The FHIR service base URL |
| `wire_version` | `r4` | `r4` or `r4b`, the release the server answers in |
| `timeout_ms` | `30000` | How long one call may take, connection included |

### `[cdm]`

The OMOP CDM database lane. This section reaches identifiable data, and the
server says so once at start-up.

| Key | Default | Secret | Meaning |
|---|---|---|---|
| `url` | none, required when the section is present | yes | The PostgreSQL connection URL |

### `[mappings]`

The FHIRconnect mapping set this deployment runs. The facade, the two
FHIRconnect operations and the mapping subcommands read the same set. The set
and its templates are read once at boot, and a mapping that does not compile
refuses the start rather than the first request that touches it.

| Key | Default | Meaning |
|---|---|---|
| `directory` | none | The directory the mapping files are read from, recursively (`.yml`, `.yaml`) |
| `templates` | none | The directory holding the operational templates the two operations compile against, as OPT 1.4 XML (`.opt`) |

The facade always takes its templates from the CDR and needs `directory`
alone. The two FHIRconnect operations take theirs from one of two sources:

- With `templates` set, every `.opt` file in that directory is read. This
  source wins when `[cdr]` is configured too, because the directory is the
  explicit choice, and the operations then run with no CDR at all.
- With `templates` unset and `[cdr]` configured, the server reads the context
  files first and fetches each template they name from the CDR
  (`GET /definition/template/adl1.4/{template_id}`, openEHR ITS-REST 1.1.0
  §Definition). A template the CDR does not hold, or any other refused fetch,
  stops the start with an error naming the template and the status the CDR
  answered, so the lane never comes up with part of its mappings.

A `directory` with neither `templates` nor `[cdr]` is refused while the
operations are enabled. The server checks this when it reads the
configuration, before it calls any upstream, and the refusal names both
sources. With `[operations] enabled = false`, `directory` alone
is enough for the facade.

```toml
# The operations compile against local templates and reach no CDR.
[mappings]
directory = "/etc/ferrobridge/mappings"
templates = "/etc/ferrobridge/templates"
```

```toml
# The operations compile against the templates the CDR serves.
[cdr]
base_url = "https://cdr.example.org/openehr/v1"

[mappings]
directory = "/etc/ferrobridge/mappings"
```

### `[facade]`

The FHIR R4 facade. It is off until `enabled` is true, and a disabled facade
mounts no route, so a request to `/fhir` answers `404` rather than `403`. An
enabled facade needs `[cdr]` and `[mappings] directory`: it compiles the
mapping set at boot against the templates the CDR holds, so a mapping that does
not compile refuses the start rather than the first request that touches it.
This lane reaches identifiable data, and the server says so once at start-up.

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Whether the facade routes are mounted |
| `base_url` | `http://127.0.0.1:8080/fhir` | The absolute FHIR service base a client reaches, which `Location` is written under |
| `ehr_policy` | `existing` | `existing` writes only into an EHR the CDR already holds; `create_on_first_write` creates one for an unknown subject |
| `identity_store` | `identity.redb` | The file the identity map is kept in, opened at boot and refused when it cannot be written |
| `subject_namespace` | `ferrobridge` | The namespace a subject identifier is looked up in on the CDR |
| `system_id` | `FerroBRIDGE` | The `AUDIT_DETAILS.system_id` every commit records |
| `composition_language` | none, required | The `COMPOSITION.language` every commit carries, an ISO 639-1 code |
| `composition_territory` | none, required | The `COMPOSITION.territory` every commit carries, an ISO 3166-1 code |

The two composition fields have no default. FHIRconnect puts them on the
project performing the mapping, and there is no correct language for a clinical
record you did not write, so an enabled facade with either key unset is a boot
error naming the key.

The identity store holds identifiers: a subject to its `ehr_id`, a resource id
to the composition it came from, and the source identity of every resource the
facade consumed. No clinical content is written into it. Back it up with the
CDR, because a lost map means the next create writes a second composition for a
resource the CDR already holds.

### `[operations]`

The `$tofhir` and `$toopenehr` lane. Both operations are pure transformations
and reach no CDR on a request, so they are served whenever `[mappings]
directory` is set and its templates load, from `[mappings] templates` or from
the CDR at boot. Without a mapping directory the two routes answer `503`.

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Whether the two operations and their direct forms are served |
| `device_reference` | `Device/ferrobridge-<version>` | The `Provenance` `agent.who` a call that supplies no `context.who` gets |
| `composer` | `FHIRconnect` | The composition composer an inbound run fills in when no mapping does |
| `composition_language` | none | The composition language, an ISO 639-1 code |
| `composition_territory` | none | The composition territory, an ISO 3166-1 alpha-2 code |

The FHIRconnect engine chapter puts the composer and the context start time on
the engine and the composition language and territory on the project performing
the mapping, so set the last two: a template whose reference model requires
them refuses to build a composition without them, and `$toopenehr` then answers
an `OperationOutcome` naming the missing field.

## The secrets, in one place

| Variable | File sibling |
|---|---|
| `FERROBRIDGE__CDR__CREDENTIALS__BEARER_TOKEN` | `FERROBRIDGE__CDR__CREDENTIALS__BEARER_TOKEN_FILE` |
| `FERROBRIDGE__CDR__CREDENTIALS__PASSWORD` | `FERROBRIDGE__CDR__CREDENTIALS__PASSWORD_FILE` |
| `FERROBRIDGE__TERMINOLOGY__CREDENTIALS__BEARER_TOKEN` | `FERROBRIDGE__TERMINOLOGY__CREDENTIALS__BEARER_TOKEN_FILE` |
| `FERROBRIDGE__TERMINOLOGY__CREDENTIALS__PASSWORD` | `FERROBRIDGE__TERMINOLOGY__CREDENTIALS__PASSWORD_FILE` |
| `FERROBRIDGE__CDM__URL` | `FERROBRIDGE__CDM__URL_FILE` |

Prefer the `_file` route: a Docker or Kubernetes secret arrives as a file, and
a file never shows up in `docker inspect` or a process listing.

## What a bad configuration does

The server refuses to start and prints one line on stderr naming the key, then
exits 78, `EX_CONFIG`. Four things are refused:

- an unknown key or an unknown section;
- a value of the wrong type, or one that is not a socket address, a URL, or a
  release name;
- a secret set both inline and through its `_file` sibling;
- a section that is present and names no value for a key it needs.

## An example

```toml
[server]
listen = "0.0.0.0:8080"

[telemetry]
format = "json"

[cdr]
base_url = "https://cdr.example.org/ehrbase/rest/openehr/v1"

[cdr.credentials]
bearer_token_file = "/run/secrets/cdr-token"

[terminology]
base_url = "https://tx.example.org/r4"
wire_version = "r4"

[mappings]
directory = "/etc/ferrobridge/mappings"

[facade]
enabled = true
base_url = "https://bridge.example.org/fhir"
ehr_policy = "create_on_first_write"
identity_store = "/var/lib/ferrobridge/identity.redb"
subject_namespace = "https://example.org/fhir/sid/patient"
composition_language = "en"
composition_territory = "GB"
```
