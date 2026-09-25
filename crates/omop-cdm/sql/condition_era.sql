-- SPDX-FileCopyrightText: Observational Health Data Sciences and Informatics (OHDSI)
-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: Apache-2.0
--
-- The PostgreSQL form of the "Condition Eras" script of the OMOP CDM SQL
-- scripts page (https://ohdsi.github.io/CommonDataModel/sqlScripts.html),
-- source site/sqlScripts.qmd of OHDSI/CommonDataModel at tag v5.4.3.
-- The published script is OHDSI SQL for SqlRender; this file keeps its
-- statements and their order and changes only the dialect (sql/PROVENANCE.md).
-- @cdmDatabaseSchema is the CDM schema, as in OHDSI's rendered DDL.

DROP TABLE IF EXISTS pg_temp.condition_era_phase_1;

DROP TABLE IF EXISTS pg_temp.cteConditionTarget;

-- create base eras from the concepts found in condition_occurrence
CREATE TEMP TABLE cteConditionTarget AS
SELECT
  co.person_id,
  co.condition_concept_id,
  co.condition_start_date,
  COALESCE(co.condition_end_date, (condition_start_date + 1)) AS condition_end_date
FROM @cdmDatabaseSchema.CONDITION_OCCURRENCE co;

DROP TABLE IF EXISTS pg_temp.cteCondEndDates;

CREATE TEMP TABLE cteCondEndDates AS
SELECT
  person_id,
  condition_concept_id,
  (event_date - 30) AS end_date -- unpad the end date
FROM (
  SELECT
    e1.person_id,
    e1.condition_concept_id,
    e1.event_date,
    COALESCE(e1.start_ordinal, MAX(e2.start_ordinal)) start_ordinal,
    e1.overall_ord
  FROM (
    SELECT
      person_id,
      condition_concept_id,
      event_date,
      event_type,
      start_ordinal,
      ROW_NUMBER() OVER (
        PARTITION BY person_id
        ,condition_concept_id ORDER BY event_date
          ,event_type
        ) AS overall_ord -- this re-numbers the inner UNION so all rows are numbered ordered by the event date
    FROM (
      -- select the start dates, assigning a row number to each
      SELECT
        person_id,
        condition_concept_id,
        condition_start_date AS event_date,
        - 1 AS event_type,
        ROW_NUMBER() OVER (
          PARTITION BY person_id
          ,condition_concept_id ORDER BY condition_start_date
          ) AS start_ordinal
      FROM cteConditionTarget

      UNION ALL

      -- pad the end dates by 30 to allow a grace period for overlapping ranges.
      SELECT
        person_id,
        condition_concept_id,
        (condition_end_date + 30),
        1 AS event_type,
        NULL
      FROM cteConditionTarget
    ) RAWDATA
  ) e1
  INNER JOIN (
    SELECT
      person_id,
      condition_concept_id,
      condition_start_date AS event_date,
      ROW_NUMBER() OVER (
        PARTITION BY person_id
        ,condition_concept_id ORDER BY condition_start_date
        ) AS start_ordinal
    FROM cteConditionTarget
  ) e2 ON e1.person_id = e2.person_id
    AND e1.condition_concept_id = e2.condition_concept_id
    AND e2.event_date <= e1.event_date
  GROUP BY e1.person_id
    ,e1.condition_concept_id
    ,e1.event_date
    ,e1.start_ordinal
    ,e1.overall_ord
) e
WHERE (2 * e.start_ordinal) - e.overall_ord = 0;

DROP TABLE IF EXISTS pg_temp.cteConditionEnds;

CREATE TEMP TABLE cteConditionEnds AS
SELECT
  c.person_id,
  c.condition_concept_id,
  c.condition_start_date,
  MIN(e.end_date) AS era_end_date
FROM cteConditionTarget c
INNER JOIN cteCondEndDates e ON c.person_id = e.person_id
  AND c.condition_concept_id = e.condition_concept_id
  AND e.end_date >= c.condition_start_date
GROUP BY c.person_id
  ,c.condition_concept_id
  ,c.condition_start_date;

INSERT INTO @cdmDatabaseSchema.condition_era (
  condition_era_id
  ,person_id
  ,condition_concept_id
  ,condition_era_start_date
  ,condition_era_end_date
  ,condition_occurrence_count
  )
SELECT
  row_number() OVER (
    ORDER BY person_id
  ) AS condition_era_id,
  person_id,
  condition_concept_id,
  min(condition_start_date) AS condition_era_start_date,
  era_end_date AS condition_era_end_date,
  COUNT(*) AS condition_occurrence_count
FROM cteConditionEnds
GROUP BY person_id
  ,condition_concept_id
  ,era_end_date;
