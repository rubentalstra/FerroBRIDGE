// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OMOP round trip, the v0.0.4 acceptance test (`docs/architecture.md`
//! §11), behind the `FERROBRIDGE_E2E` gate.
//!
//! The reference CDR holds three synthetic laboratory compositions in two
//! EHRs, committed over ITS-REST 1.1.0 as canonical JSON. `etl run` reads
//! them over `POST /query/aql` through the job the binary calls, maps them
//! with the published OMOCL laboratory files copied verbatim from
//! `docs/specs/omocl`, and writes them into a CDM v5.4 PostgreSQL built from
//! the vendored DDL with the synthetic vocabulary loaded. The rows are a
//! reviewed snapshot over a projection that drops the surrogate ids and the
//! run's own identifiers.

use ferrobridge_openehr::client::Client;
use ferrobridge_openehr::commit::CommitContext;
use ferrobridge_openehr::composition::{CreateCompositionOutcome, UpdateCompositionOutcome};
use ferrobridge_openehr::config::Config;
use ferrobridge_openehr::ehr::CreateEhrOutcome;
use ferrobridge_openehr::ids::{EhrId, template_id, versioned_object_uid};
use ferrobridge_openehr::prefer::Prefer;
use ferrobridge_server::etl::RunOptions;
use ferrobridge_server::etl::report::RunReport;
use ferrobridge_testkit::containers::{self, Cdr, Postgres};
use ferrobridge_testkit::fixtures::{
    LABORATORY_REPORT_FLAT, LABORATORY_REPORT_OPT, LABORATORY_REPORT_TEMPLATE_ID,
};
use omop_cdm::database::{self, CdmPool};
use omop_cdm::ddl::SchemaName;
use omop_cdm::writer::{CdmWriter, PersonPolicy};
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::template::TemplateSource;
use openehr_rm::v1_2::composition::composition::Composition;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;

/// The two published laboratory files the acceptance test names, and the
/// specimen file the result file includes.
const LABORATORY_FILES: [&str; 3] = [
    "medical_data/observation/Laboratory_test_result_v1.yml",
    "medical_data/cluster/Laboratory_test_analyte_v1.yml",
    "medical_data/cluster/Specimen_v1.yml",
];

/// The composition query: each composition's latest version, whole.
const COMPOSITIONS: &str = "SELECT e/ehr_id/value AS ehr_id, \
    v/uid/value AS version_uid, c AS composition \
    FROM EHR e CONTAINS VERSION v[LATEST_VERSION] CONTAINS COMPOSITION c \
    ORDER BY c/context/start_time/value";

/// The visit query: one visit per EHR and health care facility, spanning the
/// contexts of the compositions recorded there.
const VISITS: &str = "SELECT e/ehr_id/value AS ehr_id, \
    c/context/health_care_facility/name AS visit_source, \
    c/context/start_time/value AS visit_start, c/context/end_time/value AS visit_end \
    FROM EHR e CONTAINS COMPOSITION c ORDER BY c/context/start_time/value";

/// The `*_type_concept_id` the run writes, a synthetic record type.
const TYPE_CONCEPT: i32 = 5101;

/// The `period_type_concept_id` of every observation period.
const PERIOD_TYPE_CONCEPT: i32 = 5102;

/// The `visit_concept_id` of every visit.
const VISIT_CONCEPT: i32 = 5001;

/// The FLAT path of the one event of the laboratory result.
const EVENT: &str = "synthetic_laboratory_report/synthetic_laboratory_result:0/any_event:0";

/// One synthetic analyte.
#[derive(Debug, Clone, Copy)]
struct Analyte {
    /// The analyte code, under the `LOINC` terminology.
    code: &'static str,
    /// The measured magnitude.
    magnitude: f64,
    /// The units.
    units: &'static str,
    /// The analyte's own result time, when it carries one.
    time: Option<&'static str>,
    /// The normal range, in the value's units.
    range: Option<(f64, f64)>,
    /// The `magnitude_status`, when the value is not a point value.
    status: Option<&'static str>,
}

