// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every vendored corpus under `docs/specs/` is read here, so a tree that
//! drifts from its pin, loses files, or is hand-edited fails the build
//! (.claude/rules/vendored-inputs.md).

use std::error::Error;
use std::path::{Path, PathBuf};

use sha2::Digest;

/// Each corpus: the `docs/VERSIONS.md` row that pins it, and its directory.
const CORPORA: &[(&str, &str)] = &[
    ("FHIRconnect specification source", "docs/specs/fhirconnect"),
    (
        "FHIRconnect REST API chapter (draft, unmerged)",
        "docs/specs/fhirconnect/draft-rest-api",
    ),
    (
        "FHIRconnect mapping library (corpus, never an oracle)",
        "docs/specs/fhirconnect-mapping-lib",
    ),
    ("OMOCL corpus", "docs/specs/omocl"),
    (
        "OMOP CDM definitions and PostgreSQL DDL",
        "docs/specs/omop-cdm",
    ),
    ("openEHR ITS-REST OpenAPI", "docs/specs/its-rest"),
    (
        "KDS Diagnose operational template (fixture)",
        "tools/ferrobridge-testkit/fixtures/opt/kds",
    ),
];

/// The repository root, reached from this crate's manifest directory.
fn repo_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Returns the whole `Pin` cell of the matrix row whose `Item` cell is `item`,
/// with backticks removed.
///
/// The corpus rows carry a repository and a commit or tag in one cell, which is
/// more than [`ferrobridge_testkit::matrix_pin`] returns, so the row is parsed
/// here the same way.
fn matrix_pin_cell(item: &str) -> Result<String, Box<dyn Error>> {
    let text = std::fs::read_to_string(repo_root().join("docs/VERSIONS.md"))?;
    for line in text.lines() {
        let mut cells = line
            .split('|')
            .skip(1)
            .map(|cell| cell.replace('`', "").trim().to_owned());
        let (Some(key), Some(pin)) = (cells.next(), cells.next()) else {
            continue;
        };
        if key == item {
            return Ok(pin);
        }
    }
    Err(format!("docs/VERSIONS.md has no row for {item}").into())
}

/// Returns the commit or tag a pin cell names: its first 40-hex token, or the
/// token after the word `tag`.
fn pinned_ref(cell: &str) -> Result<String, Box<dyn Error>> {
    let tokens: Vec<&str> = cell.split_whitespace().collect();
    if let Some(commit) = tokens
        .iter()
        .find(|t| t.len() == 40 && t.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return Ok((*commit).to_owned());
    }
    for (index, token) in tokens.iter().enumerate() {
        if *token == "tag"
            && let Some(tag) = tokens.get(index + 1)
        {
            return Ok(tag.trim_end_matches(',').to_owned());
        }
    }
    Err(format!("the pin '{cell}' names no commit and no tag").into())
}

/// Returns every file under `dir`, sorted, with directories walked.
fn files_under(dir: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in std::fs::read_dir(&current)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                found.push(entry.path());
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Counts the files whose extension is one of `extensions`.
fn count_with_extension(files: &[PathBuf], extensions: &[&str]) -> usize {
    files
        .iter()
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| extensions.contains(&ext))
        })
        .count()
}

/// Returns the lowercase hexadecimal sha256 of a file's bytes.
fn sha256_of(path: &Path) -> Result<String, Box<dyn Error>> {
    use std::fmt::Write;

    let mut hasher = sha2::Sha256::new();
    hasher.update(std::fs::read(path)?);
    let mut hex = String::new();
    for byte in hasher.finalize() {
        write!(hex, "{byte:02x}")?;
    }
    Ok(hex)
}

/// Returns the sha256 `docs/architecture.md` records for a FHIRconnect schema.
fn recorded_sha256(architecture: &str, file: &str) -> Result<String, Box<dyn Error>> {
    let needle = format!("{file}` sha256 `");
    let start = architecture
        .find(&needle)
        .ok_or_else(|| format!("docs/architecture.md records no sha256 for {file}"))?
        + needle.len();
    let hex: String = architecture
        .get(start..)
        .ok_or_else(|| format!("the sha256 for {file} is truncated"))?
        .chars()
        .take(64)
        .collect();
    if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(hex)
    } else {
        Err(
            format!("the sha256 docs/architecture.md records for {file} is not 64 hex digits")
                .into(),
        )
    }
}

#[test]
fn every_corpus_provenance_names_the_pin_the_matrix_names() -> Result<(), Box<dyn Error>> {
    let root = repo_root();
    for (item, dir) in CORPORA {
        let cell = matrix_pin_cell(item)?;
        let pinned = pinned_ref(&cell)?;
        let repository = ferrobridge_testkit::matrix_pin(item)?;
        let provenance = std::fs::read_to_string(root.join(dir).join("PROVENANCE.md"))?;
        assert!(
            provenance.contains(&pinned),
            "{dir}/PROVENANCE.md does not name the pin {pinned} that docs/VERSIONS.md records for '{item}'"
        );
        assert!(
            provenance.contains(&repository),
            "{dir}/PROVENANCE.md does not name the upstream repository {repository}"
        );
        assert!(
            provenance.contains("Fetched:"),
            "{dir}/PROVENANCE.md records no fetch date"
        );
        assert!(
            provenance.contains("Upstream licence:"),
            "{dir}/PROVENANCE.md records no upstream licence"
        );
    }
    Ok(())
}

