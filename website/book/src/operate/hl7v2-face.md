<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# The HL7 v2 face

The HL7 v2 face is a second listener in the `ferrobridge serve` process. It
accepts HL7 v2 messages over MLLP, turns each one into an R4 message Bundle
with the ConceptMaps of the HL7 v2-to-FHIR implementation guide, and writes the
Bundle's resources into the CDR through the same ingest service the FHIR
facade's transaction uses. It answers each message with an original-mode
acknowledgment once the CDR has committed or refused it.

No specification governs the hand-off from a message Bundle to the CDR. It is
FerroBRIDGE's own design, and this page states it.

<!-- toc -->

## Turning it on

The face is off until `[hl7v2] enabled` is true, and it needs the FHIR facade:
the programs, the identity map and the ingest service it writes through are the
facade's. `[hl7v2] enabled` with the facade off is a boot error. So is a
`concept_maps` directory that does not load and a port that cannot be bound.
The guide's table maps are translated on the `[terminology]` server, so load
the guide's table ConceptMaps into it. Without a `[terminology]` section every
table value is a counted `no-terminology` outcome.

```toml
[facade]
enabled = true
ehr_policy = "existing"
composition_language = "en"
composition_territory = "GB"

[hl7v2]
enabled = true
listen = "0.0.0.0:2575"
concept_maps = "/srv/hl7.fhir.uv.v2mappings/package"
ehr_policy = "create_on_first_write"
profiles = [
  { resource_type = "Observation", profile = "https://example.org/fhir/StructureDefinition/lab-result" },
]
```

## The keys

Every key is settable as `FERROBRIDGE__HL7V2__<KEY>`, and a list takes TOML
syntax in the variable. The section holds no secret.

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Whether the MLLP listener runs |
| `listen` | `127.0.0.1:2575` | The socket address the listener binds |
| `default_charset` | `ASCII` | The HL7 table 0211 code a message with an empty MSH-18 is read in: `ASCII`, `UNICODE UTF-8`, `8859/1` to `8859/9` or `8859/15` |
| `concept_maps` | none, required | The directory of the guide's ConceptMaps: the `package` directory of `hl7.fhir.uv.v2mappings` |
| `supplements` | `[]` | Directories of your own ConceptMaps, loaded in order after the guide and the supplements the face ships; a map with the id or the url of a loaded one replaces it |
| `unmapped_entries` | `skip_and_count` | What an entry no program maps does: `skip_and_count` commits the others, `refuse` refuses the message |
| `ehr_policy` | the facade's | `existing` or `create_on_first_write`, for the face alone |
| `profiles` | `[]` | A list of `{ resource_type, profile }`: the profile a resource of that type claims in `meta.profile` when the guide's maps wrote none |
| `idle_timeout_ms` | `300000` | How long a connection may sit between messages before it is closed; `0` keeps it open |
| `frame_timeout_ms` | `30000` | How long one message may take from its first byte to its trailer before it is answered `AR`; `0` waits |
| `frame_limit_bytes` | `1048576` | The largest message one frame may carry |
| `log_outcomes` | `false` | Whether an `AE` or `AR` also logs the counted outcomes of the run, by kind, at debug level |
| `senders` | `[]` | The sending facilities (MSH-4) the face accepts, each `{ namespace_id = "..." }` (HD.1) or `{ universal_id = "...", universal_id_type = "..." }` (HD.2 and HD.3); empty accepts any |

## What happens to one message

1. The listener reads one MLLP Release 1 frame. A frame that is malformed,
   larger than `frame_limit_bytes`, or not complete within `frame_timeout_ms`
   is answered `AR` when a header can be read from it, and the connection
   closes.
2. The message is decoded in the character set its MSH-18 names, split by
   position and grouped by its message structure. With `senders` set, a
   message whose MSH-4 names none of them is answered `AR` here and is never
   mapped.
3. The guide's message, segment and data type maps, with the supplements over
   them, write an R4 message Bundle. Every row the run cannot carry is a
   counted outcome, and every supplement map the run used is counted once as
   `supplemented`.
