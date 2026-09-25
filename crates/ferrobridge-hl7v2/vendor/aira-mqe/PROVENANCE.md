<!-- This file describes third-party material fetched at build time; the bytes
     it describes keep their upstream terms and are never committed. -->

# Provenance: the AIRA MQE example messages

Fetched verbatim at build time by `scripts/vendor/hl7v2-samples.sh
--build-time` into this directory, which `.gitignore` refuses except for
this file. Never commit a file from here and never edit one: change the pin in
docs/VERSIONS.md, run the script with `--build-time --stamp`, and record the
count and digest it prints.

- Source: <https://github.com/immregistries/mqe>
- Pin: commit `8b8a4274e1830cdb78ff5c8627763e1625caeac7`, the nine `examples/*.hl7.txt`
  files, each a run of generated VXU^V04 2.5.1 messages
- Files: 9
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `1fbda5ede288bdaf9ea53b16ded2149f11627cf4b0b39d2837299ab5803972b7`
- Stamped: 2026-09-25

## Terms

The repository states no licence: it has no licence file at this commit and
GitHub reports none for it. With no grant to redistribute, the files are
fetched at build time and never committed; the owner ruled on 2026-09-25
(#292). The corpus test reads the first messages of each file as a
deterministic smoke subset.
