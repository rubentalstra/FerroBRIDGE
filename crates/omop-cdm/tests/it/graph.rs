// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The record graph against the CDM v5.4 column metadata: a row names only
//! real columns, holds only values their datatype admits, carries every
//! required column, and a graph holds only its own composition's rows.

use omop_cdm::graph::{
    ArchetypeRootPath, Discriminator, GraphError, Link, LinkEnd, MappingName, OccurrencePath,
    RecordGraph, RecordKey, Reference, Refusal, Row, RowBuilder, Source, Value, Visit, VisitKey,
    VisitSource,
};
use omop_cdm::value::CdmDate;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use std::error::Error;

/// The synthetic EHR every case uses.
const EHR: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";

/// The synthetic versioned composition every case uses.
const COMPOSITION: &str = "8849182c-82ad-4088-a07f-48ead4180515";

/// Returns the first version of the synthetic composition.
fn source() -> Result<Source, Box<dyn Error>> {
    Ok(Source::new(
        HierObjectId::new(EHR)?,
        ObjectVersionId::new(format!("{COMPOSITION}::ferrobridge.test::1"))?,
    ))
}

/// Returns the key of the node at `occurrence` in the synthetic composition.
fn key(occurrence: &str) -> Result<RecordKey, Box<dyn Error>> {
    Ok(RecordKey::new(
        &source()?,
        ArchetypeRootPath::new("/content[openEHR-EHR-OBSERVATION.laboratory_test_result.v1]")?,
        OccurrencePath::new(occurrence)?,
        Discriminator::new(MappingName::new("Laboratory_test_analyte_v1")?, 0, 0),
    ))
}

/// Returns a measurement builder with every required column but the date.
fn measurement(occurrence: &str) -> Result<RowBuilder, Box<dyn Error>> {
    Ok(Row::builder("measurement", key(occurrence)?)?
        .reference("person_id", Reference::Person(HierObjectId::new(EHR)?))?
        .value("measurement_concept_id", Value::Integer(1001))?
        .value("measurement_type_concept_id", Value::Integer(32817))?)
}

/// Returns the graph of the synthetic composition.
fn graph() -> Result<RecordGraph, Box<dyn Error>> {
    Ok(RecordGraph::new(source()?))
}

/// Returns a date.
fn date(text: &str) -> Result<Value, Box<dyn Error>> {
    Ok(Value::Date(CdmDate::new(text)?))
}

#[test]
fn a_row_with_every_required_column_builds() -> Result<(), Box<dyn Error>> {
    let row = measurement("/items[at0001]")?
        .value("measurement_date", date("2026-06-15")?)?
        .value("value_as_number", Value::Float(7.25))?
        .value("measurement_source_value", Value::Text(String::from("GLU")))?
        .build()?;
    assert_eq!("measurement", row.table().name);
    assert_eq!(6, row.cells().len(), "the cells set are the cells kept");
    Ok(())
}

#[test]
fn a_missing_required_date_refuses_the_row_and_names_the_column() -> Result<(), Box<dyn Error>> {
    let error = measurement("/items[at0001]")?
        .build()
        .expect_err("a measurement needs its date");
    assert_eq!(
        GraphError::Missing {
            table: "measurement",
            column: "measurement_date",
        },
        error
    );
    let refusal = Refusal::of_row(&error).with_element("/items[at0001]/time");
    assert_eq!(Some("measurement_date"), refusal.column());
    assert_eq!(Some("/items[at0001]/time"), refusal.element());
    Ok(())
}

#[test]
fn a_table_outside_the_cdm_schema_is_refused() -> Result<(), Box<dyn Error>> {
    for table in ["concept", "cohort", "no_such_table"] {
        let error = Row::builder(table, key("/")?).expect_err("only CDM-schema tables take rows");
        assert!(
            matches!(error, GraphError::UnknownTable { .. }),
            "{table}: {error:?}"
        );
    }
    Ok(())
}

