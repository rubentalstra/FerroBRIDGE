// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The container harness behind the `FERROBRIDGE_E2E` gate.
//!
//! A container-backed test asks [`e2e_enabled`] first and returns without
//! touching Docker when the gate is unset, so the ordinary suite stays offline.
//! Every image is pinned by tag and digest in [`POSTGRES`], [`CDR`],
//! [`CDR_POSTGRES`] and [`TERMINOLOGY`], which `docs/VERSIONS.md` repeats and
//! `scripts/checks/versions.sh` compares.
//!
//! No specification governs the harness; it is FerroBRIDGE's own design.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use testcontainers::core::{AccessMode, Healthcheck, IntoContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// The environment variable that admits the container-backed tests.
pub const E2E_GATE: &str = "FERROBRIDGE_E2E";

/// The value of [`E2E_GATE`] that admits them.
pub const E2E_GATE_VALUE: &str = "1";

/// The PostgreSQL port inside a container.
const POSTGRES_PORT: u16 = 5432;

/// The HTTP port the reference CDR listens on inside its container.
const CDR_PORT: u16 = 8080;

/// The HTTP port the reference terminology server listens on inside its
/// container.
const TERMINOLOGY_PORT: u16 = 8080;

/// Where the synthetic terminology fixtures are mounted in the container.
const TERMINOLOGY_RESOURCES: &str = "/data/codesystems";

/// The fixture directory the mount reads, absolute at compile time.
const TERMINOLOGY_FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/terminology");

/// The login role, its password and its database in the PostgreSQL container.
const POSTGRES_ROLE: &str = "ferrobridge";

/// The database the reference CDR image creates for its own role.
const CDR_DATABASE_ROLE: &str = "ferroehr";

/// How long the readiness poll of the CDR waits before it gives up.
const READINESS_BUDGET: Duration = Duration::from_secs(120);

/// How long the readiness poll sleeps between two probes.
const READINESS_INTERVAL: Duration = Duration::from_millis(250);

/// How often a container's own health check runs, and its per-run budget.
const HEALTH_INTERVAL: Duration = Duration::from_secs(1);

/// How many consecutive health-check failures make a container unhealthy.
const HEALTH_RETRIES: u32 = 30;

/// Distinguishes the networks of two harnesses in one process.
static NETWORK_SEQUENCE: AtomicU32 = AtomicU32::new(0);

/// Returns whether the container-backed tests may run.
///
/// They run when `FERROBRIDGE_E2E` is exactly `1`. A test that returns early
/// on a `false` here is reported by `cargo nextest` as passed rather than
/// skipped, which is the deliberate trade: the gate keeps the ordinary suite
/// offline, and the end-to-end lane of `.github/workflows/ci.yml` is the run
/// where these tests have to do their work.
#[must_use]
pub fn e2e_enabled() -> bool {
    std::env::var(E2E_GATE).is_ok_and(|value| value == E2E_GATE_VALUE)
}

/// One container image, pinned by tag and by digest.
///
/// The digest is what Docker resolves; the tag travels beside it so a reader
/// sees which release the digest is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinnedImage {
    /// The repository, registry included for anything but Docker Hub.
    pub repository: &'static str,
    /// The tag the digest was published under.
    pub tag: &'static str,
    /// The `sha256:` digest of the image index.
    pub digest: &'static str,
}

impl PinnedImage {
    /// Returns the reference Docker resolves this image by.
    ///
    /// # Examples
    ///
    /// ```
    /// let reference = ferrobridge_testkit::containers::POSTGRES.reference();
    /// assert!(reference.contains("@sha256:"));
    /// ```
    #[must_use]
    pub fn reference(&self) -> String {
        format!("{}:{}@{}", self.repository, self.tag, self.digest)
    }

    /// Returns the image with the digest in the tag position, which is how
    /// `testcontainers` spells a reference (`name:tag`).
    fn image(&self) -> GenericImage {
        GenericImage::new(
            self.repository.to_owned(),
            format!("{}@{}", self.tag, self.digest),
        )
    }
}

/// The PostgreSQL the CDM catalogue test applies the OHDSI DDL to.
pub const POSTGRES: PinnedImage = PinnedImage {
    repository: "postgres",
    tag: "18.6",
    digest: "sha256:4ef4dbc939d61acea57712655ddb4b4ab27419c913f94cca0cd57cb3ea3c2280",
};