#[test]
fn the_kds_template_hashes_to_the_pinned_sha256() -> Result<(), Box<dyn Error>> {
    let cell = matrix_pin_cell("KDS Diagnose operational template (fixture)")?;
    let tokens: Vec<&str> = cell.split_whitespace().collect();
    let pinned = tokens
        .windows(2)
        .find_map(|pair| match pair {
            ["sha256", digest] => Some(*digest),
            _ => None,
        })
        .ok_or("the KDS template pin names no sha256")?;
    let file = repo_root().join("tools/ferrobridge-testkit/fixtures/opt/kds/KDS_Diagnose.opt");
    assert_eq!(
        sha256_of(&file)?,
        pinned,
        "the vendored KDS_Diagnose.opt no longer hashes to the value docs/VERSIONS.md pins"
    );
    Ok(())
}

#[test]
fn no_corpus_directory_is_empty() -> Result<(), Box<dyn Error>> {
    let root = repo_root();
    for (_, dir) in CORPORA {
        let files = files_under(&root.join(dir))?;
        assert!(files.len() > 1, "{dir} holds only {} file(s)", files.len());
    }
    Ok(())
}

#[test]
fn the_vendored_fhirconnect_schemas_hash_to_the_recorded_values() -> Result<(), Box<dyn Error>> {
    let root = repo_root();
    let architecture = std::fs::read_to_string(root.join("docs/architecture.md"))?;
    let published = root.join("docs/specs/fhirconnect/build/site/FHIRconnect/v1.0.0/_attachments");
    let source = root.join("docs/specs/fhirconnect/modules/ROOT/attachments");
    for schema in [
        "model-mapping.schema.json",
        "contextual-mapping.schema.json",
    ] {
        let want = recorded_sha256(&architecture, schema)?;
        assert_eq!(
            sha256_of(&published.join(schema))?,
            want,
            "the published v1.0.0 {schema} no longer hashes to the value docs/architecture.md records"
        );
        assert!(
            source.join(schema).is_file(),
            "the repository-source copy of {schema} is missing from the corpus"
        );
    }
    Ok(())
}

#[test]
fn the_mapping_library_carries_its_full_yaml_corpus() -> Result<(), Box<dyn Error>> {
    let files = files_under(&repo_root().join("docs/specs/fhirconnect-mapping-lib"))?;
    assert_eq!(
        count_with_extension(&files, &["yml", "yaml"]),
        107,
        "the FHIRconnect mapping library is pinned at 107 YAML mapping files"
    );
    Ok(())
}

#[test]
fn the_omocl_corpus_carries_its_full_yaml_corpus() -> Result<(), Box<dyn Error>> {
    let files = files_under(&repo_root().join("docs/specs/omocl"))?;
    assert_eq!(
        count_with_extension(&files, &["yml", "yaml"]),
        202,
        "the OMOCL corpus is pinned at 202 mapping files"
    );
    Ok(())
}

#[test]
fn the_cdm_corpus_carries_the_four_postgresql_ddl_files() -> Result<(), Box<dyn Error>> {
    let ddl = repo_root().join("docs/specs/omop-cdm/inst/ddl/5.4/postgresql");
    let files = files_under(&ddl)?;
    assert_eq!(
        count_with_extension(&files, &["sql"]),
        4,
        "OHDSI renders four PostgreSQL files for 5.4: the DDL, the primary keys, the indices and the constraints"
    );
    for csv in [
        "OMOP_CDMv5.4_Field_Level.csv",
        "OMOP_CDMv5.4_Table_Level.csv",
    ] {
        let path = repo_root().join("docs/specs/omop-cdm/inst/csv").join(csv);
        assert!(path.is_file(), "the CDM definitions are missing {csv}");
    }
    Ok(())
}

#[test]
fn the_its_rest_corpus_carries_the_three_stable_documents() -> Result<(), Box<dyn Error>> {
    let oas = repo_root().join("docs/specs/its-rest/computable/OAS");
    let files = files_under(&oas)?;
    assert_eq!(
        count_with_extension(&files, &["yaml"]),
        3,
        "the bridge depends on the EHR, Query and Definition documents, and on no other"
    );
    for module in ["ehr", "query", "definition"] {
        let path = oas.join(format!("{module}-codegen.openapi.yaml"));
        let text = std::fs::read_to_string(&path)?;
        assert!(
            text.contains("x-status: STABLE"),
            "{module}-codegen.openapi.yaml is not the STABLE document"
        );
    }
    Ok(())
}