#[test]
fn fact_relationship_and_the_derived_tables_take_no_rows() -> Result<(), Box<dyn Error>> {
    for table in [
        "fact_relationship",
        "observation_period",
        "condition_era",
        "drug_era",
        "dose_era",
    ] {
        let error = Row::builder(table, key("/")?).expect_err("the writer owns these tables");
        assert!(
            matches!(error, GraphError::NotWritable { .. }),
            "{table}: {error:?}"
        );
    }
    Ok(())
}

#[test]
fn the_primary_key_is_the_writers() -> Result<(), Box<dyn Error>> {
    let error = measurement("/")?
        .value("measurement_id", Value::Integer(1))
        .expect_err("the writer assigns the key");
    assert!(matches!(error, GraphError::PrimaryKey { .. }), "{error:?}");
    Ok(())
}

#[test]
fn an_unknown_column_and_a_column_set_twice_are_refused() -> Result<(), Box<dyn Error>> {
    let unknown = measurement("/")?
        .value("qualifier_concept_id", Value::Integer(1))
        .expect_err("measurement has no qualifier column");
    assert!(
        matches!(unknown, GraphError::UnknownColumn { .. }),
        "{unknown:?}"
    );
    let twice = measurement("/")?
        .value("measurement_concept_id", Value::Integer(2))
        .expect_err("a column is set once");
    assert!(matches!(twice, GraphError::Duplicate { .. }), "{twice:?}");
    Ok(())
}

#[test]
fn a_value_of_another_datatype_is_refused() -> Result<(), Box<dyn Error>> {
    for (column, value) in [
        ("value_as_number", Value::Integer(7)),
        ("measurement_date", Value::Text(String::from("2026-06-15"))),
        ("measurement_source_value", Value::Integer(7)),
        ("value_as_concept_id", Value::Float(1.0)),
    ] {
        let error = measurement("/")?
            .value(column, value)
            .expect_err("the datatype decides the value");
        assert!(
            matches!(error, GraphError::Mismatch { .. }),
            "{column}: {error:?}"
        );
    }
    Ok(())
}

#[test]
fn text_is_bounded_in_characters_by_its_varchar() -> Result<(), Box<dyn Error>> {
    let fits = "µ".repeat(50);
    measurement("/")?.value("measurement_source_value", Value::Text(fits))?;
    let error = measurement("/")?
        .value("measurement_source_value", Value::Text("µ".repeat(51)))
        .expect_err("51 characters exceed varchar(50)");
    assert_eq!(
        GraphError::TooLong {
            table: "measurement",
            column: "measurement_source_value",
            limit: 50,
            length: 51,
        },
        error
    );
    Ok(())
}

#[test]
fn a_float_that_is_not_finite_is_refused() -> Result<(), Box<dyn Error>> {
    for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = measurement("/")?
            .value("value_as_number", Value::Float(number))
            .expect_err("NUMERIC holds no NaN or infinity here");
        assert!(matches!(error, GraphError::NotFinite { .. }), "{error:?}");
    }
    Ok(())
}

#[test]
fn a_reference_must_fit_the_columns_foreign_key() -> Result<(), Box<dyn Error>> {
    let visit = Reference::Visit(VisitKey::new(
        HierObjectId::new(EHR)?,
        VisitSource::new("encounter-1")?,
    ));
    measurement("/")?.reference("visit_occurrence_id", visit.clone())?;
    let refused = [
        ("provider_id", Reference::Person(HierObjectId::new(EHR)?)),
        ("measurement_concept_id", Reference::Row(key("/")?)),
        ("measurement_source_value", visit.clone()),
        ("visit_occurrence_id", Reference::Row(key("/")?)),
    ];
    let builder = Row::builder("measurement", key("/")?)?;
    for (column, reference) in refused {
        let error = builder
            .clone()
            .reference(column, reference)
            .expect_err("the foreign key decides the reference");
        assert!(
            matches!(error, GraphError::Reference { .. }),
            "{column}: {error:?}"
        );
    }
    Ok(())
}