/// The reference openEHR CDR, which speaks ITS-REST 1.1.0.
pub const CDR: PinnedImage = PinnedImage {
    repository: "ghcr.io/rubentalstra/ferroehr",
    tag: "4.2.5",
    digest: "sha256:aa5a9e0447befadb396084fd19ce7a6d30ea1fb8e16806615071e9f1748125ca",
};

/// The PostgreSQL the reference CDR image documents as its database.
///
/// The image carries the role, the database, the schemas and the extensions
/// the CDR's own migrations expect to find, so the CDR is started against it
/// rather than against a bare PostgreSQL.
pub const CDR_POSTGRES: PinnedImage = PinnedImage {
    repository: "ghcr.io/rubentalstra/ferroehr-postgres",
    tag: "4.2.5",
    digest: "sha256:e094461744fa8510ca8c1c4ecde4460474befb00b310ba41f7d9ff6e67e181bc",
};

/// The reference FHIR terminology server.
///
/// It serves each FHIR release under its own path prefix (`/r4`, `/r4b`,
/// `/r5`, `/r6`), which is why [`Terminology::base_url`] takes the release.
pub const TERMINOLOGY: PinnedImage = PinnedImage {
    repository: "ghcr.io/rubentalstra/ferroterm",
    tag: "0.1.3",
    digest: "sha256:b1ef80382e03c2474bfec2ec57a698d83314e1290dd0cb5a2612ea208bde020c",
};

/// A container could not be started, or did not become usable.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HarnessError {
    /// Docker refused to start or to inspect a container.
    #[error("the {image} container could not be started")]
    Container {
        /// The image that was being started.
        image: &'static str,
        /// What Docker reported.
        #[source]
        source: testcontainers::TestcontainersError,
    },
    /// The readiness probe could not be sent.
    #[error("the readiness probe of {url} could not be sent")]
    Probe {
        /// The endpoint that was probed.
        url: String,
        /// What the HTTP stack reported.
        #[source]
        source: reqwest::Error,
    },
    /// The service did not answer its readiness probe inside the budget.
    #[error("{url} did not become ready within {}s", budget.as_secs())]
    NotReady {
        /// The endpoint that was probed.
        url: String,
        /// How long the probe waited.
        budget: Duration,
    },
}

/// A started PostgreSQL, torn down when it is dropped.
#[derive(Debug)]
pub struct Postgres {
    /// The container, owned so that its lifetime is this value's.
    container: ContainerAsync<GenericImage>,
    /// The connection URL of the login role.
    url: String,
}

impl Postgres {
    /// Returns the connection URL of the login role.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the container, for a caller that needs to inspect it.
    #[must_use]
    pub fn container(&self) -> &ContainerAsync<GenericImage> {
        &self.container
    }
}

/// A started reference CDR with its own PostgreSQL, torn down when it is
/// dropped.
///
/// The two containers are fields, so dropping this value stops both.
#[derive(Debug)]
pub struct Cdr {
    /// The server itself.
    server: ContainerAsync<GenericImage>,
    /// The database it was started against.
    database: ContainerAsync<GenericImage>,
    /// The openEHR REST API root, the path `/ehr` and `/query` live under.
    base_url: String,
}

impl Cdr {
    /// Returns the openEHR REST API root of the started CDR.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns the CDR container.
    #[must_use]
    pub fn container(&self) -> &ContainerAsync<GenericImage> {
        &self.server
    }

    /// Returns the database container the CDR was started against.
    #[must_use]
    pub fn database(&self) -> &ContainerAsync<GenericImage> {
        &self.database
    }
}

/// A started reference terminology server, torn down when it is dropped.
#[derive(Debug)]
pub struct Terminology {
    /// The server itself, owned so that its lifetime is this value's.
    container: ContainerAsync<GenericImage>,
    /// The origin the server is reachable at, with no path.
    origin: String,
}

impl Terminology {
    /// Returns the FHIR service base URL of `release`.
    ///
    /// `release` is the path segment the server serves that FHIR release
    /// under, for example `r4` or `r4b`.
    #[must_use]
    pub fn base_url(&self, release: &str) -> String {
        format!("{}/{release}", self.origin)
    }

