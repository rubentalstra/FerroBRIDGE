<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# The container image

FerroBRIDGE ships as one image carrying one binary. This page describes the
image, the quickstart `compose.yaml` beside it, the variables the server reads,
and the health probe you point your orchestrator at. No release has been cut
yet, so no tag exists on the registry; the recipe and the quickstart are in the
repository and the first release publishes the image they describe.

No specification governs packaging, the process model, or health probes: this
page is FerroBRIDGE's own design.

<!-- toc -->

## What the image is

| Fact | Value |
|---|---|
| Registry and repository | `ghcr.io/rubentalstra/ferrobridge` |
| Tag | the product version, for example `0.0.1`; each release publishes its own |
| Base | `gcr.io/distroless/static-debian13:nonroot`, pinned by index digest |
| Platforms | `linux/amd64` and `linux/arm64` |
| User | uid 65532, gid 65532, written numerically |
| Entrypoint | `/usr/local/bin/ferrobridge`, exec form, with `serve` as the default command |
| Port | 8080/tcp |

The base is distroless static: CA certificates, time zone data, an
`/etc/passwd` carrying the non-root user, and a `/tmp`. There is no shell, no
package manager, and no HTTP client, so the image cannot run a script and
nothing can be installed into it at runtime. The binary is a static musl build
that the release lane produces, attests, and stages before the image is built;
the image compiles nothing.

The user is written as `65532:65532` rather than as a name because the kubelet
cannot resolve a name against an image it does not read, so a named `USER`
makes `runAsNonRoot: true` refuse the pod.

Every build-independent OCI label is set in the recipe
(`org.opencontainers.image.title`, `.description`, `.url`, `.documentation`,
`.source`, `.vendor`, `.licenses`, `.base.name`). The release lane adds the
version, revision and creation labels at build time, so the same file produces
a correctly labelled image at every cut.

The tag scheme follows the product version, and a version is never rebuilt: a
bad cut ships forward as the next patch version.

## Running it with compose

`compose.yaml` at the repository root is the quickstart, and it ships as a
release asset. Download it, create the secret files beside it, and start:

```sh
mkdir -p secrets
printf '%s' "$CDM_PASSWORD" > secrets/cdm_password
printf 'postgresql://ferrobridge:%s@cdm:5432/omop' "$CDM_PASSWORD" > secrets/cdm_url
: > secrets/cdr_bearer_token

docker compose up
curl http://127.0.0.1:8080/health/liveness
```

Compose refuses to start a service whose secret source file is missing, so
create all three even when a lane is off; a file nothing reads may be empty.
Never commit them.

The published port binds `127.0.0.1` by default. A published port is DNAT'd
ahead of the host firewall's own chains, so a port published on `0.0.0.0` is
reachable from the network even when the firewall says otherwise
([Docker: packet filtering and
firewalls](https://docs.docker.com/engine/network/packet-filtering-firewalls/)).
To serve another machine, name that interface explicitly or put a reverse proxy
in front:

```sh
FERROBRIDGE_BIND_HOST=10.0.0.7 docker compose up
```

The service runs with a read-only root filesystem, every Linux capability
dropped, and `no-new-privileges:true`. There is no `tmpfs` beside it because
the server writes nothing: no cache, no scratch file, no upload directory, and
every secret is read once at boot.

## The environment variables

Every key is settable as `FERROBRIDGE__<SECTION>__<KEY>`, and
[Configuring the server](configuration.md) is the complete reference with every
default and every refusal. This section covers the variables the quickstart
names.

The image sets one of them itself: `FERROBRIDGE__SERVER__LISTEN=0.0.0.0:8080`,
because the binary's own default is loopback and no container can publish a
loopback port.

The quickstart passes the upstream variables through from your shell or your
`.env` file rather than defaulting them, because an empty base URL is a boot
error while an absent one is a lane the server never starts. Set the ones you
use:

```sh
FERROBRIDGE__CDR__BASE_URL=https://cdr.example.org/openehr/v1
FERROBRIDGE__CDR__CREDENTIALS__BEARER_TOKEN_FILE=/run/secrets/cdr_bearer_token
FERROBRIDGE__TERMINOLOGY__BASE_URL=https://tx.example.org/r4
FERROBRIDGE__TERMINOLOGY__WIRE_VERSION=r4
FERROBRIDGE__CDM__URL_FILE=/run/secrets/cdm_url
```

Every credential is reachable through a `_file` sibling read once at boot, and
that is what the compose secrets are for: a secret mounts at
`/run/secrets/<name>`, so the `_file` variable names that path and the value
never enters the environment, where `docker inspect` would show it. Setting a
value together with its `_file` sibling is a boot error, and a refused
configuration exits 78.

`FERROBRIDGE_CONFIG` names a TOML file instead, for a deployment that mounts
its configuration rather than setting variables.

## The health probe

The image declares no `HEALTHCHECK`. A distroless image has no shell and no
HTTP client, and the binary carries no probe subcommand, so the probe is
external. Two endpoints answer it:

- `GET /health/liveness` answers `200` while the process is up. Restart the
  container when it stops answering.
- `GET /health/readiness` runs one bounded check per configured upstream and
  answers `200` when every one of them answered. Take the instance out of the
  load balancer when it does not.

Readiness answers `503` when any indicator is down, with a JSON body naming the
aggregate and every indicator by name:

```json
{
  "state": "down",
  "indicators": {
    "cdr": { "state": "up" },
    "terminology": { "state": "down", "detail": "the upstream answered 503 Service Unavailable" }
  }
}
```

The `detail` names the upstream status or the reason the call never reached it,
and never a body, a credential, or clinical content. A probe that reaches the
upstream counts it up, `401` and `404` included; a `5xx` and a failure to
connect count it down. Each check is bounded, so a wedged upstream cannot hang
the probe. In Kubernetes, point `livenessProbe` at `/health/liveness` and
`readinessProbe` at `/health/readiness`.

## The OMOP CDM profiles

Two compose profiles cover the OMOP target, and neither runs on a plain
`docker compose up`.

The `cdm` profile stands up the CDM database on PostgreSQL 18.6, pinned by
digest, with its password read from `secrets/cdm_password` rather than from the
environment, and a `pg_isready` health check that gates the jobs below:

```sh
docker compose --profile cdm up -d cdm
docker compose --profile cdm run --rm cdm-init
```

`cdm-init` runs the bridge image with `cdm init`, which applies the OMOP CDM
v5.4 DDL once the database reports healthy.

The `vocab` profile loads an OHDSI Athena vocabulary export. The export is a
licensed download you obtain yourself; it is bind-mounted read only and no
vocabulary byte enters the image, the build context, or a release asset:

```sh
FERROBRIDGE_VOCAB_DIR=/srv/athena/2026-09 \
  docker compose --profile vocab run --rm vocab-load
```

Both subcommands exit 2 today with a line naming the issue that lands them, so
the two services are the shape the jobs run in rather than a working load. The
compose file says so beside each one.

## Sources

- distroless: <https://github.com/GoogleContainerTools/distroless>
- The Dockerfile reference: <https://docs.docker.com/reference/dockerfile/>
- The Compose file reference: <https://docs.docker.com/reference/compose-file/>
- Docker packet filtering and firewalls:
  <https://docs.docker.com/engine/network/packet-filtering-firewalls/>
