// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The stub CDR the facade cases run over: the mounted routes and the
//! `wiremock` responders that echo, take and read back a commit.

use std::sync::Arc;
use std::time::Duration;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers;

use super::CONTAINER;
use super::CONTRIBUTION;
use super::EHR_ID;
use super::Harness;
use super::VERSION_TWO;
use super::ehr_body;

/// Mounts the EHR lookup that answers with [`EHR_ID`].
pub(super) async fn mount_ehr(cdr: &MockServer) {
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/ehr"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ehr_body()))
        .mount(cdr)
        .await;
}

/// Mounts the composition create that answers `201` with `body`.
pub(super) async fn mount_create(cdr: &MockServer, version: &str, body: &serde_json::Value) {
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("ETag", format!("W/\"{version}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(body.to_string()),
        )
        .mount(cdr)
        .await;
}

/// Mounts the composition read that answers `200` with `body`.
pub(super) async fn mount_read(cdr: &MockServer, version: &str, body: &serde_json::Value) {
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/composition/{CONTAINER}"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{version}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(body.to_string()),
        )
        .mount(cdr)
        .await;
}

/// Mounts the composition update that answers with `response`.
pub(super) async fn mount_update(cdr: &MockServer, response: ResponseTemplate) {
    Mock::given(matchers::method("PUT"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/composition/{CONTAINER}"
        )))
        .respond_with(response)
        .mount(cdr)
        .await;
}

/// Returns the `201` a contribution commit answers under
/// `Prefer: return=representation`: the CONTRIBUTION, whose `versions`
/// reference each committed version (`ehr-codegen.openapi.yaml`,
/// `201_CONTRIBUTION` and the `Contribution` schema).
pub(crate) fn contribution_created(contribution: &str, versions: &[&str]) -> ResponseTemplate {
    let body = ferrobridge_testkit::stubs::its_rest::contribution_body(contribution, versions);
    ResponseTemplate::new(201)
        .insert_header("ETag", format!("W/\"{contribution}\""))
        .insert_header("Content-Type", "application/json")
        .set_body_string(body)
}

/// How the echo CDR answers what a contribution committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Echo {
    /// The version list in the order the versions were sent.
    Sent,
    /// An empty `201`, as a CDR that does not honour
    /// `Prefer: return=representation` answers, with the CONTRIBUTION served
    /// on its read.
    Minimal,
    /// The version list reversed, which ITS-REST does not forbid.
    Reversed,
    /// The versions in order, each read back naming an item no entry sent.
    Foreign,
}

/// The compositions a stub CDR committed, by version and by container.
#[derive(Debug, Default)]
pub(super) struct Committed {
    /// Every committed composition, in commit order.
    versions: Vec<(String, serde_json::Value)>,
    /// How many version containers the commits created.
    created: usize,
    /// The versions the latest contribution committed, in commit order.
    latest: Vec<String>,
}

impl Committed {
    /// Returns the distinct version containers committed, in commit order.
    pub(super) fn containers(&self) -> Vec<&str> {
        let mut seen = Vec::new();
        for (version, _) in &self.versions {
            let container = version.split("::").next().unwrap_or_default();
            if !seen.contains(&container) {
                seen.push(container);
            }
        }
        seen
    }
}

/// The stub CDR half that takes a contribution and remembers its versions.
struct TakeContribution {
    /// What was committed.
    committed: Arc<std::sync::Mutex<Committed>>,
    /// The version containers handed out, in order.
    containers: &'static [&'static str],
    /// How the answer lists the versions.
    echo: Echo,
    /// How long the answer waits after the commit is taken.
    delay: Duration,
}

impl wiremock::Respond for TakeContribution {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        self.answer(request).set_delay(self.delay)
    }
}

impl TakeContribution {
    /// Takes the contribution `request` carries and answers it.
    fn answer(&self, request: &wiremock::Request) -> ResponseTemplate {
        let Ok(body) = serde_json::from_slice::<serde_json::Value>(&request.body) else {
            return ResponseTemplate::new(400);
        };
        let Ok(mut committed) = self.committed.lock() else {
            return ResponseTemplate::new(500);
        };
        let mut named = Vec::new();
        for data in body["versions"].as_array().into_iter().flatten() {
            // A version that names its `preceding_version_uid` is the next
            // version of that container (`ehr-codegen.openapi.yaml`,
            // `UpdateVersion`); any other opens the next container.
            let version = if let Some(prior) = data["preceding_version_uid"]["value"].as_str() {
                let mut parts = prior.split("::");
                let container = parts.next().unwrap_or_default();
                let tree = parts.nth(1).and_then(|tree| tree.parse::<u32>().ok());
                let Some(tree) = tree else {
                    return ResponseTemplate::new(400);
                };
                format!("{container}::ferrobridge.test::{}", tree + 1)
            } else {
                let Some(container) = self.containers.get(committed.created) else {
                    return ResponseTemplate::new(500);
                };
                committed.created += 1;
                format!("{container}::ferrobridge.test::1")
            };
            let mut stored = data["data"].clone();
            if self.echo == Echo::Foreign {
                stored["feeder_audit"]["originating_system_item_ids"][0]["id"] =
                    serde_json::json!("an-item-no-entry-sent");
            }
            committed.versions.push((version.clone(), stored));
            named.push(version);
        }
        committed.latest.clone_from(&named);
        if self.echo == Echo::Reversed {
            named.reverse();
        }
        if self.echo == Echo::Minimal {
            return ferrobridge_testkit::stubs::its_rest::contribution_created_minimal(
                CONTRIBUTION,
            );
        }
        let named: Vec<&str> = named.iter().map(String::as_str).collect();
        contribution_created(CONTRIBUTION, &named)
    }
}

