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

### The console

Two more variables set the console for one run, and they win over the file
and over `FERROBRIDGE__TELEMETRY__FORMAT` and `FERROBRIDGE__TELEMETRY__FILTER`:

| Variable | Sets | Values |
|---|---|---|
| `FERROBRIDGE_LOG_FORMAT` | `[telemetry] format` | `auto`, `json` or `pretty`; any other value refuses the start with exit 78 |
| `RUST_LOG` | `[telemetry] filter` | A `tracing` filter directive such as `debug,hyper=warn` |

```sh
FERROBRIDGE_LOG_FORMAT=pretty RUST_LOG=debug ferrobridge serve
```

`auto` writes `pretty` with colour on a terminal and `json` without colour
anywhere else, so a container or a log pipeline gets one JSON object per line
with no configuration. An explicit `pretty` keeps its colour into a pipe.
Colour only wraps text, so a line reads the same once a collector strips the
escapes. In `pretty`, a carriage return or line feed inside a logged value is
written as the two characters `\r` or `\n`, so no value can start a second
line; `json` escapes it inside the string.

Under `pretty`, `ferrobridge serve` prints a banner to stdout before the first
log line: the wordmark, the version, the pins this build serves, the commit and
build instant, and each lane with the hosts it reaches. A host is the URL's
host and port and never its user information. Under `json` nothing but log
lines reaches stdout.

The first boot events describe the process:

| Event | Fields |
|---|---|
| `console` | `format`, `rendering`, `filter` (the one in effect, after a fallback), `colour` |
| `build` | `version`, `commit`, `built_at`, `rustc`, `openehr_crates`, `fhir_types` |
| `lane` | `lane` (`facade`, `operations`, `etl` or `terminology`), `enabled`, `identifiable`, and where set `cdr_host`, `terminology_host`, `cdm_host`, `cdm_schema`, `bridge_schema`, `contexts`, `programs`, `templates` |

`listening` is the last boot event. The `etl` lane reports what
`ferrobridge etl run` would reach; `serve` never runs it.

`GET /health/info` answers the same build facts and pins as JSON, so a scraper
reads what the banner shows:

```json
{
  "product": "FerroBRIDGE",
  "version": "0.0.3",
  "build": {
    "commit": "0123456789abcdef0123456789abcdef01234567",
    "commit_short": "0123456789ab",
    "built_at": "2026-01-01T00:00:00Z",
    "rustc": "rustc 1.98.1 (48a229cea 2026-09-01)"
  },
  "pins": [
    { "name": "FHIRconnect", "version": "v1.0.0" },
    { "name": "FHIR", "version": "R4 (4.0.1)" }
  ]
}
```

The pin list is shortened here; the route lists every pin the banner prints.
`built_at` is the `SOURCE_DATE_EPOCH` instant when the build sets one, and the
commit is `unknown` for a build outside a git checkout.

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

For the CDR, the bridge calls through the generated ITS-REST client of the
`openehr-its` crate, and the three keys are that client's retry budget. Only a
`GET`, `PUT` or `DELETE` is sent again, after a failure to connect, a
timeout, or a `5xx` other than `501`; a `POST` is sent once. A `401`, a `403`
and a status the operation does not document are never retried.
`[cdr] timeout_ms` is the per-request timeout of the client's HTTP engine.

### `[cdr.credentials]` and `[terminology.credentials]`

Set one scheme. A bearer token and a user together is a boot error, and so is
an inline value beside its `_file` sibling.

| Key | Default | Secret | Meaning |
|---|---|---|---|
| `bearer_token` | none | yes | An RFC 6750 bearer token |
| `user` | none | | The user name of RFC 7617 basic authentication |
| `password` | none | yes | The password of RFC 7617 basic authentication |

For the CDR, the scheme becomes the `Authorization` header of every call the
generated client sends: `Bearer <token>` for a token, and `Basic` over
`user:password` for a user and a password.

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
| `url` | none, required when the section is present | yes | The PostgreSQL connection URL; it must name its `sslmode` |
| `tls_ca` | none | no | The PEM CA the database's certificate is checked against |
| `tls_ca_file` | none | no | A file holding that CA, read at boot |
| `schema` | `cdm` | no | The schema the CDM tables live in, an unquoted lower-case identifier |
| `bridge_schema` | `ferrobridge` | no | The schema the bridge keeps its natural-key side table, id sequences and watermarks in |
| `person_policy` | `create_on_first_sight` | no | `create_on_first_sight` gives an EHR a `person_id` the first time a row refers to it; `existing` refuses a row whose EHR has no `PERSON` row yet |