#[test]
fn a_graph_refuses_a_row_of_another_composition() -> Result<(), Box<dyn Error>> {
    let mut graph = graph()?;
    let other = RecordKey::new(
        &Source::new(
            HierObjectId::new(EHR)?,
            ObjectVersionId::new("another-composition::ferrobridge.test::1")?,
        ),
        ArchetypeRootPath::new("/content[x]")?,
        OccurrencePath::new("/")?,
        Discriminator::new(MappingName::new("Laboratory_test_analyte_v1")?, 0, 0),
    );
    let row = Row::builder("measurement", other)?
        .reference("person_id", Reference::Person(HierObjectId::new(EHR)?))?
        .value("measurement_concept_id", Value::Integer(1001))?
        .value("measurement_date", date("2026-06-15")?)?
        .value("measurement_type_concept_id", Value::Integer(32817))?
        .build()?;
    let error = graph
        .push_row(row)
        .expect_err("the key names another composition");
    assert!(matches!(error, GraphError::ForeignRow { .. }), "{error:?}");
    Ok(())
}

#[test]
fn a_graph_refuses_two_rows_of_one_table_under_one_key() -> Result<(), Box<dyn Error>> {
    let mut graph = graph()?;
    let row = measurement("/items[at0001]")?
        .value("measurement_date", date("2026-06-15")?)?
        .build()?;
    graph.push_row(row.clone())?;
    let error = graph.push_row(row).expect_err("the key is taken");
    assert!(
        matches!(error, GraphError::DuplicateRow { .. }),
        "{error:?}"
    );
    Ok(())
}

#[test]
fn a_link_must_name_rows_the_graph_holds() -> Result<(), Box<dyn Error>> {
    let mut graph = graph()?;
    graph.push_row(
        measurement("/items[at0001]")?
            .value("measurement_date", date("2026-06-15")?)?
            .build()?,
    )?;
    let held = LinkEnd::new("measurement", key("/items[at0001]")?, 21)?;
    let missing = LinkEnd::new("measurement", key("/items[at0002]")?, 21)?;
    let error = graph
        .push_link(Link::new(held.clone(), missing))
        .expect_err("the second row is not in the graph");
    assert!(
        matches!(error, GraphError::DanglingLink { .. }),
        "{error:?}"
    );
    assert_eq!(
        0,
        Link::RELATIONSHIP_CONCEPT_ID,
        "no concept exists for these links"
    );
    Ok(())
}

#[test]
fn a_visit_source_is_bounded_by_visit_source_value() -> Result<(), Box<dyn Error>> {
    let day = CdmDate::new("2026-06-15")?;
    let long = VisitKey::new(HierObjectId::new(EHR)?, VisitSource::new("s".repeat(51))?);
    let error = Visit::new(long, (day.clone(), None), (day, None))
        .expect_err("visit_source_value is varchar(50)");
    assert!(matches!(error, GraphError::TooLong { .. }), "{error:?}");
    Ok(())
}

#[test]
fn an_empty_identifier_is_refused() {
    assert!(
        HierObjectId::new("").is_err(),
        "an empty ehr_id names nothing"
    );
    assert!(
        MappingName::new("").is_err(),
        "an empty mapping name names nothing"
    );
}

#[test]
fn a_source_takes_its_versioned_composition_from_the_version_as_written()
-> Result<(), Box<dyn Error>> {
    let upper = COMPOSITION.to_uppercase();
    let source = Source::new(
        HierObjectId::new(EHR)?,
        ObjectVersionId::new(format!("{upper}::ferrobridge.test::2"))?,
    );
    assert_eq!(upper, source.versioned_object_uid().value());
    assert_eq!(
        format!("{upper}::ferrobridge.test::2"),
        source.version_uid().value()
    );
    Ok(())
}
