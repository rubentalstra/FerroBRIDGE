// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The boot banner: the wordmark, what this process is, the pins it serves
//! and the lanes its configuration switches on.
//!
//! It prints to stdout before the log subscriber exists, and only when the
//! console renders `pretty`, so a log pipeline reading `json` never sees it.
//! Colour wraps text and never replaces it, so every line reads the same once
//! a pipe or a log collector strips the escapes. No specification governs
//! this: our own design.

use crate::build_info::{self, Pin};
use crate::startup::Lane;

/// The wordmark in the `FIGlet` `standard` face, kept as text so the banner
/// needs no font file and no dependency.
const WORDMARK: &str = r"
 _____                   ____  ____  ___ ____   ____ _____
|  ___|__ _ __ _ __ ___ | __ )|  _ \|_ _|  _ \ / ___| ____|
| |_ / _ \ '__| '__/ _ \|  _ \| |_) || || | | | |  _|  _|
|  _|  __/ |  | | | (_) | |_) |  _ < | || |_| | |_| | |___
|_|  \___|_|  |_|  \___/|____/|_| \_\___|____/ \____|_____|
";

/// The product line printed under the wordmark, before the version.
pub const PRODUCT_LINE: &str = "openEHR bridge to HL7 FHIR and the OMOP CDM";

/// The maintainer named under the wordmark.
pub const MAINTAINER: &str = "Ruben Talstra";

/// The project site named under the wordmark.
pub const SITE: &str = "https://ferrobridge.eu";

/// The widest line the banner writes, in characters, colour escapes aside.
pub const MAX_WIDTH: usize = 100;

/// The escape that starts the wordmark's colour: bold cyan.
const WORDMARK_COLOUR: &str = "\x1b[1;36m";
/// The escape that marks a lane that is on: green.
const ON_COLOUR: &str = "\x1b[32m";
/// The escape that marks a lane that is off: dim.
const OFF_COLOUR: &str = "\x1b[2m";
/// The escape that ends a colour.
const RESET: &str = "\x1b[0m";

/// The width of the name column of the pin and lane lists.
const NAME_WIDTH: usize = 18;

/// Renders the banner for `version`, the served `pins` and the `lanes`, with
/// colour when `colour` is set.
#[must_use]
pub fn render(version: &str, pins: &[Pin], lanes: &[Lane], colour: bool) -> String {
    let paint = |escape: &str, text: &str| {
        if colour {
            format!("{escape}{text}{RESET}")
        } else {
            text.to_owned()
        }
    };
    let mut rows: Vec<String> = WORDMARK
        .trim_start_matches('\n')
        .lines()
        .map(|line| paint(WORDMARK_COLOUR, line))
        .collect();
    rows.push(format!("  {PRODUCT_LINE} · v{version}"));
    rows.push(format!("  Maintained by {MAINTAINER} · {SITE}"));
    rows.push(String::new());
    for pin in pins {
        rows.push(format!("  {:<NAME_WIDTH$}{}", pin.name, pin.version));
    }
    rows.push(format!(
        "  {:<NAME_WIDTH$}{} · {}",
        "build",
        build_info::COMMIT_SHORT,
        build_info::BUILT_AT
    ));
    rows.push(String::new());
    for lane in lanes {
        let state = if lane.enabled {
            paint(ON_COLOUR, "on ")
        } else {
            paint(OFF_COLOUR, "off")
        };
        let listens = lane
            .listen
            .as_ref()
            .map(|listen| format!("listens on {listen}"));
        let targets: Vec<&str> = listens.as_deref().into_iter().chain(lane.hosts()).collect();
        let line = format!(
            "  {:<NAME_WIDTH$}{state}  {}",
            lane.name,
            targets.join(", ")
        );
        rows.push(line.trim_end().to_owned());
    }
    let mut out = rows.join("\n");
    out.push('\n');
    out
}

/// Prints the banner for this build to stdout.
#[expect(
    clippy::print_stdout,
    reason = "the banner is console output and prints before any log subscriber exists"
)]
pub fn print(lanes: &[Lane], colour: bool) {
    print!(
        "{}",
        render(crate::state::VERSION, &build_info::pins(), lanes, colour)
    );
}

#[cfg(test)]
mod tests {
    use super::{MAINTAINER, MAX_WIDTH, SITE, render};
    use crate::build_info::pins;
    use crate::startup::Lane;

    fn lanes() -> Vec<Lane> {
        vec![
            Lane {
                name: "facade",
                enabled: true,
                cdr_host: Some(String::from("cdr.invalid:8443")),
                ..Lane::default()
            },
            Lane {
                name: "etl",
                enabled: true,
                cdr_host: Some(String::from("cdr.invalid:8443")),
                cdm_host: Some(String::from("db.invalid:5432")),
                ..Lane::default()
            },
            Lane {
                name: "terminology",
                ..Lane::default()
            },
        ]
    }

    /// Returns `text` with every ANSI escape sequence removed.
    fn plain(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn the_banner_names_the_version_the_maintainer_and_the_site() {
        let banner = render("1.2.3", &pins(), &lanes(), false);
        assert!(banner.contains("openEHR bridge to HL7 FHIR and the OMOP CDM · v1.2.3"));
        assert!(banner.contains(&format!("Maintained by {MAINTAINER} · {SITE}")));
    }

    #[test]
    fn every_line_fits_a_hundred_columns() {
        let banner = render("1.2.3", &pins(), &lanes(), true);
        for line in plain(&banner).lines() {
            assert!(line.chars().count() <= MAX_WIDTH, "{line:?}");
        }
    }

    #[test]
    fn the_coloured_banner_reads_the_same_without_its_colour() {
        let coloured = render("1.2.3", &pins(), &lanes(), true);
        let uncoloured = render("1.2.3", &pins(), &lanes(), false);
        assert_ne!(coloured, uncoloured, "colour is on");
        assert_eq!(uncoloured, plain(&coloured));
        assert!(!uncoloured.contains('\x1b'));
    }

    #[test]
    fn every_pin_and_every_lane_has_its_line() {
        let banner = render("1.2.3", &pins(), &lanes(), false);
        for pin in pins() {
            assert!(
                banner
                    .lines()
                    .any(|line| line.contains(pin.name) && line.ends_with(&pin.version)),
                "{pin:?}\n{banner}"
            );
        }
        assert!(
            banner.contains("facade            on   cdr.invalid:8443\n"),
            "{banner}"
        );
        assert!(
            banner.contains("etl               on   cdr.invalid:8443, db.invalid:5432\n"),
            "{banner}"
        );
        assert!(banner.contains("terminology       off\n"), "{banner}");
    }

    #[test]
    fn a_face_with_a_listener_names_its_address_before_its_hosts() {
        let lanes = [Lane {
            name: "hl7v2",
            enabled: true,
            listen: Some(String::from("0.0.0.0:2575")),
            cdr_host: Some(String::from("cdr.invalid:8443")),
            ..Lane::default()
        }];
        let banner = render("1.2.3", &pins(), &lanes, false);
        assert!(
            banner.contains("hl7v2             on   listens on 0.0.0.0:2575, cdr.invalid:8443\n"),
            "{banner}"
        );
    }
}