Two clients open the database: the concept resolver's pool and the CDM
writer's own connection. Both read the `sslmode` of `url` and the CA from
`tls_ca` or `tls_ca_file`, so they agree on whether the connection is
encrypted and what it trusts. The modes are the libpq ones that never fall
back to plaintext
(<https://www.postgresql.org/docs/current/libpq-ssl.html>):

| `sslmode` | What the clients do |
|---|---|
| `disable` | Connect without TLS. A CA with `disable` is refused |
| `require` | Connect over TLS. With a CA, the certificate must chain to it, as libpq checks when a root certificate is present; without one, it is not checked |
| `verify-ca` | Connect over TLS; the certificate must chain to the CA, or to the webpki root set when no CA is set |
| `verify-full` | As `verify-ca`, and the certificate must name the host in `url` |

`prefer` and `allow` are refused when the configuration is read, naming the
mode: a client that honours them connects in plaintext when the server offers
no TLS, and that fallback is silent. A URL that names no `sslmode` is refused
the same way, because libpq's default is `prefer`: write `sslmode=disable`
for a database on a trusted local network or in a test container, and one of
the three TLS modes everywhere else. Any other `ssl` parameter in the URL
(`sslrootcert`, `sslcert`, `sslkey` and the rest) is refused too: the CA
comes from `tls_ca_file`, and the bridge sends no client certificate. For the
same reason the bridge refuses to start when `PGSSLROOTCERT`, `PGSSLCERT` or
`PGSSLKEY` is set in its environment: the pool's PostgreSQL client reads
those variables whatever the configuration says, and the writer does not. The CA is public material, so it is
not a secret, and `FERROBRIDGE__CDM__TLS_CA` and
`FERROBRIDGE__CDM__TLS_CA_FILE` set the two keys from the environment; setting
both is refused. A typical production URL, with the CA mounted beside it:

```toml
[cdm]
url_file = "/run/secrets/cdm_url"   # postgres://bridge:...@cdm.internal:5432/omop?sslmode=verify-full
tls_ca_file = "/run/secrets/cdm_ca.pem"
```

### What `cdm init` does

`ferrobridge cdm init` reads which CDM tables `[cdm] schema` already holds
before it applies anything:

| The schema holds | `cdm init` |
|---|---|
| none of the 39 CDM v5.4 tables, or does not exist | Creates the schema when absent, then applies OHDSI's tables, primary keys and indices in one transaction, and exits 0 |
| every CDM v5.4 table | Applies nothing, prints that the schema is already initialised with the `cdm_version` its `CDM_SOURCE` rows record, and exits 0 |
| some of the tables | Applies nothing and exits 1, naming every missing table |

It then creates the bridge schema, which is safe to repeat. Tables outside the
CDM's 39 are left alone and do not count. The check reads table names only,
so a schema whose tables exist without their keys or indices counts as
initialised.

OHDSI's fourth file, `OMOPCDM_postgresql_5.4_constraints.sql`, carries the
foreign keys and is left out by default. `cdm init --with-constraints`
applies it after the tables, statement by statement in one transaction of its
own. At the pinned OHDSI tag `v5.4.3` PostgreSQL refuses line 157, a foreign
key onto `vocabulary (vocabulary_id)` that the primary keys file gives no key
(SQLSTATE `42830`), so the flag exits 1, prints that statement, and leaves the
tables in place and no foreign key behind.

### `[etl]`

The OMOP ETL job that `ferrobridge etl run` runs. Both queries are parsed and
checked when the configuration loads, so a query the runner cannot read
refuses every job.

| Key | Default | Secret | Meaning |
|---|---|---|---|
| `aql` | none, required | no | The composition query. It aliases `ehr_id`, `version_uid` and the whole `composition`, carries an `ORDER BY` and no `LIMIT`. It may name `$since`, which `--since` binds. It may also alias `versioned_object_uid`, which the runner then checks against the version; when it does not, the runner reads it from the `version_uid`, whose `object_id` part names the versioned object (openEHR RM Common 1.1.0, `OBJECT_VERSION_ID`) |
| `page_size` | `100` | no | The `fetch` of each page of either query |
| `type_concept_id` | none, required | no | The `*_type_concept_id` the mappings write, the provenance of the records |
| `observation_period_type_concept_id` | none, required | no | The `period_type_concept_id` of every observation period |

### `[etl.visits]`

The visit derivation, off until the section is present. The rows are grouped
by EHR and source, each visit spanning the earliest start to the latest end.

| Key | Default | Secret | Meaning |
|---|---|---|---|
| `aql` | none, required | no | The visit query, aliasing `ehr_id`, `visit_source`, `visit_start` and `visit_end`, with an `ORDER BY` |
| `visit_concept_id` | none, required | no | The `visit_concept_id` of every visit |
| `visit_type_concept_id` | none, required | no | The `visit_type_concept_id` of every visit |

### `[mappings]`

The mapping sets this deployment runs. The facade, the two FHIRconnect
operations and the mapping subcommands read the FHIRconnect set in
`directory`; `etl run` reads the OMOCL set in `omocl`. The FHIRconnect set
and its templates are read once at boot, and a mapping that does not compile
refuses the start rather than the first request that touches it. The OMOCL
set is read once when `etl run` starts and compiled against each template
the first time a composition of it arrives, because an OMOCL file names an
archetype and no template.

| Key | Default | Meaning |
|---|---|---|
| `directory` | none | The directory the mapping files are read from, recursively (`.yml`, `.yaml`) |
| `templates` | none | The directory holding the operational templates the two operations compile against, as OPT 1.4 XML (`.opt`) |
| `omocl` | none | The directory the OMOCL files `ferrobridge etl run` maps with are read from, recursively (`.yml`, `.yaml`); read and validated once at start, and every file an `Include` names must be in it |

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
exits 78, `EX_CONFIG`. Five things are refused:

- an unknown key or an unknown section;
- a value of the wrong type, or one that is not a socket address, a URL, or a
  release name;
- a `[cdm] url` that names no `sslmode`, or one that falls back to plaintext
  or is not a libpq mode, a `[cdm]` CA that holds no PEM certificate, and a
  `PGSSLROOTCERT`, `PGSSLCERT` or `PGSSLKEY` in the environment;
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
