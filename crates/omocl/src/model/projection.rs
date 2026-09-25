// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The projection from an OMOCL key to the CDM columns it writes, per target.
//!
//! OMOCL states the projection for four targets only, in the syntax tables of
//! its wiki (<https://github.com/SevKohler/OMOCL/wiki/Syntax-and-grammar>:
//! `DrugExposure`, `ConditionOccurrence`, `ProcedureOccurrence`, `Death`, each
//! pairing an OMOCL field with the CDM fields it fills). For the other six the
//! mapping library is the only evidence of the keys, and the columns are
//! FerroBRIDGE's own reading of the CDM v5.4 field definitions. The table is
//! data: the lowering refuses a key it does not list, and the engine reads the
//! columns and parts from it.
//!
//! Every column named here exists in `omop_cdm::meta::table` of its target's
//! table, which the crate's tests assert for the whole table.

use crate::model::ast::Target;

/// What a column takes from the value an alternative produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Part {
    /// The standard concept of a coded value or a literal concept id.
    Concept,
    /// The source concept of a coded value.
    SourceConcept,
    /// The source code or text, verbatim.
    SourceValue,
    /// The date of a date or date-time value.
    Date,
    /// The date-time of a date or date-time value.
    Datetime,
    /// The year of a date value.
    Year,
    /// A numeric value; for a `DV_QUANTITY` its `magnitude`.
    Number,
    /// A text value.
    Text,
    /// The concept of a `DV_QUANTITY`'s `units`.
    Units,
    /// A `DV_QUANTITY`'s `normal_range.lower`.
    RangeLow,
    /// A `DV_QUANTITY`'s `normal_range.upper`.
    RangeHigh,
    /// The concept of a `DV_QUANTITY`'s `magnitude_status`.
    Operator,
    /// The id of a row of another CDM table, resolved from the value.
    Reference,
}

/// How the columns of one key are filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Fill {
    /// Every column is written.
    Every,
    /// Exactly one of the columns whose part is [`Part::Number`],
    /// [`Part::Concept`] or [`Part::Text`] is written, chosen by the RM type of
    /// the value read; every other column is written.
    OneByValueKind,
}

/// Where the key of a projection is attested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Attestation {
    /// A syntax table of the wiki pairs the key with its columns.
    SyntaxTable,
    /// The mapping library writes the key; no grammar artefact documents it.
    Library,
    /// Both a syntax table and the library.
    Both,
}

/// One CDM column a key writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColumnProjection {
    /// The column, as `omop_cdm::meta::ColumnMeta::name` spells it.
    pub column: &'static str,
    /// What the column takes from the value.
    pub part: Part,
}

/// One OMOCL key of one target and the columns it writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyProjection {
    /// The key, as a mapping file writes it.
    pub key: &'static str,
    /// How the columns are filled.
    pub fill: Fill,
    /// Where the key is attested.
    pub attestation: Attestation,
    /// The columns, in CDM definition order.
    pub columns: &'static [ColumnProjection],
}

/// The projection of one target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetProjection {
    /// The target.
    pub target: Target,
    /// Every key the target admits, in key order.
    pub keys: &'static [KeyProjection],
}

impl TargetProjection {
    /// Returns the projection of `key`, when the target admits it.
    #[must_use]
    pub fn key(&self, key: &str) -> Option<&'static KeyProjection> {
        self.keys.iter().find(|projection| projection.key == key)
    }
}

/// A key the library writes under a target whose CDM table has no column for
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyWithoutColumn {
    /// The target.
    pub target: Target,
    /// The key.
    pub key: &'static str,
    /// Why no column exists, citing the CDM.
    pub reason: &'static str,
}

/// Every key the library writes that no CDM column receives.
///
/// Loading a file that writes one is refused with its reason, since writing
/// the value nowhere would drop clinical content with no trace.
pub const KEYS_WITHOUT_COLUMN: &[KeyWithoutColumn] = &[KeyWithoutColumn {
    target: Target::Measurement,
    key: "qualifier",
    reason: "the CDM v5.4 MEASUREMENT table has no qualifier column; OBSERVATION's \
             qualifier_concept_id has no MEASUREMENT counterpart",
}];

/// Returns the projection of `target`.
#[must_use]
pub const fn projection(target: Target) -> &'static TargetProjection {
    match target {
        Target::Measurement => &MEASUREMENT,
        Target::Observation => &OBSERVATION,
        Target::DrugExposure => &DRUG_EXPOSURE,
        Target::ConditionOccurrence => &CONDITION_OCCURRENCE,
        Target::ProcedureOccurrence => &PROCEDURE_OCCURRENCE,
        Target::DeviceExposure => &DEVICE_EXPOSURE,
        Target::Specimen => &SPECIMEN,
        Target::Visit => &VISIT,
        Target::Death => &DEATH,
        Target::Person => &PERSON,
    }
}