4. Every entry whose resource type is listed under `profiles` and that claims
   no profile gets that profile in `meta.profile`, and every entry with a
   `subject` or `patient` reference and none set references the message's one
   `Patient`. The guide leaves the relationships between the resources of one
   message to the implementer, and the facade selects a program by
   `meta.profile`, so these two steps are what let a message reach a mapping.
5. The Bundle's entries go through the ingest service as one transaction. The
   programs are selected as the facade selects them, and the subject is read
   from the `Patient` entry's first identifier. Each composition's
   `FEEDER_AUDIT` names the message: its MSH-10 as the item id and its message
   type (`ORU^R01`) as the item type.
6. The acknowledgment is sent once the ingest settled, so `AA` means the CDR
   holds the message's compositions.

## Supplements to the guide

The guide invites an implementation to add a mapping it needs locally
(`mapping_guidelines.md`, General Format/Approach). FerroBRIDGE does that with
supplements: ConceptMaps in the guide's own shape that override a map of the
guide or add one it lacks. No specification governs how they load or how they
are marked; that is FerroBRIDGE's own design, and it works as follows.

- The face ships its supplements inside `ferrobridge-hl7v2` (the crate's
  `supplements/` directory, compiled in) and loads them over the guide's
  package at boot. The `supplements` directories of `[hl7v2]` load after
  them, so a map of your own can replace one of FerroBRIDGE's.
- A supplement with the id or the canonical url of a loaded map replaces that
  map whole. Any other supplement is added.
- Each shipped supplement names FerroBRIDGE in its `title`. Its `description`
  names the guide map it overrides, or the structure it adds, the rows it
  changes and why, and the tracker issue that records the defect. An override
  keeps the guide map's id and url. An added map takes an id in the guide's
  naming form and a url under `https://ferrobridge.eu/fhir/v2mappings/`, so it
  never claims a canonical url of the guide.
- The run counts each supplement map it uses once per message, as
  `supplemented` naming the map, so an outcome list shows which values a
  supplement wrote.

The shipped supplements:

| Map | Kind | What it changes, and why |
|---|---|---|
| `datatype-cwe-to-codeableconcept` | override | Runs the CWE.1 row into `coding[1].code` with no condition. The guide gates it on a Narrative-Condition no machine evaluates, so no Coding it writes carries a code (#332) |
| `datatype-ce-to-codeableconcept` | override | The same row for CE, the type HL7 v2.3 and earlier give most coded fields (#332) |
| `datatype-cf-to-codeableconcept` | override | Names the sources CF.1 to CF.13, where the guide names them CWE.1 to CWE.13 so no CF row runs, and runs CF.1 with no condition (#332) |
| `datatype-cwe-to-quantity` | override | Names the targets `code`, `unit` and `system`, where the guide writes `Quantity.code` and so on, which the element table cannot find under the Quantity the row fills; OBX-6 units are kept (#332) |
| `datatype-hd-name-to-messageheader-source` | override | Drops the row that writes HD.2, a universal ID, into `MessageHeader.source.software`, which R4 defines as the software's name (#324) |
| `datatype-hd-name-to-messageheader-destination` | override | Writes HD.1 into `destination.name`, as the endpoint map does, where the guide writes HD.2 and the two maps disagree (#332) |
| `datatype-pl-to-location` | override | Links the six Locations of a PL (bed, room, point of care, floor, building, facility) into one `partOf` chain in the order the guide's PL.10 rows give the finest level, where the guide's `partOf` rows disagree with it and the building's names itself; writes the point of care's `mode` and `physicalType` (`wa`, the code the guide repository's MDM^T02 sample gives PL.1) at elements Location has; names the point of care and the building in the PL.10 rows as their second set does; and writes PL.9 into the finest valued level (#342, #332) |
| `segment-msh-to-messageheader` | override | With both MSH-3 and MSH-24 valued, MSH-3 names the source and MSH-24 gives its endpoint; with one of them empty, the other runs as the guide writes it (#311) |
| `segment-orc-to-diagnosticreport` | override | Writes ORC-2 into `basedOn.identifier` (R4 `Reference.identifier`), where the guide writes `basedOn(ServiceRequest)` with no map to fill it (#336) |
| `segment-sch-to-appointment` | override | Writes SCH-26 and SCH-27 into `basedOn[1].identifier` and `basedOn[2].identifier` for the same reason (#336) |
| `segment-pid-to-appointment` | override | Writes PID-2, PID-3 and PID-4 into the identifier of the Appointment's patient references, so the message keeps one Patient where the guide's `(Patient)` rows would create one per row (#336) |
| `message-adt-a05-to-bundle`, `message-adt-a09-to-bundle` | override | Names the PD1 row `ADT_A05.PD1` and `ADT_A09.PD1`, where the guide names it `ADT_A01.PD1`, which leaves ADT_A05 and ADT_A09 with no message map (#332) |
| `message-adt-a03-to-bundle` | added | ADT_A03 (A03 discharge), from the rows of the guide's ADT_A01 map at the same paths (#256) |
| `message-bar-p01-to-bundle` | added | BAR_P01 (P01 add patient accounts), from the ADT_A01 rows for the segments the two share, the visit segments under `VISIT`; GT1, UB1, UB2, ACC and DRG have no segment map in the guide and are counted unmapped (#256) |
| `message-orl-o22-to-bundle` | added | ORL_O22 (the laboratory order response), from the guide's OML_O21 rows under `RESPONSE`, with the MSA row into `MessageHeader.response` (#256) |
| `message-oul-r22-to-bundle` | added | OUL_R22 (the specimen-oriented observation), from the guide's ORU_R01 rows at the OUL_R22 paths; the OBR row into a Specimen is left out, since SPM carries the specimen (#256) |

What the supplements leave to the guide, and what stays open:

- CWE.3 is written into `Coding.system` as the sender sends it (`LN`). The
  guide's comment on that row says a vocabulary table gives the URI, and the
  guide ships no table map for HL7 table 0396, so the value stays the v2
  mnemonic until one exists.
- DFT_P03 has no message map. Its defining segment, FT1, has no segment map in
  the guide, so a map for the rest would acknowledge a charge message without
  its charges.
- The `PL` to `Location` map's PL.11 rows write `[1-6].identifier[n].assigner`,
  one value spread over six Locations, which the interpreter refuses as it
  refuses every spreading label, so a location's assigning authority is
  counted and not written.

## Sibling resources of one value

The `PL` to `Location` map writes one Location per level of a patient
location through rows prefixed `[1].` to `[6].`, and links them with
`partOf.reference(Location[k])` rows. The guide's notation says the prefix
applies where the data type is used (`mapping_guidelines.md`, \[n\] Notation),
which for a map into a resource is the resource, and leaves the rest to the
implementer. FerroBRIDGE's own design, for any data type map whose rows
reference a labelled instance of the resource type they fill:

- A `[k].` row writes into the `k`-th sibling of the resource the `(Type)` row
  created, and the sibling exists once a value reaches it. `[1].` is that
  resource itself. A sibling no value reaches never enters the Bundle.
- A `(Location[k])` reference between siblings points at sibling `k` when it
  holds a value. When it holds none, the reference climbs to the sibling that
  `k`'s own row names, and so on up the chain, so a PL without a floor has its
  point of care in its building, or in its facility. A reference that climbs
  past the last valued level, loops, or names its own sibling is counted as
  `sibling-unresolved` and not written.
- Every reference to the family (`Encounter.location.location`, and any other
  reference to the Location the row created) takes the finest valued level:
  the sibling no other valued sibling is part of. For a PL that is the bed,
  else the room, else the point of care. It is counted once as
  `finest-sibling` naming the level. When the chain leaves more than one such
  sibling, the references stay at the Location the guide's `(Type)` row
  names, or are dropped when that Location holds no value, counted as
  `sibling-ambiguous`.

## The acknowledgment

The codes are HL7 table 0008 in original mode (HL7 v2.5.1 chapter 2
§2.9.2.2). MSA-2 echoes MSH-10, and each `ERR` carries its table 0357 code in
ERR-3 and the text in ERR-8.

| Outcome | Code | `ERR` |
|---|---|---|
| The ingest committed the message | `AA` | none |
| The same message was committed before (same MSH-10 and message type) | `AA` | none, and nothing is committed again |
| No header can be read at all | none | the connection closes unanswered |
| Delimiters that do not declare themselves, a byte outside the declared character set, a version outside 2.x | `AR` | `207` or `203` |
| A message structure the definitions or the guide do not carry | `AR` | `200` |
| A required field or segment missing, an undecoded escape | `AE` | `101`, `100` or `102`, located |
| MSH-4 names a sending facility `senders` does not list | `AR` | `207` at MSH^1^4 |
| MSH-10 empty | `AE` | `101` at MSH-10 |
| A condition of the guide that stops the mapper, a Bundle with no `MessageHeader` | `AE` | `101` or `207` |
| No program maps any entry, an entry that does not map, a second subject, a conflict with the identity map | `AE` | `207`, one per refused entry, naming its `fullUrl` |
| The terminology server refused or did not answer a translation | `AR` | `207` |
| The CDR failed, was unreachable, timed out, or refused the bridge's credentials (`5xx`, `401`, `403`, `407`, `408`, `429`) | `AR` | `207` |

An `AR` asks the sender to resend later; an `AE` says the message itself cannot
be taken as it is. A message that arrives while another delivery of the same
message is still in flight is `AE` naming the entries, and commits nothing.
Resend it after the first delivery answered. The ERR-8 text is printable ASCII,
so it encodes in every character set a message can declare.

## What is logged

Each connection runs in an `mllp_connection` span naming the peer, with the
count of messages it answered (`messages`) and of messages refused for their
sender (`refused`), and each
message in an `hl7v2_message` span naming its MSH-10 and message type. One
event per message names the acknowledgment code and how many entries were
committed and skipped. With `log_outcomes` an `AE` or `AR` also logs, at debug
level, each outcome kind of the run with its count. No byte of a message, no
FHIR resource and no CDR error body reaches a log line; the `ERR` segments
travel only to the sender.

The boot banner has an `hl7v2` line naming the address the listener binds and
the CDR and terminology hosts it reaches, and `/health/readiness` carries an
`hl7v2-listener` indicator that is down from the stop signal on.
`/health/info` lists every lane under `lanes`; the `hl7v2` lane carries its
`listen` address and `up`, whether the listener accepts connections now. The
facade's `CapabilityStatement` adds one sentence to `implementation.description`
naming the HL7 v2 face as another inbound path under the same identity and
replay rules, only when the face is configured. On `SIGTERM`
the listener stops accepting, every message in flight is mapped, committed and
acknowledged, and the connection closes, within the same
`[server] shutdown_timeout_ms` as the HTTP drain.

## Limits

- The face runs in one process. The claims that hold a message in flight and
  the identity map that recognises a resent one live in that process, so run
  one process per identity store and point every sender of a feed at it.
- A batch envelope (FHS, BHS) is read only as far as the parser accepts it.
  The face answers one acknowledgment per frame and does not split a batch.
- MLLP Release 2's commit acknowledgment is not spoken, and enhanced-mode
  acknowledgments are not sent.
- The allow list matches MSH-4 alone; the sending application (MSH-3) is not
  checked.

## Until a mapping context covers the guide's resources

No published FHIRconnect context in the tree maps the resources the guide and
its supplements write for the message families (ADT, BAR, ORU, OUL, ORM, OML
and ORL, MDM, SIU, VXU):
the published contexts do not compile in-tree for lack of their templates
(issue #258). Until one does, a message's Bundle reaches the ingest service and
is refused for lack of a program: the sender gets `AE` with one `ERR` per
entry, and nothing is committed. To take messages before then, write a context
for the resources you need, claim its profile under `profiles`, and load it
with the facade's mapping set.