/// One synthetic laboratory composition.
#[derive(Debug, Clone, Copy)]
struct Laboratory {
    /// The name the snapshot shows for it.
    label: &'static str,
    /// Which of the two EHRs holds it.
    ehr: usize,
    /// The health care facility of its context, the visit it belongs to.
    facility: &'static str,
    /// The context start and end times.
    context: (&'static str, &'static str),
    /// The event time.
    event: &'static str,
    /// The two analytes.
    analytes: [Analyte; 2],
    /// Whether it carries the specimen.
    specimen: bool,
}

/// The three compositions the round trip commits.
const LABORATORIES: [Laboratory; 3] = [
    Laboratory {
        label: "lab-1",
        ehr: 0,
        facility: "synthetic-ward-a",
        context: ("2026-09-20T07:30:00Z", "2026-09-20T12:00:00Z"),
        event: "2026-09-20T08:00:00Z",
        analytes: [
            Analyte {
                code: "SYN-LAB-1",
                magnitude: 7.25,
                units: "mmol/L",
                time: Some("2026-09-20T09:15:00Z"),
                range: Some((3.5, 6.5)),
                status: None,
            },
            Analyte {
                code: "SYN-LAB-2",
                magnitude: 140.0,
                units: "g/L",
                time: None,
                range: Some((120.0, 160.0)),
                status: Some("<"),
            },
        ],
        specimen: true,
    },
    Laboratory {
        label: "lab-2",
        ehr: 0,
        facility: "synthetic-ward-a",
        context: ("2026-09-21T08:00:00Z", "2026-09-21T09:00:00Z"),
        event: "2026-09-21T08:10:00Z",
        analytes: [
            Analyte {
                code: "SYN-LAB-1",
                magnitude: 5.1,
                units: "mmol/L",
                time: None,
                range: None,
                status: Some("="),
            },
            Analyte {
                code: "SYN-LAB-9",
                magnitude: 2.2,
                units: "mmol/L",
                time: None,
                range: None,
                status: Some(">="),
            },
        ],
        specimen: false,
    },
    Laboratory {
        label: "lab-3",
        ehr: 1,
        facility: "synthetic-ward-b",
        context: ("2026-09-22T10:00:00Z", "2026-09-22T11:00:00Z"),
        event: "2026-09-22T10:05:00Z",
        analytes: [
            Analyte {
                code: "SYN-LAB-2",
                magnitude: 150.0,
                units: "g/L",
                time: None,
                range: None,
                status: None,
            },
            Analyte {
                code: "SYN-LAB-1",
                magnitude: 6.0,
                units: "mmol/L",
                time: Some("2026-09-22T10:30:00Z"),
                range: Some((3.5, 6.5)),
                status: None,
            },
        ],
        specimen: false,
    },
];

/// Returns the FLAT form of `laboratory`, from the fixture's FLAT keys.
fn flat(laboratory: &Laboratory) -> Result<Map<String, Value>, Box<dyn Error>> {
    let mut flat: Map<String, Value> = serde_json::from_str(LABORATORY_REPORT_FLAT)?;
    flat.retain(|key, _| {
        !key.contains("/synthetic_analyte:") && (laboratory.specimen || !key.contains("specimen"))
    });
    flat.insert(
        String::from("synthetic_laboratory_report/context/start_time"),
        json!(laboratory.context.0),
    );
    flat.insert(format!("{EVENT}/time"), json!(laboratory.event));
    for (index, analyte) in laboratory.analytes.iter().enumerate() {
        let at = format!("{EVENT}/synthetic_analyte:{index}");
        flat.insert(format!("{at}/analyte_name|code"), json!(analyte.code));
        flat.insert(
            format!("{at}/analyte_name|value"),
            json!(format!("Synthetic analyte {}", analyte.code)),
        );
        flat.insert(format!("{at}/analyte_name|terminology"), json!("LOINC"));
        flat.insert(
            format!("{at}/result_value|magnitude"),
            json!(analyte.magnitude),
        );
        flat.insert(format!("{at}/result_value|unit"), json!(analyte.units));
        if let Some(time) = analyte.time {
            flat.insert(format!("{at}/result_time"), json!(time));
        }
    }
    Ok(flat)
}

