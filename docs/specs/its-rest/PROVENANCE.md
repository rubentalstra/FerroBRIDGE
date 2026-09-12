<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the openEHR ITS-REST OpenAPI documents

Vendored verbatim by `scripts/vendor/its-rest.sh`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/openEHR/specifications-ITS-REST>
- Pin: tag `Release-1.1.0`, which resolves to commit `24058992d5fa96e8dfbd855d9c133f328387fc09`
- Fetched: 2026-09-12
- Upstream licence: the specification content declares `Creative Commons Attribution-NoDerivs 3.0 Unported` in each
  document's `info.license`. The repository's own `LICENSE` file is the
  Apache License 2.0 and is vendored beside this file, so both statements are
  here and neither is assumed.
- Layout: the upstream paths, unchanged
- Files: 4
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `e8572a95c1c0684987b57732fd67b0658ec340a2dced3c7c41b5dde4c3c8ad01`

## Why a blob id per file

Every document at this tag says `info.version: latest`, so the file content
carries no release identity of its own. The tag, the commit it resolves to, and
the git blob id of each file are what identify these bytes
(docs/architecture.md section 2).

Each of the three is `x-status: STABLE`, which the script checks. The Admin and
Demographic documents of the same release are `x-status: DEVELOPMENT` and are
not vendored.

| File | sha256 | git blob id |
|---|---|---|
| `computable/OAS/ehr-codegen.openapi.yaml` | `a0e37a217524c5a2c6351d128041c88d1c137fcde106badda05dde8c0269cd5c` | `d18ba6bbb0ac503a62840c0e83d8fdfbb72bf415` |
| `computable/OAS/query-codegen.openapi.yaml` | `d92e82c9cd6c9c8f6543ea425ea88b11e2fd0b133a1003347d470625d1d19bec` | `0a56228f763f1306a85a1edf4258a3a8a1d07757` |
| `computable/OAS/definition-codegen.openapi.yaml` | `6c30fe7552ee7fea57ae97e00137f863a0b28a733c27dc4ed97c5ea9bacae930` | `28fec040ff586e0e425bf6c1303716b4c638bb7f` |
| `LICENSE` | `c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4` | `261eeb9e9f8b2b4b0d119366dda99c6fd7d35c64` |
