<!-- This file describes third-party material fetched at build time; the bytes
     it describes keep their upstream terms and are never committed. -->

# Provenance: the HL7 v2 legacy tables (NIST IGAMT export of HL7's v2 database)

Fetched verbatim at build time by `scripts/vendor/v2-legacy.sh` into this
directory, which `.gitignore` refuses except for this file. Never commit a
file from here and never edit one: change the pin in docs/VERSIONS.md, run
`scripts/vendor/v2-legacy.sh --stamp`, and record the count and digest it
prints.

- Source: <https://github.com/usnistgov/igamt-hl7Tools-service>
- Pin: commit `83322dffdac18cb129a1bd80149f5439cf5686fd`, path `src/main/resources/hl7db`, taken whole at its upstream layout
- Files: 169
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `967715c682b43383ba28894e171fd1e5c2a5980b16b5676a1c60f3fd0a5c2c92`
- Stamped: 2026-09-25. The script verifies the count and the digest on every
  fetch, and keeps a tree already on disk at that digest.
- Content: one directory per HL7 v2 version (2.1 2.2 2.3 2.3.1 2.4 2.5 2.5.1 2.6 2.7 2.7.1 2.8 2.8.1 2.8.2), each a set of
  JSON tables (messages, groups, elements, segments, fields, data elements,
  data types, events, tables and codes) that the NIST IGAMT tooling exported
  from HL7's v2 database. The repository's `readme.md` describes the export:

  > The HLTools to IGAMT Lite is a process for converting from an HL7 supplied database to the IGAMT database.
  >
  > Before this program can be run, one must transfer the following tables from the official HL7v2 standards database into a
  > mySQL database:

- Use: generator input for `tools/fhir-codegen` and nothing else, read for
  the message structures the v2.9.1 definitions (`../hl7-v2ig/`) no longer
  carry and the segments those structures name. The tree is never packaged
  and never published.

## Licence

The repository states no licence. It has no `LICENSE` file at this commit
(the script refuses a pin where one appears), and GitHub reports no licence
for it. The tables are HL7's copyrighted content: a public-domain status of
NIST's own work, or a label a third party gives its extract, does not
relicense them. The licence text HL7 attaches to its machine-readable v2
publication is the licence page of the HL7 v2+ site,
<https://github.com/HL7/v2plus> commit `1a8fbb7e198047d71b9b13ceb582aaded4fa19f3`, `license.html` (sha256
`48e2d1108ab38faafb9993342a9626b52d673ce2e683ef123716950564d76efd`). Its passages on use, quoted verbatim, one source line each:

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

The owner decided on 2026-09-25 to use HL7's v2 definitions as a generator
input under the HL7 organisational membership the owner holds, which is the
condition the licence text above sets on incorporating the specified
material, and its decision on #303 the same day extends that to this export. The Rust code the
generator derives from it is the project's own code and carries Apache-2.0,
as `hl7v2-types` does for the v2.9.1 tables. The fetched tables themselves
are never committed, packaged or published.