/// Calls `edit` on every `DV_QUANTITY` below `value`, in document order.
fn each_quantity(value: &mut Value, edit: &mut dyn FnMut(&mut Map<String, Value>)) {
    match value {
        Value::Object(object) => {
            if object.get("_type").and_then(Value::as_str) == Some("DV_QUANTITY") {
                edit(object);
            } else {
                for member in object.values_mut() {
                    each_quantity(member, edit);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                each_quantity(item, edit);
            }
        }
        _ => {}
    }
}

/// Returns the canonical JSON of `laboratory`: the FLAT build, with the
/// context's end time and facility, and each analyte's range and status.
fn canonical(index: &WebTemplateIndex, laboratory: &Laboratory) -> Result<Value, Box<dyn Error>> {
    let mut value = index
        .build_from_flat(&flat(laboratory)?, "2026-09-25T00:00:00Z")?
        .into_value();
    let context = value
        .get_mut("context")
        .and_then(Value::as_object_mut)
        .ok_or("the composition carries a context")?;
    context.insert(
        String::from("end_time"),
        json!({"_type": "DV_DATE_TIME", "value": laboratory.context.1}),
    );
    context.insert(
        String::from("health_care_facility"),
        json!({"_type": "PARTY_IDENTIFIED", "name": laboratory.facility}),
    );
    let mut analytes = laboratory.analytes.iter();
    each_quantity(&mut value, &mut |quantity| {
        let Some(analyte) = analytes.next() else {
            return;
        };
        if let Some(status) = analyte.status {
            quantity.insert(String::from("magnitude_status"), json!(status));
        }
        if let Some((low, high)) = analyte.range {
            quantity.insert(
                String::from("normal_range"),
                json!({
                    "_type": "DV_INTERVAL",
                    "lower": {"_type": "DV_QUANTITY", "magnitude": low, "units": analyte.units},
                    "upper": {"_type": "DV_QUANTITY", "magnitude": high, "units": analyte.units},
                    "lower_unbounded": false,
                    "upper_unbounded": false,
                    "lower_included": true,
                    "upper_included": true
                }),
            );
        }
    });
    Ok(value)
}

/// Returns `value` as the RM composition the client commits.
fn composition(value: &Value) -> Result<Composition, Box<dyn Error>> {
    Ok(openehr_its::json::from_canonical_json::<Composition>(
        &value.to_string(),
    )?)
}

/// Returns the commit context of a laboratory composition.
fn commit() -> Result<CommitContext, Box<dyn Error>> {
    Ok(CommitContext {
        template_id: Some(template_id(LABORATORY_REPORT_TEMPLATE_ID)?),
        ..CommitContext::default()
    })
}

/// Uploads the laboratory template through the CDR's own route.
async fn upload_template(cdr: &Cdr) -> Result<(), Box<dyn Error>> {
    let response = reqwest::Client::new()
        .post(format!("{}/definition/template/adl1.4", cdr.base_url()))
        .header("Content-Type", "application/xml")
        .body(LABORATORY_REPORT_OPT)
        .send()
        .await?;
    let status = response.status();
    if status != reqwest::StatusCode::CREATED {
        return Err(format!(
            "the CDR refused the template: {status} {}",
            response.text().await?
        )
        .into());
    }
    Ok(())
}

/// Creates an EHR and returns its identifier.
async fn create_ehr(client: &Client) -> Result<EhrId, Box<dyn Error>> {
    match client.create_ehr(None, Prefer::Minimal).await? {
        CreateEhrOutcome::Created { ehr_id, .. } => Ok(ehr_id),
        other => Err(format!("the CDR refused the EHR: {other:?}").into()),
    }
}

/// Commits the first version of `value` into `ehr`.
async fn create(
    client: &Client,
    ehr: &EhrId,
    value: &Value,
) -> Result<ObjectVersionId, Box<dyn Error>> {
    match client
        .create_composition(ehr, &composition(value)?, &commit()?, Prefer::Minimal)
        .await?
    {
        CreateCompositionOutcome::Created { version_id, .. } => Ok(version_id),
        other => Err(format!("the CDR refused the composition: {other:?}").into()),
    }
}

/// Commits `value` as the next version after `preceding`.
async fn update(
    client: &Client,
    ehr: &EhrId,
    preceding: &ObjectVersionId,
    value: &Value,
) -> Result<ObjectVersionId, Box<dyn Error>> {
    match client
        .update_composition(
            ehr,
            &versioned_object_uid(preceding),
            preceding,
            &composition(value)?,
            &commit()?,
            Prefer::Minimal,
        )
        .await?
    {
        UpdateCompositionOutcome::Updated { version_id, .. } => Ok(version_id),
        other => Err(format!("the CDR refused the new version: {other:?}").into()),
    }
}

/// Copies the laboratory files from the vendored corpus into `directory`,
/// checking that every copied byte equals the vendored one.
fn copy_mappings(directory: &Path) -> Result<(), Box<dyn Error>> {
    let corpus = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/specs/omocl"
    ));
    for file in LABORATORY_FILES {
        let from = corpus.join(file);
        let to = directory.join(from.file_name().ok_or("a corpus file name")?);
        std::fs::copy(&from, &to)?;
        assert_eq!(
            std::fs::read(&from)?,
            std::fs::read(&to)?,
            "{file} is copied verbatim"
        );
    }
    Ok(())
}

