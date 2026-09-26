// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The message sets of the two corpora, read from `vendor/` in path order.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// The vendored and fetched sets, relative to this crate's manifest.
pub(super) const VENDOR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor");

/// The expected Bundles of the Microsoft set, under its vendored root.
const CONVERTER_EXPECTED: &str =
    "src/Microsoft.Health.Fhir.Liquid.Converter.FunctionalTests/TestData/Expected/Hl7v2";

/// The `ReportStream` data tests, under that set's vendored root.
const REPORTSTREAM_TESTS: &str = "prime-router/src/testIntegration/resources/datatests";

/// The `ReportStream` directories whose `.fhir` file is the conversion of the
/// `.hl7` message beside it; in `FHIR_to_HL7/` the direction is the reverse.
const REPORTSTREAM_CONVERSIONS: &[&str] = &["HL7_to_FHIR", "mappinginventory"];

/// How many messages of each AIRA example file the smoke corpus reads.
///
/// No specification governs this: our own design; the files hold 13,583
/// generated messages, and the first of each file keeps the case list short
/// and the same on every run.
const AIRA_PER_FILE: usize = 25;

/// One message of a corpus, with the Bundle its set expects for it.
pub(super) struct Message {
    /// The case id: the set, then the path under it, spaces as `%20`.
    pub(super) id: String,
    /// The bytes as the file holds them.
    pub(super) bytes: Vec<u8>,
    /// The file holding the expected Bundle, when the set carries one.
    pub(super) expected: Option<PathBuf>,
}

/// The case id of `relative` under `set`.
fn case_id(set: &str, relative: &Path) -> String {
    let path = relative.to_string_lossy().replace('\\', "/");
    format!("{set}/{path}").replace(' ', "%20")
}

/// Every file under `directory`, relative to `root`, in path order.
fn files(root: &Path, directory: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut found = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path.strip_prefix(root)?.to_path_buf());
            }
        }
    }
    found.sort();
    Ok(found)
}

/// The Microsoft FHIR-Converter samples, each with the expected Bundle its
/// file name names.
pub(super) fn converter() -> Result<Vec<Message>, Box<dyn Error>> {
    let root = Path::new(VENDOR).join("fhir-converter");
    let mut expected = BTreeMap::new();
    for path in files(&root, &root.join(CONVERTER_EXPECTED))? {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        if let Some(stem) = name
            .as_deref()
            .and_then(|name| name.strip_suffix("-expected.json"))
        {
            expected.insert(stem.to_owned(), root.join(&path));
        }
    }
    let mut messages = Vec::new();
    for path in files(&root, &root.join("data/SampleData/Hl7v2"))? {
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned());
        messages.push(Message {
            id: case_id("fhir-converter", &path),
            bytes: fs::read(root.join(&path))?,
            expected: stem.and_then(|stem| expected.get(&stem).cloned()),
        });
    }
    Ok(messages)
}

/// The `ReportStream` `.hl7` files, each with the `.fhir` conversion beside it
/// in the directories that hold one.
pub(super) fn reportstream() -> Result<Vec<Message>, Box<dyn Error>> {
    let root = Path::new(VENDOR).join("reportstream");
    let tests = root.join(REPORTSTREAM_TESTS);
    let mut messages = Vec::new();
    for path in files(&root, &tests)? {
        if path.extension().is_none_or(|extension| extension != "hl7") {
            continue;
        }
        let converts = root
            .join(&path)
            .strip_prefix(&tests)?
            .components()
            .next()
            .is_some_and(|first| {
                REPORTSTREAM_CONVERSIONS
                    .iter()
                    .any(|directory| first.as_os_str() == *directory)
            });
        let bundle = root.join(path.with_extension("fhir"));
        messages.push(Message {
            id: case_id("reportstream", &path),
            bytes: fs::read(root.join(&path))?,
            expected: (converts && bundle.is_file()).then_some(bundle),
        });
    }
    Ok(messages)
}