/// Returns the entry of [`KEYS_WITHOUT_COLUMN`] for `key` under `target`.
#[must_use]
pub fn key_without_column(target: Target, key: &str) -> Option<&'static KeyWithoutColumn> {
    KEYS_WITHOUT_COLUMN
        .iter()
        .find(|entry| entry.target == target && entry.key == key)
}

/// Builds one column of a projection.
const fn col(column: &'static str, part: Part) -> ColumnProjection {
    ColumnProjection { column, part }
}

/// Builds a key every column of which is written.
const fn every(
    key: &'static str,
    attestation: Attestation,
    columns: &'static [ColumnProjection],
) -> KeyProjection {
    KeyProjection {
        key,
        fill: Fill::Every,
        attestation,
        columns,
    }
}

/// The projection of a `DV_QUANTITY` or coded `value` into MEASUREMENT.
const MEASUREMENT_VALUE: KeyProjection = KeyProjection {
    key: "value",
    fill: Fill::OneByValueKind,
    attestation: Attestation::Library,
    columns: &[
        col("value_as_number", Part::Number),
        col("value_as_concept_id", Part::Concept),
        col("value_source_value", Part::SourceValue),
    ],
};

/// `MEASUREMENT`.
const MEASUREMENT: TargetProjection = TargetProjection {
    target: Target::Measurement,
    keys: &[
        every(
            "concept_id",
            Attestation::Library,
            &[
                col("measurement_concept_id", Part::Concept),
                col("measurement_source_value", Part::SourceValue),
                col("measurement_source_concept_id", Part::SourceConcept),
            ],
        ),
        every(
            "measurement_date",
            Attestation::Library,
            &[
                col("measurement_date", Part::Date),
                col("measurement_datetime", Part::Datetime),
            ],
        ),
        every(
            "operator_concept_id",
            Attestation::Library,
            &[col("operator_concept_id", Part::Operator)],
        ),
        every(
            "range_high",
            Attestation::Library,
            &[col("range_high", Part::RangeHigh)],
        ),
        every(
            "range_low",
            Attestation::Library,
            &[col("range_low", Part::RangeLow)],
        ),
        every(
            "unit",
            Attestation::Library,
            &[
                col("unit_concept_id", Part::Units),
                col("unit_source_value", Part::SourceValue),
                col("unit_source_concept_id", Part::SourceConcept),
            ],
        ),
        MEASUREMENT_VALUE,
    ],
};

/// `OBSERVATION`.
const OBSERVATION: TargetProjection = TargetProjection {
    target: Target::Observation,
    keys: &[
        every(
            "concept_id",
            Attestation::Library,
            &[
                col("observation_concept_id", Part::Concept),
                col("observation_source_value", Part::SourceValue),
                col("observation_source_concept_id", Part::SourceConcept),
            ],
        ),
        every(
            "observation_date",
            Attestation::Library,
            &[
                col("observation_date", Part::Date),
                col("observation_datetime", Part::Datetime),
            ],
        ),
        every(
            "qualifier",
            Attestation::Library,
            &[
                col("qualifier_concept_id", Part::Concept),
                col("qualifier_source_value", Part::SourceValue),
            ],
        ),
        every(
            "unit",
            Attestation::Library,
            &[
                col("unit_concept_id", Part::Units),
                col("unit_source_value", Part::SourceValue),
            ],
        ),
        KeyProjection {
            key: "value",
            fill: Fill::OneByValueKind,
            attestation: Attestation::Library,
            columns: &[
                col("value_as_number", Part::Number),
                col("value_as_string", Part::Text),
                col("value_as_concept_id", Part::Concept),
                col("value_source_value", Part::SourceValue),
            ],
        },
    ],
};