/// Writes the configuration `etl run` reads: both containers, the two
/// queries, the configured concepts, and the OMOCL directory.
fn configuration(
    directory: &Path,
    cdr: &Cdr,
    postgres: &Postgres,
    mappings: &Path,
) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let text = format!(
        "[cdr]\nbase_url = \"{cdr}/\"\n\n\
         [cdm]\nurl = \"{cdm}\"\n\n\
         [etl]\naql = \"{COMPOSITIONS}\"\ntype_concept_id = {TYPE_CONCEPT}\n\
         observation_period_type_concept_id = {PERIOD_TYPE_CONCEPT}\n\n\
         [etl.visits]\naql = \"{VISITS}\"\nvisit_concept_id = {VISIT_CONCEPT}\n\
         visit_type_concept_id = {TYPE_CONCEPT}\n\n\
         [mappings]\nomocl = \"{omocl}\"\n",
        cdr = cdr.base_url(),
        cdm = postgres.url(),
        omocl = mappings.display(),
    );
    let path = directory.join("ferrobridge.toml");
    std::fs::write(&path, text)?;
    Ok(path)
}

/// Builds the CDM schema and the bridge schema, and loads the synthetic
/// vocabulary.
async fn build_cdm(postgres: &Postgres) -> Result<(), Box<dyn Error>> {
    let pool = CdmPool::connect(
        sqlx::postgres::PgPoolOptions::new().max_connections(1),
        postgres.url().parse()?,
        SchemaName::new("cdm")?,
    )
    .await?;
    database::init(&pool).await?;
    let mut connection = pool.pool().acquire().await?;
    ferrobridge_testkit::vocabulary::load(&mut connection).await?;
    drop(connection);
    pool.pool().close().await;
    let mut writer = CdmWriter::connect(
        postgres.url(),
        SchemaName::new("cdm")?,
        SchemaName::new("ferrobridge")?,
        PersonPolicy::CreateOnFirstSight,
    )
    .await?;
    writer.init().await?;
    Ok(())
}

/// Runs `etl run` once through the job the binary calls.
async fn etl_run(config: &Path) -> Result<RunReport, Box<dyn Error>> {
    let settings = ferrobridge_server::config::Config::load(Some(config))?.resolve()?;
    Ok(ferrobridge_server::etl::job::run(&settings, &RunOptions::default()).await?)
}

/// Returns every row `query` answers, each cell rendered as text or `NULL`.
async fn rows(postgres: &Postgres, query: &str) -> Result<Vec<Vec<String>>, Box<dyn Error>> {
    let (client, connection) =
        tokio_postgres::connect(postgres.url(), tokio_postgres::NoTls).await?;
    tokio::spawn(connection);
    let answered = client.simple_query(query).await?;
    let mut rows = Vec::new();
    for message in answered {
        if let tokio_postgres::SimpleQueryMessage::Row(row) = message {
            rows.push(
                (0..row.len())
                    .map(|column| row.get(column).unwrap_or("NULL").to_owned())
                    .collect(),
            );
        }
    }
    Ok(rows)
}

