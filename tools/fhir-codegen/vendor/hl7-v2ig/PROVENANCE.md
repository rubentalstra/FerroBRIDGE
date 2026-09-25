<!-- This file describes third-party material fetched at build time; the bytes
     it describes keep their upstream terms and are never committed. -->

# Provenance: the HL7 v2 definitions (HL7/v2ig source of truth)

Fetched verbatim at build time by `scripts/vendor/v2ig.sh` into this
directory, which `.gitignore` refuses except for this file. Never commit a
file from here and never edit one: change the pin in docs/VERSIONS.md, run
`scripts/vendor/v2ig.sh --stamp`, and record the count and digest it prints.

- Source: <https://github.com/HL7/v2ig>
- Pin: commit `3adcdbfff654ccbff5cd33aa34bd909a087e8e19`, path `input/sourceOfTruth`, taken whole at its upstream layout
- Files: 1694
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `9a1fb2b974b69f575bcd9b30fa66d2ff7f13ef7869c1350eca7462067e974f93`
- Stamped: 2026-09-25. The script verifies the count and the digest on every
  fetch, and keeps a tree already on disk at that digest.
- Content: FHIR StructureDefinitions of kind `logical` (with a few
  CodeSystem, ValueSet and List resources) for the v2 messages, message
  structures, events, segments and data types, declaring `fhirVersion`
  5.0.0. They hold one v2 version: the repository's
  `V291_EXTRACTION_SUMMARY.md` describes their extraction from the HL7
  V2.9.1 Word documents.
- Use: generator input for `tools/fhir-codegen` and nothing else. The tree is
  never packaged and never published.

## Licence

The repository states no licence. It has no `LICENSE` file at this commit
(the script refuses a pin where one appears), and GitHub reports no licence
for it. Its README reads:

> This IG is in development and is not yet production ready.  It is not expected to successfully build using IG publisher at this time.

The licence text HL7 attaches to its machine-readable v2 publication is the
licence page of the HL7 v2+ site, <https://github.com/HL7/v2plus> commit
`1a8fbb7e198047d71b9b13ceb582aaded4fa19f3`, `license.html` (sha256 `48e2d1108ab38faafb9993342a9626b52d673ce2e683ef123716950564d76efd`). It names the 2019 v2+
publication and the `http://www.HL7.org/legal/ippolicy.cfm` terms. Its
passages on use, quoted verbatim, one source line each:

> The 2019 Health Level Seven v2+ Publication is copyrighted by Health Level Seven, International (HL7) and is therefore protected by the Copyright Law of the United States and copyright provisions of various international treaties.  The effect of such laws and treaties is that you may not, without a license from Health Level Seven, International, copy or distribute HL7's publication Product.
>
> HL7 licenses its standards and select IP free of charge.
>
> A. HL7 INDIVIDUAL, STUDENT AND HEALTH PROFESSIONAL MEMBERS, who register and agree to the terms of HL7’s license, are authorized, without additional charge, to read, and to use Specified Material to develop and sell products and services that implement, but do not directly incorporate, the Specified Material in whole or in part without paying license fees to HL7.
>
> INDIVIDUAL, STUDENT AND HEALTH PROFESSIONAL MEMBERS wishing to incorporate additional items of Special Material in whole or part, into products and services, or to enjoy additional authorizations granted to HL7 ORGANIZATIONAL MEMBERS as noted below, must become ORGANIZATIONAL MEMBERS of HL7.
>
> B. HL7 ORGANIZATION MEMBERS, who register and agree to the terms of HL7's License, are authorized, without additional charge, on a perpetual (except as provided for in the full license terms governing the Material), non-exclusive and worldwide basis, the right to (a) download, copy (for internal purposes only) and share this Material with your employees and consultants for study purposes, and (b) utilize the Material for the purpose of developing, making, having made, using, marketing, importing, offering to sell or license, and selling or licensing, and to otherwise distribute, Compliant Products, in all cases subject to the conditions set forth in this Agreement and any relevant patent and other intellectual property rights of third parties (which may include members of HL7). No other license, sublicense, or other rights of any kind are granted under this Agreement.
>
> C. NON-MEMBERS, who register and agree to the terms of HL7’s IP policy for Specified Material, are authorized, without additional charge, to read and use the Specified Material for evaluating whether to implement, or in implementing, the Specified Material, and to use Specified Material to develop and sell products and services that implement, but do not directly incorporate, the Specified Material in whole or in part.
>
> NON-MEMBERS wishing to incorporate additional items of Specified Material in whole or part, into products and services, or to enjoy the additional authorizations granted to HL7 ORGANIZATIONAL MEMBERS, as noted above, must become ORGANIZATIONAL MEMBERS of HL7.

Copying for internal purposes only does not permit redistribution, so this
tree takes the path for such material: it is fetched into an ignored
directory at build time and the repository ships none of it.

## The owner's decision (2026-09-25)

The owner decided on 2026-09-25 to use this material as a generator input,
under the HL7 organisational membership the owner holds, which is the
condition the licence text above sets on incorporating the specified material.
The Rust code the generator derives from it is the project's own code and
carries Apache-2.0, as `fhir-types` does for the FHIR model (#253). The
fetched definitions themselves are never committed, packaged or published.