/// `DRUG_EXPOSURE`, from the `DrugExposure` syntax table and the library.
const DRUG_EXPOSURE: TargetProjection = TargetProjection {
    target: Target::DrugExposure,
    keys: &[
        every(
            "concept_id",
            Attestation::Both,
            &[
                col("drug_concept_id", Part::Concept),
                col("drug_source_value", Part::SourceValue),
                col("drug_source_concept_id", Part::SourceConcept),
            ],
        ),
        every(
            "days_supply",
            Attestation::SyntaxTable,
            &[col("days_supply", Part::Number)],
        ),
        every(
            "dose_unit_source_value",
            Attestation::SyntaxTable,
            &[col("dose_unit_source_value", Part::Text)],
        ),
        every(
            "drug_exposure_end_date",
            Attestation::Both,
            &[
                col("drug_exposure_end_date", Part::Date),
                col("drug_exposure_end_datetime", Part::Datetime),
            ],
        ),
        // NOTE: the `DrugExposure` syntax table writes the second column as
        // `drug_exposure_start_date_time`, which CDM v5.4 spells `_datetime`.
        every(
            "drug_exposure_start_date",
            Attestation::Both,
            &[
                col("drug_exposure_start_date", Part::Date),
                col("drug_exposure_start_datetime", Part::Datetime),
            ],
        ),
        every(
            "lot_number",
            Attestation::SyntaxTable,
            &[col("lot_number", Part::Text)],
        ),
        every(
            "provider_id",
            Attestation::Both,
            &[col("provider_id", Part::Reference)],
        ),
        every(
            "quantity",
            Attestation::Library,
            &[col("quantity", Part::Number)],
        ),
        every(
            "refills",
            Attestation::SyntaxTable,
            &[col("refills", Part::Number)],
        ),
        every(
            "route_concept_id",
            Attestation::Both,
            &[
                col("route_concept_id", Part::Concept),
                col("route_source_value", Part::SourceValue),
            ],
        ),
        every("sig", Attestation::SyntaxTable, &[col("sig", Part::Text)]),
        every(
            "stop_reason",
            Attestation::SyntaxTable,
            &[col("stop_reason", Part::Text)],
        ),
        every(
            "verbatim_end_date",
            Attestation::SyntaxTable,
            &[col("verbatim_end_date", Part::Date)],
        ),
    ],
};

/// `CONDITION_OCCURRENCE`, from the `ConditionOccurrence` syntax table and the
/// library.
const CONDITION_OCCURRENCE: TargetProjection = TargetProjection {
    target: Target::ConditionOccurrence,
    keys: &[
        every(
            "concept_id",
            Attestation::Both,
            &[
                col("condition_concept_id", Part::Concept),
                col("condition_source_value", Part::SourceValue),
                col("condition_source_concept_id", Part::SourceConcept),
            ],
        ),
        every(
            "condition_end_date",
            Attestation::Both,
            &[
                col("condition_end_date", Part::Date),
                col("condition_end_datetime", Part::Datetime),
            ],
        ),
        every(
            "condition_start_date",
            Attestation::Both,
            &[
                col("condition_start_date", Part::Date),
                col("condition_start_datetime", Part::Datetime),
            ],
        ),
        every(
            "condition_status_concept_id",
            Attestation::Both,
            &[
                col("condition_status_concept_id", Part::Concept),
                col("condition_status_source_value", Part::SourceValue),
            ],
        ),
        every(
            "provider_id",
            Attestation::Both,
            &[col("provider_id", Part::Reference)],
        ),
        every(
            "stop_reason",
            Attestation::SyntaxTable,
            &[col("stop_reason", Part::Text)],
        ),
        every(
            "visit_detail_id",
            Attestation::Library,
            &[col("visit_detail_id", Part::Reference)],
        ),
    ],
};

/// `PROCEDURE_OCCURRENCE`, from the `ProcedureOccurrence` syntax table and the
/// library.
const PROCEDURE_OCCURRENCE: TargetProjection = TargetProjection {
    target: Target::ProcedureOccurrence,
    keys: &[
        // NOTE: the `ProcedureOccurrence` syntax table pairs `concept_id` with
        // `condition_concept_id`, a column PROCEDURE_OCCURRENCE does not have.
        every(
            "concept_id",
            Attestation::Both,
            &[
                col("procedure_concept_id", Part::Concept),
                col("procedure_source_value", Part::SourceValue),
                col("procedure_source_concept_id", Part::SourceConcept),
            ],
        ),
        every(
            "modifier",
            Attestation::SyntaxTable,
            &[
                col("modifier_concept_id", Part::Concept),
                col("modifier_source_value", Part::SourceValue),
            ],
        ),
        every(
            "procedure_date",
            Attestation::SyntaxTable,
            &[
                col("procedure_date", Part::Date),
                col("procedure_datetime", Part::Datetime),
            ],
        ),
        every(
            "procedure_end_date",
            Attestation::Both,
            &[
                col("procedure_end_date", Part::Date),
                col("procedure_end_datetime", Part::Datetime),
            ],
        ),
        // NOTE: the library writes `procedure_start_date` where the syntax
        // table and CDM v5.4 write `procedure_date`; both keys project there.
        every(
            "procedure_start_date",
            Attestation::Library,
            &[
                col("procedure_date", Part::Date),
                col("procedure_datetime", Part::Datetime),
            ],
        ),
        every(
            "provider_id",
            Attestation::SyntaxTable,
            &[col("provider_id", Part::Reference)],
        ),
        every(
            "quantity",
            Attestation::SyntaxTable,
            &[col("quantity", Part::Number)],
        ),
    ],
};