/// The deterministic projection of the `MEASUREMENT` rows: the composition,
/// the natural key, the person's EHR and the visit's source in place of the
/// surrogate ids.
const MEASUREMENTS: &str = "SELECT r.versioned_object_uid, r.archetype_root_path, r.occurrence_path, r.mapping, \
    r.entry, r.branch, p.ehr_id, v.visit_source_value, m.measurement_concept_id, \
    m.measurement_source_value, m.measurement_source_concept_id, m.measurement_date, \
    m.measurement_datetime, m.value_as_number, m.value_source_value, m.unit_concept_id, \
    m.unit_source_value, m.unit_source_concept_id, m.range_low, m.range_high, \
    m.operator_concept_id, m.measurement_type_concept_id \
    FROM ferrobridge.record r \
    JOIN cdm.measurement m ON r.cdm_table = 'measurement' AND m.measurement_id = r.surrogate_id \
    JOIN ferrobridge.person_map p ON p.person_id = m.person_id \
    LEFT JOIN cdm.visit_occurrence v ON v.visit_occurrence_id = m.visit_occurrence_id";

/// The projection of the `SPECIMEN` rows.
const SPECIMENS: &str = "SELECT r.versioned_object_uid, r.archetype_root_path, r.occurrence_path, p.ehr_id, \
    s.specimen_concept_id, s.specimen_source_value, s.specimen_date, s.specimen_datetime, \
    s.specimen_type_concept_id \
    FROM ferrobridge.record r \
    JOIN cdm.specimen s ON r.cdm_table = 'specimen' AND s.specimen_id = r.surrogate_id \
    JOIN ferrobridge.person_map p ON p.person_id = s.person_id";

/// The projection of the `FACT_RELATIONSHIP` rows: each end named by its
/// table and natural key.
const FACTS: &str = "SELECT f.domain_concept_id_1, r1.cdm_table, r1.versioned_object_uid, \
    r1.archetype_root_path, r1.occurrence_path, f.domain_concept_id_2, r2.cdm_table, \
    r2.archetype_root_path, r2.occurrence_path, \
    f.relationship_concept_id \
    FROM cdm.fact_relationship f \
    JOIN ferrobridge.record r1 ON r1.surrogate_id = f.fact_id_1 \
      AND r1.cdm_table = CASE f.domain_concept_id_1 WHEN 9011 THEN 'measurement' ELSE 'specimen' END \
    JOIN ferrobridge.record r2 ON r2.surrogate_id = f.fact_id_2 \
      AND r2.cdm_table = CASE f.domain_concept_id_2 WHEN 9011 THEN 'measurement' ELSE 'specimen' END";

/// The projection of the `VISIT_OCCURRENCE` and `OBSERVATION_PERIOD` rows.
const VISITS_AND_PERIODS: &str = "SELECT 'visit', p.ehr_id, v.visit_source_value, \
    v.visit_concept_id, v.visit_start_date, v.visit_start_datetime, v.visit_end_date, \
    v.visit_end_datetime, v.visit_type_concept_id \
    FROM cdm.visit_occurrence v JOIN ferrobridge.person_map p ON p.person_id = v.person_id \
    UNION ALL SELECT 'observation_period', p.ehr_id, NULL, NULL, \
    o.observation_period_start_date, NULL, o.observation_period_end_date, NULL, \
    o.period_type_concept_id \
    FROM cdm.observation_period o JOIN ferrobridge.person_map p ON p.person_id = o.person_id";

/// Renders the rows of `query`, identifiers replaced by the names in
/// `names`, sorted.
async fn projected(
    postgres: &Postgres,
    query: &str,
    names: &BTreeMap<String, String>,
) -> Result<String, Box<dyn Error>> {
    let mut lines: Vec<String> = rows(postgres, query)
        .await?
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|cell| names.get(&cell).cloned().unwrap_or(cell))
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .collect();
    lines.sort();
    Ok(lines.join("\n"))
}

/// Renders the whole projection the snapshot pins.
async fn projection(
    postgres: &Postgres,
    names: &BTreeMap<String, String>,
) -> Result<String, Box<dyn Error>> {
    Ok(format!(
        "MEASUREMENT\n{}\n\nSPECIMEN\n{}\n\nFACT_RELATIONSHIP\n{}\n\nVISIT_OCCURRENCE and OBSERVATION_PERIOD\n{}\n",
        projected(postgres, MEASUREMENTS, names).await?,
        projected(postgres, SPECIMENS, names).await?,
        projected(postgres, FACTS, names).await?,
        projected(postgres, VISITS_AND_PERIODS, names).await?,
    ))
}

/// Returns the data of the CDM and bridge schemas as `pg_dump` writes it,
/// each table's rows sorted and the watermark's run columns cut.
fn dump(postgres: &Postgres) -> Result<String, Box<dyn Error>> {
    crate::etl::dump(postgres)
}

