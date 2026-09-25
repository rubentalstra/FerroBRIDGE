// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The startup summary and the banner over a resolved configuration: which
//! lanes are on, the hosts they reach, and no credential anywhere.

use ferrobridge_server::config::Config;
use ferrobridge_server::{banner, build_info, startup};
use std::collections::BTreeMap;
use std::error::Error as StdError;

/// A deployment whose CDR URL carries user information, with the operations
/// taking their templates from the CDR and the terminology lane on.
const FILE: &str = r#"
[cdr]
base_url = "http://synthetic:s3cret@cdr.invalid:8080/v1"

[terminology]
base_url = "https://tx.invalid/r4"
wire_version = "r4"

[mappings]
directory = "/srv/ferrobridge/mappings"
"#;

#[test]
fn every_lane_is_reported_with_the_hosts_it_reaches() -> Result<(), Box<dyn StdError>> {
    let settings = Config::from_sources(Some(FILE), &BTreeMap::new())?.resolve()?;
    let lanes = startup::lanes(&settings);
    let names: Vec<&str> = lanes.iter().map(|lane| lane.name).collect();
    assert_eq!(vec!["facade", "operations", "etl", "terminology"], names);
    let lane = |name: &str| lanes.iter().find(|lane| lane.name == name);
    let facade = lane("facade").expect("the facade lane");
    assert!(!facade.enabled, "no [facade] section");
    assert_eq!(None, facade.cdr_host, "an off lane reaches nothing");
    let operations = lane("operations").expect("the operations lane");
    assert!(
        operations.enabled,
        "a mapping directory and the default switch"
    );
    assert!(
        operations.identifiable,
        "the operations carry clinical content"
    );
    assert_eq!(Some("cdr.invalid:8080"), operations.cdr_host.as_deref());
    let terminology = lane("terminology").expect("the terminology lane");
    assert!(terminology.enabled, "a [terminology] section");
    assert_eq!(Some("tx.invalid"), terminology.terminology_host.as_deref());
    assert!(
        !lane("etl").expect("the etl lane").enabled,
        "no [etl] section"
    );
    Ok(())
}

#[test]
fn the_banner_names_the_hosts_and_never_the_user_information() -> Result<(), Box<dyn StdError>> {
    let settings = Config::from_sources(Some(FILE), &BTreeMap::new())?.resolve()?;
    let text = banner::render(
        "0.0.0",
        &build_info::pins(),
        &startup::lanes(&settings),
        false,
    );
    assert!(text.contains("cdr.invalid:8080"), "{text}");
    assert!(text.contains("tx.invalid"), "{text}");
    assert!(!text.contains("s3cret"), "no credential: {text}");
    assert!(!text.contains("synthetic"), "no user name: {text}");
    Ok(())
}
