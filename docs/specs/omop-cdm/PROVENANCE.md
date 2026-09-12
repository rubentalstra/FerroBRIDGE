<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the OMOP Common Data Model v5.4 definitions and PostgreSQL DDL

Vendored verbatim by `scripts/vendor/omop-cdm.sh`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/OHDSI/CommonDataModel>
- Pin: tag `v5.4.3`, which resolves to commit `746a15e0fb36a95ba6cc0993737f1273bbad92f2`
- Fetched: 2026-09-12
- Upstream licence: `Apache License 2.0`. **The repository has no `LICENSE` file.**
  `DESCRIPTION` is the only place the licence is declared, which is why that
  file is vendored here and the script fails if a `LICENSE` file ever appears.
- Layout: the upstream paths, unchanged
- Files: 7
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `c9bcab5c4f9ede06508fa5bf3c56c92ce4ab142e69835c05f1d21e993611550e`

## What is here

The two CSV files are the machine-readable definitions the `omop-cdm` row
types are generated from. The four PostgreSQL files are OHDSI's rendered DDL:
OHDSI renders them from the same CSVs through a dialect layer that sits outside
them, so they are vendored rather than generated, and a test asserts the
generated column set equals the DDL's (docs/architecture.md section 10).

| File | sha256 | git blob id |
|---|---|---|
| `inst/csv/OMOP_CDMv5.4_Field_Level.csv` | `2b763c7a2aeb309372c1564350939551531318e2078fd4443e03b2741e79b77c` | `fdec16107fba2a45c3ec360ef84c0d837a43998b` |
| `inst/csv/OMOP_CDMv5.4_Table_Level.csv` | `a8ad96bc9eece99b8496ea7ca4e5c61df7ec6be09688862e46e326dfc82b8284` | `8e6e2ad2e3b56a4b1c4fc3824785178904405e82` |
| `inst/ddl/5.4/postgresql/OMOPCDM_postgresql_5.4_ddl.sql` | `c340fc1723cc71c411aa7d5d67844882ec720bed1135eca2b8a9029d0a3ef854` | `162bdc5c863fec8ca32e07814a40a347f267788a` |
| `inst/ddl/5.4/postgresql/OMOPCDM_postgresql_5.4_primary_keys.sql` | `d8f50617f9a698bd9fbfe8d4106d76dcd543949a6976032786f64a91c1941f81` | `9304d020bcb007bca8ad8e57873b76a2b626e8d9` |
| `inst/ddl/5.4/postgresql/OMOPCDM_postgresql_5.4_indices.sql` | `ea23abbb327ee94e974728196cce2b2db466c7ad4bb4313f36fa3ce2388e8776` | `18b739493af0c3bcc90bff728e987a5b06a7bb0b` |
| `inst/ddl/5.4/postgresql/OMOPCDM_postgresql_5.4_constraints.sql` | `b09f54e6f1550c8594941a96798246944c10d42d9d4be8291a5058033be0f91d` | `19818c3cd59fa47d153160197a8be9399040a6b6` |
| `DESCRIPTION` | `88a1f1f5ba02ecc694b327d3a989b3cb97ed1cb6c12092c84094ce49f95f7aed` | `1a5c4f5215981853019f5a3cecf5f7e795d6b225` |
