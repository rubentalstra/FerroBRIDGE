<!-- This file describes third-party material fetched at build time; the bytes
     it describes keep their upstream terms and are never committed. -->

# Provenance: the NIST HL7 v2 test bundles

Fetched verbatim at build time by `scripts/vendor/hl7v2-samples.sh
--build-time` into this directory, which `.gitignore` refuses except for
this file. Never commit a file from here and never edit one: change a pin in
docs/VERSIONS.md, run the script with `--build-time --stamp`, and record the
counts and digests it prints.

- Source: <https://github.com/usnistgov/hit-mu-tools-resource-bundles>, three branches, each
  pinned by commit
- `lri-r2/`: branch `lri-r2` at commit `060f7af14daa359937f326c361bb9610b7a8ba48`,
  the Laboratory Results Interface bundle: 117 files, tree
  digest `666bafb90db5eeec7f727617fcbbe51b778065f93bc726f2746f524d70de7ec4`
- `loi-r1/`: branch `loi-r1` at commit `2c5e502787cd5e0fe0f347a7c00d5507a9f8035a`,
  the Laboratory Orders Interface bundle: 65 files, tree
  digest `712d023d5a7ceb21ba520f513490fe1a29603436ed4756f48646d5922702f549`
- `ss-r2/`: branch `ss-r2` at commit `0d7a2c7b714188878b030068a94b94d1db6cb30a`, the
  syndromic surveillance bundle: 24 files, tree digest
  `aca07c08f2718139456f712ce8e45fc42cadba9c9fed665fd7ef5465c30af4c6`
- Taken from each: every `src/main/resources/Context*/**/Message.txt` and
  `src/main/resources/About/Disclaimer.html`, at their upstream layout
- Tree digest: sha256 over the sorted per-file `sha256  path` listing of each
  branch directory
- Stamped: 2026-09-25. The script verifies each count and digest on
  every fetch, and keeps a tree already on disk at its digest.

## Terms

The repository carries no licence file. Each bundle's
`About/Disclaimer.html` states, quoted from the LRI bundle with its markup
removed:

> This software was developed at the NIST by employees of the Federal Government in the course of their official duties. Pursuant to title 17 Section 105 of the United States Code this software is not subject to copyright protection and is in the public domain. NIST assumes no responsibility whatsoever for its use by other parties, and makes no guarantees, expressed or implied, about its quality, reliability, or any other characteristic. We would appreciate acknowledgment if the software is used. This software can be redistributed and/or modified freely provided that any derivative works bear some notice that they are derived from it, and any modified versions bear some notice that they have been modified.

The test messages are NIST's own test data, never patient data. The corpus
test reads them as message-only smoke coverage and writes nothing from them
into this repository; the owner ruled on 2026-09-25 (#292) to fetch them at
build time.
