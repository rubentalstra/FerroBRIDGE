# syntax=docker/dockerfile:1
# The FerroBRIDGE image: one static binary on distroless static, per platform.
# Nothing is compiled here. The release lane builds and attests the musl
# binaries and stages them under dist/<os>_<arch>/ferrobridge before this file
# is built for linux/amd64 and linux/arm64 (docs/release.md). .dockerignore
# holds the build context to that one directory.
#
# Build from a staged dist/ tree, at the repository root:
#   docker buildx build -f docker/Dockerfile --platform linux/arm64 .

# distroless static: ca-certificates, tzdata, an /etc/passwd carrying the
# nonroot user (uid 65532), a /tmp, no shell and no package manager. Pinned by
# the digest of its image index, which is what Docker resolves for a
# multi-platform build; Dependabot (the docker ecosystem over /docker) proposes
# the bumps. https://github.com/GoogleContainerTools/distroless
FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3

# The platform of the manifest being built, set by buildx per platform.
# https://docs.docker.com/reference/dockerfile/#automatic-platform-args-in-the-global-scope
ARG TARGETOS
ARG TARGETARCH

# Root-owned, executable, and not writable by the runtime user.
COPY --chmod=0555 dist/${TARGETOS}_${TARGETARCH}/ferrobridge /usr/local/bin/ferrobridge

# The binary's own default is loopback, which no container can publish, so the
# image default is every interface on the documented port. The configuration
# key is `[server] listen`; every override is FERROBRIDGE__<SECTION>__<KEY>.
ENV FERROBRIDGE__SERVER__LISTEN=0.0.0.0:8080
EXPOSE 8080/tcp

# Numeric on purpose: the kubelet cannot resolve a user name against an image
# it does not read, so `runAsNonRoot: true` refuses a named USER.
USER 65532:65532

# No HEALTHCHECK: the base has no shell and no HTTP client, and the binary has
# no probe subcommand, so the probe is external, a GET /health/liveness (and
# /health/readiness for the upstreams) from the orchestrator or the compose
# file. Kubernetes ignores the instruction and keeps its own probes anyway.

# Exec form: the binary is PID 1 and receives SIGTERM itself, which it drains
# on. CMD is the default job, so a Kubernetes Job sets `args:` alone and never
# has to override `command:`.
ENTRYPOINT ["/usr/local/bin/ferrobridge"]
CMD ["serve"]

# Build-independent labels only. The release lane sets the version, revision
# and created labels at build time and mirrors every label as an index
# annotation, which is what GHCR reads for the package page.
LABEL org.opencontainers.image.title="FerroBRIDGE" \
      org.opencontainers.image.description="A standalone bridge between openEHR and HL7 FHIR, driven by FHIRconnect, and the OMOP Common Data Model, driven by OMOCL" \
      org.opencontainers.image.url="https://ferrobridge.eu" \
      org.opencontainers.image.documentation="https://ferrobridge.eu/docs/operate/container.html" \
      org.opencontainers.image.source="https://github.com/rubentalstra/FerroBRIDGE" \
      org.opencontainers.image.vendor="FerroBRIDGE" \
      org.opencontainers.image.licenses="BUSL-1.1" \
      org.opencontainers.image.base.name="gcr.io/distroless/static-debian13:nonroot"