/// `DEVICE_EXPOSURE`.
const DEVICE_EXPOSURE: TargetProjection = TargetProjection {
    target: Target::DeviceExposure,
    keys: &[
        every(
            "concept_id",
            Attestation::Library,
            &[
                col("device_concept_id", Part::Concept),
                col("device_source_value", Part::SourceValue),
                col("device_source_concept_id", Part::SourceConcept),
            ],
        ),
        every(
            "device_exposure_end_date",
            Attestation::Library,
            &[
                col("device_exposure_end_date", Part::Date),
                col("device_exposure_end_datetime", Part::Datetime),
            ],
        ),
        every(
            "device_exposure_start_date",
            Attestation::Library,
            &[
                col("device_exposure_start_date", Part::Date),
                col("device_exposure_start_datetime", Part::Datetime),
            ],
        ),
        every(
            "production_id",
            Attestation::Library,
            &[col("production_id", Part::Text)],
        ),
        every(
            "unique_device_id",
            Attestation::Library,
            &[col("unique_device_id", Part::Text)],
        ),
        every(
            "visit_detail_id",
            Attestation::Library,
            &[col("visit_detail_id", Part::Reference)],
        ),
    ],
};

/// `SPECIMEN`.
const SPECIMEN: TargetProjection = TargetProjection {
    target: Target::Specimen,
    keys: &[
        every(
            "anatomic_site_concept",
            Attestation::Library,
            &[
                col("anatomic_site_concept_id", Part::Concept),
                col("anatomic_site_source_value", Part::SourceValue),
            ],
        ),
        every(
            "concept_id",
            Attestation::Library,
            &[
                col("specimen_concept_id", Part::Concept),
                col("specimen_source_value", Part::SourceValue),
            ],
        ),
        every(
            "quantity",
            Attestation::Library,
            &[col("quantity", Part::Number)],
        ),
        every(
            "specimen_date",
            Attestation::Library,
            &[
                col("specimen_date", Part::Date),
                col("specimen_datetime", Part::Datetime),
            ],
        ),
        every(
            "specimen_source_id",
            Attestation::Library,
            &[col("specimen_source_id", Part::Text)],
        ),
    ],
};

/// `VISIT_OCCURRENCE`.
const VISIT: TargetProjection = TargetProjection {
    target: Target::Visit,
    keys: &[
        every(
            "end_datetime",
            Attestation::Library,
            &[
                col("visit_end_date", Part::Date),
                col("visit_end_datetime", Part::Datetime),
            ],
        ),
        every(
            "start_datetime",
            Attestation::Library,
            &[
                col("visit_start_date", Part::Date),
                col("visit_start_datetime", Part::Datetime),
            ],
        ),
        every(
            "visit_concept",
            Attestation::Library,
            &[
                col("visit_concept_id", Part::Concept),
                col("visit_source_value", Part::SourceValue),
                col("visit_source_concept_id", Part::SourceConcept),
            ],
        ),
    ],
};

/// `DEATH`, from the `Death` syntax table and the library.
const DEATH: TargetProjection = TargetProjection {
    target: Target::Death,
    keys: &[
        every(
            "cause",
            Attestation::SyntaxTable,
            &[
                col("cause_concept_id", Part::Concept),
                col("cause_source_value", Part::SourceValue),
                col("cause_source_concept_id", Part::SourceConcept),
            ],
        ),
        every(
            "death_date",
            Attestation::Both,
            &[
                col("death_date", Part::Date),
                col("death_datetime", Part::Datetime),
            ],
        ),
    ],
};

/// `PERSON`.
const PERSON: TargetProjection = TargetProjection {
    target: Target::Person,
    keys: &[
        every(
            "birth_datetime",
            Attestation::Library,
            &[col("birth_datetime", Part::Datetime)],
        ),
        every(
            "ethnicity_concept_id",
            Attestation::Library,
            &[
                col("ethnicity_concept_id", Part::Concept),
                col("ethnicity_source_value", Part::SourceValue),
                col("ethnicity_source_concept_id", Part::SourceConcept),
            ],
        ),
        every(
            "gender_concept",
            Attestation::Library,
            &[
                col("gender_concept_id", Part::Concept),
                col("gender_source_value", Part::SourceValue),
                col("gender_source_concept_id", Part::SourceConcept),
            ],
        ),
        every(
            "year_of_birth",
            Attestation::Library,
            &[col("year_of_birth", Part::Year)],
        ),
    ],
};

#[cfg(test)]
mod tests {
    use super::projection;
    use crate::model::ast::Target;

    #[test]
    fn every_target_lists_its_keys_in_key_order_once() {
        for target in Target::ALL {
            let keys: Vec<&str> = projection(*target).keys.iter().map(|k| k.key).collect();
            let mut sorted = keys.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(keys, sorted, "{target}");
            assert_eq!(projection(*target).target, *target);
        }
    }

    #[test]
    fn every_key_writes_at_least_one_column() {
        for target in Target::ALL {
            for key in projection(*target).keys {
                assert!(!key.columns.is_empty(), "{target}.{}", key.key);
            }
        }
    }
}