    /// Returns the origin the server is reachable at, with no path.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Returns the server container.
    #[must_use]
    pub fn container(&self) -> &ContainerAsync<GenericImage> {
        &self.container
    }
}

/// Starts the reference terminology server over the synthetic terminology
/// fixtures and waits for its readiness endpoint.
///
/// The image reads `FERROTERM_CODESYSTEMS` as a directory of FHIR resources,
/// so `fixtures/terminology` is bind-mounted read-only and the synthetic
/// `CodeSystem`, `ValueSet` and `ConceptMap` are served through the ordinary
/// operations. The server enforces no authentication of its own, so the
/// returned base URL needs no credentials.
///
/// # Errors
///
/// Returns [`HarnessError::Container`] when Docker refuses the container,
/// [`HarnessError::Probe`] when the readiness probe cannot be sent, and
/// [`HarnessError::NotReady`] when the server does not answer it in time.
pub async fn terminology() -> Result<Terminology, HarnessError> {
    let fixtures = Mount::bind_mount(TERMINOLOGY_FIXTURES, TERMINOLOGY_RESOURCES)
        .with_access_mode(AccessMode::ReadOnly);
    let container = TERMINOLOGY
        .image()
        .with_wait_for(WaitFor::message_on_stdout("listening"))
        .with_exposed_port(TERMINOLOGY_PORT.tcp())
        .with_mount(fixtures)
        .with_env_var("FERROTERM_CODESYSTEMS", TERMINOLOGY_RESOURCES)
        .with_env_var("FERROTERM_UI", "off")
        .start()
        .await
        .map_err(|source| HarnessError::Container {
            image: TERMINOLOGY.repository,
            source,
        })?;
    let host = container
        .get_host()
        .await
        .map_err(|source| HarnessError::Container {
            image: TERMINOLOGY.repository,
            source,
        })?;
    let port = container
        .get_host_port_ipv4(TERMINOLOGY_PORT.tcp())
        .await
        .map_err(|source| HarnessError::Container {
            image: TERMINOLOGY.repository,
            source,
        })?;
    let origin = format!("http://{host}:{port}");
    await_readiness(&format!("{origin}/health")).await?;
    Ok(Terminology { container, origin })
}

/// Starts a PostgreSQL and returns it with the URL of its login role.
///
/// The role, its password and its database are all `ferrobridge`.
///
/// # Errors
///
/// Returns [`HarnessError::Container`] when Docker refuses the container or
/// the mapped port cannot be read.
pub async fn postgres() -> Result<Postgres, HarnessError> {
    let container = POSTGRES
        .image()
        .with_wait_for(WaitFor::healthcheck())
        .with_exposed_port(POSTGRES_PORT.tcp())
        .with_health_check(postgres_health_check(POSTGRES_ROLE, POSTGRES_ROLE))
        .with_env_var("POSTGRES_USER", POSTGRES_ROLE)
        .with_env_var("POSTGRES_PASSWORD", POSTGRES_ROLE)
        .with_env_var("POSTGRES_DB", POSTGRES_ROLE)
        .start()
        .await
        .map_err(|source| HarnessError::Container {
            image: POSTGRES.repository,
            source,
        })?;
    let host = container
        .get_host()
        .await
        .map_err(|source| HarnessError::Container {
            image: POSTGRES.repository,
            source,
        })?;
    let port = container
        .get_host_port_ipv4(POSTGRES_PORT.tcp())
        .await
        .map_err(|source| HarnessError::Container {
            image: POSTGRES.repository,
            source,
        })?;
    let url = format!("postgres://{POSTGRES_ROLE}:{POSTGRES_ROLE}@{host}:{port}/{POSTGRES_ROLE}");
    Ok(Postgres { container, url })
}