/// The stub CDR half that answers the latest CONTRIBUTION it committed on its
/// read (`ehr-codegen.openapi.yaml`, `contribution_get`).
struct ReadContribution {
    /// What was committed.
    committed: Arc<std::sync::Mutex<Committed>>,
}

impl wiremock::Respond for ReadContribution {
    fn respond(&self, _request: &wiremock::Request) -> ResponseTemplate {
        let Ok(committed) = self.committed.lock() else {
            return ResponseTemplate::new(500);
        };
        let named: Vec<&str> = committed.latest.iter().map(String::as_str).collect();
        if named.is_empty() {
            return ferrobridge_testkit::stubs::its_rest::not_found(
                "no contribution was committed",
            );
        }
        ferrobridge_testkit::stubs::its_rest::retrieved(
            &ferrobridge_testkit::stubs::its_rest::contribution_body(CONTRIBUTION, &named),
        )
    }
}

/// The stub CDR half that answers a committed composition by its version, or
/// by its container at the latest version.
struct ReadCommitted {
    /// What was committed.
    committed: Arc<std::sync::Mutex<Committed>>,
}

impl wiremock::Respond for ReadCommitted {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let Ok(committed) = self.committed.lock() else {
            return ResponseTemplate::new(500);
        };
        let wanted = request
            .url
            .path_segments()
            .and_then(Iterator::last)
            .map(str::to_owned)
            .unwrap_or_default();
        let found = committed.versions.iter().rev().find(|(version, _)| {
            *version == wanted || version.split("::").next() == Some(wanted.as_str())
        });
        match found {
            Some((version, composition)) => ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{version}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(composition.to_string()),
            None => ResponseTemplate::new(404),
        }
    }
}

/// Mounts a stub CDR that takes contributions, handing out `containers` in
/// order, and reads back what it committed.
pub(crate) async fn mount_echo(cdr: &MockServer, containers: &'static [&'static str], echo: Echo) {
    mount_echo_after(cdr, containers, echo, Duration::ZERO).await;
}

/// Mounts the echo CDR of [`mount_echo`], whose contribution answer waits
/// `delay` after the commit is taken.
pub(super) async fn mount_echo_after(
    cdr: &MockServer,
    containers: &'static [&'static str],
    echo: Echo,
    delay: Duration,
) {
    mount_echo_tracked(cdr, containers, echo, delay).await;
}

/// Mounts the echo CDR of [`mount_echo_after`], and returns what it commits
/// so a case can count the containers.
pub(super) async fn mount_echo_tracked(
    cdr: &MockServer,
    containers: &'static [&'static str],
    echo: Echo,
    delay: Duration,
) -> Arc<std::sync::Mutex<Committed>> {
    let committed = Arc::new(std::sync::Mutex::new(Committed::default()));
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/contribution")))
        .respond_with(TakeContribution {
            committed: Arc::clone(&committed),
            containers,
            echo,
            delay,
        })
        .mount(cdr)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/contribution/{CONTRIBUTION}"
        )))
        .respond_with(ReadContribution {
            committed: Arc::clone(&committed),
        })
        .mount(cdr)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path_regex(format!(
            "^/ehr/{EHR_ID}/composition/.+$"
        )))
        .respond_with(ReadCommitted {
            committed: Arc::clone(&committed),
        })
        .mount(cdr)
        .await;
    committed
}

/// The containers the echo CDR hands out for a transaction.
pub(super) const ECHOED: &[&str] = &[
    CONTAINER,
    "5f0e2b1a-0000-4000-8000-00000000000d",
    "6a1f3c2b-0000-4000-8000-00000000000f",
];

/// Mounts the composition update that answers `200` with [`VERSION_TWO`].
pub(super) async fn mount_version_two(harness: &Harness) {
    mount_update(
        &harness.cdr,
        ResponseTemplate::new(200)
            .insert_header("ETag", format!("W/\"{VERSION_TWO}\""))
            .insert_header("Content-Type", "application/json")
            .set_body_string(harness.composition().to_string()),
    )
    .await;
}

/// Mounts the EHR lookup that finds no EHR for any subject.
pub(super) async fn mount_no_ehr(cdr: &MockServer) {
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/ehr"))
        .respond_with(ferrobridge_testkit::stubs::its_rest::not_found(
            "no EHR for the subject",
        ))
        .mount(cdr)
        .await;
}
