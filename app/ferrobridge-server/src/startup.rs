// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The startup summary: one structured event per lane, naming whether it is
//! on, the host each upstream it reaches answers on, and what it loaded.
//!
//! A host is the URL's host and port and never its user information, so no
//! credential reaches the log. No specification governs this: our own design.

use secrecy::ExposeSecret;

use crate::config::Settings;

/// What one mapping lane loaded at boot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MappingCounts {
    /// The context mappings the set holds.
    pub contexts: usize,
    /// The programs compiled from them.
    pub programs: usize,
    /// The operational templates the programs compile against.
    pub templates: usize,
}

/// One lane of this deployment, as the banner and the summary report it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lane {
    /// The lane's name: `facade`, `hl7v2`, `operations`, `etl` or
    /// `terminology`.
    pub name: &'static str,
    /// The socket address a face listens on, for a face with a listener of
    /// its own.
    pub listen: Option<String>,
    /// Whether the configuration switches it on.
    pub enabled: bool,
    /// Whether it carries identifiable data.
    pub identifiable: bool,
    /// The host of the CDR it reaches.
    pub cdr_host: Option<String>,
    /// The host of the terminology server it reaches.
    pub terminology_host: Option<String>,
    /// The host of the CDM database it writes to.
    pub cdm_host: Option<String>,
    /// The `sslmode` both CDM clients connect with, as libpq spells it.
    pub cdm_sslmode: Option<&'static str>,
    /// The schema the CDM tables live in.
    pub cdm_schema: Option<String>,
    /// The schema the natural-key side table lives in.
    pub bridge_schema: Option<String>,
    /// What its mapping set loaded, once it has.
    pub mappings: Option<MappingCounts>,
}

impl Lane {
    /// Returns the hosts this lane reaches, in `cdr`, `terminology`, `cdm`
    /// order.
    #[must_use]
    pub fn hosts(&self) -> Vec<&str> {
        [&self.cdr_host, &self.terminology_host, &self.cdm_host]
            .into_iter()
            .filter_map(Option::as_deref)
            .collect()
    }
}

/// Returns the five lanes `settings` describes, with no mapping counts yet.
///
/// The operations reach the CDR only for their templates, so a deployment that
/// names a template directory reports no CDR host for them. The ETL runs under
/// `ferrobridge etl run`; `serve` reports the lane it would run. The HL7 v2
/// face writes through the facade's CDR and translates its table values on
/// the terminology server.
#[must_use]
pub fn lanes(settings: &Settings) -> Vec<Lane> {
    let cdr_host = settings.cdr.as_ref().and_then(|cdr| host_of(&cdr.base_url));
    let terminology_host = settings
        .terminology
        .as_ref()
        .and_then(|terminology| host_of(&terminology.base_url));
    let facade = settings.facade.is_some();
    let hl7v2 = settings.hl7v2.as_ref();
    let operations = settings.operations.enabled && settings.mapping_directory.is_some();
    let etl = settings.etl.is_some();
    let cdm = settings.cdm.as_ref();
    let database_host = cdm
        .filter(|_| etl)
        .and_then(|cdm| url::Url::parse(cdm.url.expose_secret()).ok())
        .and_then(|url| host_of(&url));
    vec![
        Lane {
            name: "facade",
            enabled: facade,
            identifiable: facade,
            cdr_host: cdr_host.clone().filter(|_| facade),
            ..Lane::default()
        },
        Lane {
            name: "hl7v2",
            enabled: hl7v2.is_some(),
            identifiable: hl7v2.is_some(),
            listen: hl7v2.map(|face| face.listen.to_string()),
            cdr_host: cdr_host.clone().filter(|_| hl7v2.is_some()),
            terminology_host: terminology_host.clone().filter(|_| hl7v2.is_some()),
            ..Lane::default()
        },
        Lane {
            name: "operations",
            enabled: operations,
            identifiable: operations,
            cdr_host: cdr_host
                .clone()
                .filter(|_| operations && settings.mappings.is_none()),
            ..Lane::default()
        },
        Lane {
            name: "etl",
            enabled: etl,
            identifiable: etl,
            cdr_host: cdr_host.filter(|_| etl),
            cdm_host: database_host,
            cdm_sslmode: cdm
                .filter(|_| etl)
                .map(|cdm| cdm.connection.ssl_mode().as_str()),
            cdm_schema: cdm
                .filter(|_| etl)
                .map(|cdm| cdm.schema.as_str().to_owned()),
            bridge_schema: cdm
                .filter(|_| etl)
                .map(|cdm| cdm.bridge_schema.as_str().to_owned()),
            ..Lane::default()
        },
        Lane {
            name: "terminology",
            enabled: settings.terminology.is_some(),
            terminology_host,
            ..Lane::default()
        },
    ]
}

/// Returns `url`'s host, with its port when the URL names one.
///
/// The user information is never part of it.
#[must_use]
pub fn host_of(url: &url::Url) -> Option<String> {
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

/// Logs one event per lane.
pub fn log(lanes: &[Lane]) {
    for lane in lanes {
        tracing::info!(
            lane = lane.name,
            enabled = lane.enabled,
            identifiable = lane.identifiable,
            listen = lane.listen.as_deref(),
            cdr_host = lane.cdr_host.as_deref(),
            terminology_host = lane.terminology_host.as_deref(),
            cdm_host = lane.cdm_host.as_deref(),
            cdm_sslmode = lane.cdm_sslmode,
            cdm_schema = lane.cdm_schema.as_deref(),
            bridge_schema = lane.bridge_schema.as_deref(),
            contexts = lane.mappings.map(|counts| counts.contexts),
            programs = lane.mappings.map(|counts| counts.programs),
            templates = lane.mappings.map(|counts| counts.templates),
            "lane"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Lane, host_of};

    #[test]
    fn a_host_keeps_its_port_and_drops_the_user_information() {
        let url = url::Url::parse("postgres://bridge:s3cret@db.invalid:5433/cdm")
            .expect("a synthetic URL");
        assert_eq!(Some(String::from("db.invalid:5433")), host_of(&url));
        let url = url::Url::parse("https://cdr.invalid/openehr/v1").expect("a synthetic URL");
        assert_eq!(Some(String::from("cdr.invalid")), host_of(&url));
    }

    #[test]
    fn the_hosts_of_a_lane_come_in_upstream_order() {
        let lane = Lane {
            name: "etl",
            cdr_host: Some(String::from("cdr.invalid")),
            cdm_host: Some(String::from("db.invalid")),
            ..Lane::default()
        };
        assert_eq!(vec!["cdr.invalid", "db.invalid"], lane.hosts());
    }
}
