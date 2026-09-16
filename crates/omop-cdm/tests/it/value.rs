// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The three column types against the forms the CDM's datatypes accept.

use std::error::Error;

use omop_cdm::value::{CdmDate, CdmDatetime, ValueError, Varchar};

#[test]
fn a_varchar_accepts_its_bound_and_refuses_one_character_more() -> Result<(), Box<dyn Error>> {
    let boundary = "x".repeat(20);
    let value: Varchar<20> = Varchar::new(boundary.clone())?;
    assert_eq!(boundary, value.as_str());

    let over_long = "x".repeat(21);
    assert_eq!(
        Err(ValueError::TooLong {
            length: 21,
            limit: 20
        }),
        Varchar::<20>::new(over_long)
    );
    Ok(())
}

#[test]
fn a_varchar_counts_characters_not_bytes() -> Result<(), Box<dyn Error>> {
    let value: Varchar<2> = Varchar::new("é€")?;
    assert_eq!("é€", value.as_str());
    Ok(())
}

#[test]
fn a_date_is_the_iso_8601_extended_form() {
    assert!(CdmDate::new("2026-09-12").is_ok());
    assert!(CdmDate::new("2024-02-29").is_ok(), "2024 is a leap year");

    for refused in [
        "2026-9-12",
        "2026-09-12T00:00:00",
        "12-09-2026",
        "2026-13-01",
        "2026-02-30",
        "2023-02-29",
        "2026-09-12 ",
        "",
    ] {
        assert_eq!(
            Err(ValueError::NotADate),
            CdmDate::new(refused),
            "`{refused}` was accepted as a date"
        );
    }
}

#[test]
fn a_datetime_carries_a_time_and_no_offset() {
    assert!(CdmDatetime::new("2026-09-12T10:00:00").is_ok());
    assert!(CdmDatetime::new("2026-09-12T10:00:00.123").is_ok());

    for refused in [
        "2026-09-12",
        "2026-09-12T10:00",
        "2026-09-12 10:00:00",
        "2026-09-12T10:00:00Z",
        "2026-09-12T10:00:00+02:00",
        "2026-09-12T24:00:00",
        "2026-09-12T10:60:00",
        "2026-09-12T10:00:60",
        "2026-09-12T10:00:00.",
        "2026-09-12T10:00:00.1234567890",
    ] {
        assert_eq!(
            Err(ValueError::NotADatetime),
            CdmDatetime::new(refused),
            "`{refused}` was accepted as a date and time"
        );
    }
}

#[test]
fn a_row_carries_the_column_types_the_definitions_declare() -> Result<(), Box<dyn Error>> {
    let period = omop_cdm::generated::observation_period::ObservationPeriod {
        observation_period_id: 1,
        person_id: 2,
        observation_period_start_date: CdmDate::new("2026-01-01")?,
        observation_period_end_date: CdmDate::new("2026-12-31")?,
        period_type_concept_id: 32_817,
    };
    assert_eq!("2026-01-01", period.observation_period_start_date.as_str());
    Ok(())
}