#[tokio::test]
async fn the_laboratory_round_trip_writes_the_reviewed_rows_and_a_rerun_changes_nothing()
-> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let cdr = containers::cdr().await?;
    let postgres = containers::postgres().await?;
    build_cdm(&postgres).await?;
    upload_template(&cdr).await?;
    let client = Client::new(Config::new(format!("{}/", cdr.base_url()).parse()?))?;
    let ehrs = [create_ehr(&client).await?, create_ehr(&client).await?];

    let index = WebTemplateIndex::build(&TemplateSource::opt14(LABORATORY_REPORT_OPT)?)?;
    let mut names = BTreeMap::new();
    for (position, ehr) in ehrs.iter().enumerate() {
        names.insert(ehr.as_str().to_owned(), format!("ehr-{}", position + 1));
    }
    let mut versions = Vec::new();
    for laboratory in &LABORATORIES {
        let ehr = ehrs.get(laboratory.ehr).ok_or("an EHR")?;
        let version = create(&client, ehr, &canonical(&index, laboratory)?).await?;
        names.insert(
            versioned_object_uid(&version).value().to_owned(),
            laboratory.label.to_owned(),
        );
        versions.push(version);
    }

    let scratch = tempfile::tempdir()?;
    let mappings = tempfile::tempdir()?;
    copy_mappings(mappings.path())?;
    let config = configuration(scratch.path(), &cdr, &postgres, mappings.path())?;

    let first = etl_run(&config).await?;
    assert_eq!(3, first.compositions.committed, "{first}");
    assert!(first.refusals.is_empty(), "{first}");
    assert_eq!(3, first.compositions.with_visit, "{first}");
    assert_eq!(Some(&6), first.rows.get("measurement"), "{first}");
    assert_eq!(Some(&1), first.rows.get("specimen"), "{first}");
    assert_eq!(2, first.derived.visit_occurrence, "{first}");
    assert_eq!(2, first.derived.observation_period, "{first}");
    let zero = first
        .mappings
        .get("Laboratory_test_analyte_v1")
        .and_then(|totals| {
            totals
                .concept_zero
                .get("measurement.measurement_concept_id")
        });
    assert_eq!(Some(&1), zero, "the unmapped analyte is counted: {first}");
    assert_eq!(
        vec![vec![String::from("0")]],
        rows(
            &postgres,
            "SELECT count(*) FROM cdm.measurement WHERE visit_occurrence_id IS NULL"
        )
        .await?,
        "every clinical row carries its visit"
    );
    insta::assert_snapshot!(
        "laboratory_round_trip",
        projection(&postgres, &names).await?
    );
    let before = dump(&postgres)?;

    let second = etl_run(&config).await?;
    assert_eq!(3, second.compositions.committed, "{second}");
    assert_eq!(
        before,
        dump(&postgres)?,
        "a second run leaves the data as it was"
    );

    let altered = LABORATORIES.get(1).ok_or("the second composition")?;
    let mut changed = *altered;
    if let Some(analyte) = changed.analytes.first_mut() {
        analyte.magnitude = 5.4;
    }
    let preceding = versions.get(1).ok_or("the second version")?;
    let ehr = ehrs.get(altered.ehr).ok_or("an EHR")?;
    update(&client, ehr, preceding, &canonical(&index, &changed)?).await?;

    let rows_before = projection(&postgres, &names).await?;
    let third = etl_run(&config).await?;
    assert_eq!(3, third.compositions.committed, "{third}");
    assert_ne!(
        before,
        dump(&postgres)?,
        "the altered composition changes the data"
    );
    let rows_after = projection(&postgres, &names).await?;
    let differing: Vec<(&str, &str)> = rows_before
        .lines()
        .zip(rows_after.lines())
        .filter(|(was, is)| was != is)
        .collect();
    assert_eq!(1, differing.len(), "exactly one row changed: {differing:?}");
    let (was, is) = differing.first().ok_or("the changed row")?;
    assert!(
        was.starts_with("lab-2 |") && was.contains("| 5.1 |") && is.contains("| 5.4 |"),
        "the altered analyte of lab-2 is the row replaced: {was} / {is}"
    );
    Ok(())
}