/// The seven benchmark messages derived from the v2-to-FHIR page and the
/// sample message beside its Bundle, each with the HL7-authored Bundle whose
/// file name names its structure.
pub(super) fn v2_to_fhir() -> Result<Vec<Message>, Box<dyn Error>> {
    let root = Path::new(VENDOR).join("v2-to-fhir");
    let bundles = root.join("samples/fhir-bundles");
    // NOTE: `samples/fhir-bundles/FHIR_bundle.hl7_MDM_T02.json` holds XML at the
    // pin, so the MDM_T02 Bundle is read from its `.xml` twin.
    let expected_for = |structure: &str| match structure {
        "ADT_A01" => Some(bundles.join("FHIR_bundle.hl7_ADT_A01.json")),
        "MDM_T02" => Some(bundles.join("FHIR_bundle.hl7_MDM_T02.xml")),
        _ => None,
    };
    let mut messages = Vec::new();
    for path in files(&root, &root.join("derived"))? {
        let structure = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned());
        messages.push(Message {
            id: case_id("v2-to-fhir", &path),
            bytes: fs::read(root.join(&path))?,
            expected: structure.as_deref().and_then(expected_for),
        });
    }
    let sample = Path::new("samples/messages/Message.hl7_MDM_T02.txt");
    messages.push(Message {
        id: case_id("v2-to-fhir", sample),
        bytes: fs::read(root.join(sample))?,
        expected: expected_for("MDM_T02"),
    });
    Ok(messages)
}

/// The NIST test steps: every `Message.txt` of the three fetched bundles.
pub(super) fn nist(root: &Path) -> Result<Vec<Message>, Box<dyn Error>> {
    let mut messages = Vec::new();
    for path in files(root, root)? {
        if path.file_name().is_some_and(|name| name == "Message.txt") {
            messages.push(Message {
                id: case_id("nist", &path),
                bytes: fs::read(root.join(&path))?,
                expected: None,
            });
        }
    }
    Ok(messages)
}

/// The first [`AIRA_PER_FILE`] messages of each fetched AIRA example file,
/// split where a segment opens with `MSH`.
pub(super) fn aira(root: &Path) -> Result<Vec<Message>, Box<dyn Error>> {
    let mut messages = Vec::new();
    for path in files(root, root)? {
        let bytes = fs::read(root.join(&path))?;
        let normal = normalised(&bytes);
        let mut starts: Vec<usize> = Vec::new();
        for (index, _) in normal.iter().enumerate() {
            let opens = index == 0 || normal.get(index.wrapping_sub(1)) == Some(&b'\r');
            if opens && normal.get(index..index.saturating_add(4)) == Some(b"MSH|".as_slice()) {
                starts.push(index);
            }
        }
        for (number, window) in starts.iter().enumerate().take(AIRA_PER_FILE) {
            let end = starts
                .get(number.saturating_add(1))
                .copied()
                .unwrap_or(normal.len());
            messages.push(Message {
                id: format!(
                    "{}#{}",
                    case_id("aira-mqe", &path),
                    number.saturating_add(1)
                ),
                bytes: normal.get(*window..end).unwrap_or_default().to_vec(),
                expected: None,
            });
        }
    }
    Ok(messages)
}

/// The message as a frame carries it: a leading UTF-8 byte order mark
/// removed, every line ending a carriage return, and no empty segment.
///
/// No specification governs this: our own design; the byte order mark and
/// the line endings are how a file stores the text, and HL7 v2.5.1 chapter 2
/// §2.5.4 ends every segment with a carriage return.
pub(super) fn normalised(bytes: &[u8]) -> Vec<u8> {
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let mut out = Vec::with_capacity(text.len());
    for line in text.split(|byte| *byte == b'\r' || *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        out.extend_from_slice(line);
        out.push(b'\r');
    }
    out
}

#[test]
fn normalising_removes_the_byte_order_mark_and_ends_every_segment_with_a_carriage_return() {
    let stored = b"\xEF\xBB\xBFMSH|^~\\&|A\r\nPID|1\n\nOBX|1\r\n";
    assert_eq!(normalised(stored), b"MSH|^~\\&|A\rPID|1\rOBX|1\r".to_vec());
}