/// Starts the reference CDR with its own PostgreSQL and waits for its
/// readiness endpoint.
///
/// Authentication is switched off (`FERROEHR__AUTH__ENABLED=false`), which the
/// image documents as its development posture, so the returned base URL needs
/// no credentials. The CDR reaches its database by container name on a network
/// this function creates.
///
/// # Errors
///
/// Returns [`HarnessError::Container`] when Docker refuses a container,
/// [`HarnessError::Probe`] when the readiness probe cannot be sent, and
/// [`HarnessError::NotReady`] when the CDR does not answer it in time.
pub async fn cdr() -> Result<Cdr, HarnessError> {
    let sequence = NETWORK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let process = std::process::id();
    let network = format!("ferrobridge-e2e-{process}-{sequence}");
    let database_name = format!("ferrobridge-e2e-db-{process}-{sequence}");

    let database = CDR_POSTGRES
        .image()
        .with_wait_for(WaitFor::healthcheck())
        .with_health_check(postgres_health_check(CDR_DATABASE_ROLE, CDR_DATABASE_ROLE))
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        .with_network(network.clone())
        .with_container_name(database_name.clone())
        .start()
        .await
        .map_err(|source| HarnessError::Container {
            image: CDR_POSTGRES.repository,
            source,
        })?;

    let dsn = format!(
        "postgres://{CDR_DATABASE_ROLE}:{CDR_DATABASE_ROLE}@{database_name}:{POSTGRES_PORT}/{CDR_DATABASE_ROLE}"
    );
    let cdr = CDR
        .image()
        .with_wait_for(WaitFor::message_on_stdout("ferroehr-rest listening"))
        .with_exposed_port(CDR_PORT.tcp())
        .with_network(network)
        .with_env_var("FERROEHR__DB__URL", dsn)
        .with_env_var("FERROEHR__AUTH__ENABLED", "false")
        .start()
        .await
        .map_err(|source| HarnessError::Container {
            image: CDR.repository,
            source,
        })?;

    let host = cdr
        .get_host()
        .await
        .map_err(|source| HarnessError::Container {
            image: CDR.repository,
            source,
        })?;
    let port = cdr
        .get_host_port_ipv4(CDR_PORT.tcp())
        .await
        .map_err(|source| HarnessError::Container {
            image: CDR.repository,
            source,
        })?;
    let origin = format!("http://{host}:{port}");
    await_readiness(&format!("{origin}/health/readiness")).await?;

    Ok(Cdr {
        server: cdr,
        database,
        base_url: format!("{origin}/ferroehr/rest/openehr/v1"),
    })
}

/// Returns the health check that answers when PostgreSQL accepts TCP
/// connections.
///
/// `pg_isready` is PostgreSQL's own readiness utility
/// (<https://www.postgresql.org/docs/18/app-pg-isready.html>), and the host is
/// named explicitly because the entrypoint's initialisation server listens on
/// the Unix socket alone; a socket probe would report ready before the port
/// the harness hands out serves anything.
fn postgres_health_check(role: &str, database: &str) -> Healthcheck {
    Healthcheck::cmd(["pg_isready", "-h", "127.0.0.1", "-U", role, "-d", database])
        .with_interval(HEALTH_INTERVAL)
        .with_timeout(HEALTH_INTERVAL)
        .with_retries(HEALTH_RETRIES)
}

/// Polls `url` until it answers `200`, or the budget runs out.
async fn await_readiness(url: &str) -> Result<(), HarnessError> {
    let client = reqwest::Client::builder()
        .timeout(READINESS_INTERVAL.saturating_mul(8))
        .build()
        .map_err(|source| HarnessError::Probe {
            url: url.to_owned(),
            source,
        })?;
    let deadline = std::time::Instant::now().checked_add(READINESS_BUDGET);
    loop {
        if let Ok(response) = client.get(url).send().await
            && response.status() == reqwest::StatusCode::OK
        {
            return Ok(());
        }
        let expired = deadline.is_none_or(|deadline| std::time::Instant::now() >= deadline);
        if expired {
            return Err(HarnessError::NotReady {
                url: url.to_owned(),
                budget: READINESS_BUDGET,
            });
        }
        tokio::time::sleep(READINESS_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{CDR, CDR_POSTGRES, POSTGRES, PinnedImage, TERMINOLOGY};

    #[test]
    fn every_pin_names_a_tag_and_a_digest() {
        for image in [POSTGRES, CDR, CDR_POSTGRES, TERMINOLOGY] {
            assert!(!image.tag.is_empty(), "{} has no tag", image.repository);
            assert!(
                image.digest.starts_with("sha256:"),
                "{} is not pinned by a sha256 digest",
                image.repository
            );
        }
    }

    #[test]
    fn a_reference_is_the_repository_tag_and_digest() {
        let image = PinnedImage {
            repository: "postgres",
            tag: "16.15",
            digest: "sha256:0000",
        };
        assert_eq!("postgres:16.15@sha256:0000", image.reference());
    }
}
