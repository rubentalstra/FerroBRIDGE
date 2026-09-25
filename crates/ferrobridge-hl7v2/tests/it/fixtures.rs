// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Synthetic HL7 v2 messages, invented for these tests. No value is taken
//! from a real patient, a real system or a production extract.

/// Joins segments with the carriage return that terminates each (HL7 v2.5.1
/// chapter 2 §2.5.4).
pub(crate) fn message(segments: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for segment in segments {
        bytes.extend_from_slice(segment);
        bytes.push(b'\r');
    }
    bytes
}

/// A laboratory result, ORU^R01 from a 2.5.1 sender, in ISO 8859-1 (MSH-18
/// `8859/1`): a patient whose name and street carry Latin-1 letters, a visit,
/// one order with its request and two numeric results, and a local
/// Z-segment.
pub(crate) fn oru_r01() -> Vec<u8> {
    message(&[
        b"MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00001|P|2.5.1||||||8859/1",
        b"PID|1||PAT-0001^^^NORTHLAB^MR||M\xFCller^J\xF6rg^^^^^L||19800101|M|||Hauptstra\xDFe 1^^Musterstadt^^12345^DE",
        b"PV1|1|O",
        b"ORC|RE|PLC-1|FIL-1",
        b"OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN|||20260925120000+0200",
        b"OBX|1|NM|2345-7^Glucose^LN||5.4|mmol/L^mmol/L^UCUM|3.9-5.8|N|||F|||20260925130000+0200",
        b"OBX|2|NM|2951-2^Sodium^LN||140|mmol/L^mmol/L^UCUM|135-145|N|||F|||20260925130000+0200",
        b"ZXX|synthetic local segment",
    ])
}

/// An admission, ADT^A01, in UTF-8 (MSH-18 `UNICODE UTF-8`).
pub(crate) fn adt_a01() -> Vec<u8> {
    message(&[
        "MSH|^~\\&|ADMIT|NORTHHOSP|EHR|SOUTHCLINIC|20260925090000+0200||ADT^A01^ADT_A01|MSG00002|P|2.5.1||||||UNICODE UTF-8".as_bytes(),
        "EVN||20260925085900+0200".as_bytes(),
        "PID|1||PAT-0002^^^NORTHHOSP^MR||Zo\u{EB}^Anna^^^^^L||19920304|F".as_bytes(),
        "PV1|1|I|WARD1^101^A".as_bytes(),
    ])
}

/// A document notification, MDM^T02, with an empty MSH-18, read in the
/// connection's agreed character set.
pub(crate) fn mdm_t02() -> Vec<u8> {
    message(&[
        b"MSH|^~\\&|DOCS|NORTHHOSP|EHR|SOUTHCLINIC|20260925100000+0200||MDM^T02^MDM_T02|MSG00003|P|2.5.1",
        b"EVN||20260925095900+0200",
        b"PID|1||PAT-0003^^^NORTHHOSP^MR||Doe^Sam^^^^^L||19751111|U",
        b"PV1|1|O",
        b"TXA|1|CN|TX|20260925095000+0200||||||||DOC-0001|||||AU",
        b"OBX|1|TX|11506-3^Progress note^LN||Synthetic note text||||||F",
    ])
}

/// The ORU^R01 of [`oru_r01`] without PID-5, which the definitions mark
/// required.
pub(crate) fn oru_r01_without_patient_name() -> Vec<u8> {
    message(&[
        b"MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00004|P|2.5.1",
        b"PID|1||PAT-0001^^^NORTHLAB^MR||||19800101|M",
        b"ORC|RE|PLC-1|FIL-1",
        b"OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN",
        b"OBX|1|NM|2345-7^Glucose^LN||5.4|mmol/L^mmol/L^UCUM|||||F",
    ])
}

/// A result whose sending and receiving applications carry universal IDs:
/// an ISO OID in MSH-3 and a UUID in MSH-5.
pub(crate) fn oru_r01_universal_applications() -> Vec<u8> {
    message(&[
        b"MSH|^~\\&|LAB^1.2.3.4.5^ISO|NORTHLAB|EHR^2b3c4d5e-0000-4000-8000-000000000001^UUID|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00006|P|2.5.1",
        b"PID|1||PAT-0006^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
        b"ORC|RE|PLC-1|FIL-1",
        b"OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN",
        b"OBX|1|NM|2345-7^Glucose^LN||5.4|mmol/L^mmol/L^UCUM|||||F",
    ])
}

/// A result that names its sender in none of MSH-3, MSH-24 and MSH-4.
pub(crate) fn oru_r01_without_sender() -> Vec<u8> {
    message(&[
        b"MSH|^~\\&|||EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00007|P|2.5.1",
        b"PID|1||PAT-0007^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
        b"ORC|RE|PLC-1|FIL-1",
        b"OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN",
        b"OBX|1|NM|2345-7^Glucose^LN||5.4|mmol/L^mmol/L^UCUM|||||F",
    ])
}

/// A result that names its sender and receiver only by facility, in MSH-4
/// and MSH-6.
pub(crate) fn oru_r01_facilities_only() -> Vec<u8> {
    message(&[
        b"MSH|^~\\&||North Lab^1.2.3.4.5^ISO||SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00008|P|2.5.1",
        b"PID|1||PAT-0008^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
        b"ORC|RE|PLC-1|FIL-1",
        b"OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN",
        b"OBX|1|NM|2345-7^Glucose^LN||5.4|mmol/L^mmol/L^UCUM|||||F",
    ])
}

/// A result from a sending and to a receiving application named with spaces
/// in MSH-3 and MSH-5, which the guide writes into FHIR `url` elements.
pub(crate) fn oru_r01_named_applications() -> Vec<u8> {
    message(&[
        b"MSH|^~\\&|North Lab App|NORTHLAB|South EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00005|P|2.5.1",
        b"PID|1||PAT-0005^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
        b"ORC|RE|PLC-1|FIL-1",
        b"OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN",
        b"OBX|1|NM|2345-7^Glucose^LN||5.4|mmol/L^mmol/L^UCUM|||||F",
    ])
}
