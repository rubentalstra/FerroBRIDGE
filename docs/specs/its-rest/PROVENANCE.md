<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the openEHR ITS-REST OpenAPI documents and Simplified Formats

Vendored verbatim by `scripts/vendor/its-rest.sh`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/openEHR/specifications-ITS-REST>
- Pin: tag `Release-1.1.0`, which resolves to commit `24058992d5fa96e8dfbd855d9c133f328387fc09`
- Fetched: 2026-09-24
- Upstream licence: the specification content declares `Creative Commons Attribution-NoDerivs 3.0 Unported` in each
  document's `info.license`. The repository's own `LICENSE` file is the
  Apache License 2.0 and is vendored beside this file, so both statements are
  here and neither is assumed.
- Layout: the upstream paths, unchanged
- Files: 17
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `f9874abc6e3eb4e1d4146707ff41f2114c08104f19ef93058c866f78c22f2354`

## Why a blob id per file

Every document at this tag says `info.version: latest`, so the file content
carries no release identity of its own. The tag, the commit it resolves to, and
the git blob id of each file are what identify these bytes
(docs/architecture.md section 2).

Each of the three is `x-status: STABLE`, which the script checks. The Admin and
Demographic documents of the same release are `x-status: DEVELOPMENT` and are
not vendored.

`docs/simplified_formats/` and `docs/simplified_data_template/` are the
AsciiDoc sources of the Simplified Formats and Simplified Data Template
sub-specifications at the same commit, taken whole. The FLAT format of the
composition seam, its `ctx/` shortcuts and its `_`-prefixed attribute
families (`master05-rm_mapping.adoc`) are specified there.

| File | sha256 | git blob id |
|---|---|---|
| `LICENSE` | `c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4` | `261eeb9e9f8b2b4b0d119366dda99c6fd7d35c64` |
| `computable/OAS/definition-codegen.openapi.yaml` | `6c30fe7552ee7fea57ae97e00137f863a0b28a733c27dc4ed97c5ea9bacae930` | `28fec040ff586e0e425bf6c1303716b4c638bb7f` |
| `computable/OAS/ehr-codegen.openapi.yaml` | `a0e37a217524c5a2c6351d128041c88d1c137fcde106badda05dde8c0269cd5c` | `d18ba6bbb0ac503a62840c0e83d8fdfbb72bf415` |
| `computable/OAS/query-codegen.openapi.yaml` | `d92e82c9cd6c9c8f6543ea425ea88b11e2fd0b133a1003347d470625d1d19bec` | `0a56228f763f1306a85a1edf4258a3a8a1d07757` |
| `docs/simplified_data_template/manifest_vars.adoc` | `ce57e6dc7b29a182074f2045ead7197eb2b4b048438cd29cb43266f314686561` | `a02f2c66dc9cd10ece8be8d6b0d03ad50385ee69` |
| `docs/simplified_data_template/master.adoc` | `0b590f86097ae634f05db56bf50814ff2a87cfc7a7118e78ae34e2980f9ba9a1` | `33ef9465ebbc0f07d965bce0149f2f740758912e` |
| `docs/simplified_data_template/master00-amendment_record.adoc` | `96f160d1caa4cf0616c0f53114468801c938f86afa0803654350edc95cbed93c` | `d738d214623ead7649aa944422fce5268d3359b6` |
| `docs/simplified_data_template/master01-preface.adoc` | `c6dec3b4a1145e84220deebca33adecb322fd1342783e349ce30a1c5a804f0c0` | `fd287ee138e6729bf5bf2e59e65dbd99e57569a2` |
| `docs/simplified_formats/manifest_vars.adoc` | `f4403c6716277ade525a825486b8a0367d9706ce19a446cd23f002d57f946092` | `3c52e10df6b8eefae0be0407c96a74825241fd1d` |
| `docs/simplified_formats/master.adoc` | `235148db43f7ac6355c5b59695b86a79c1b7b65cabd08d3850d7c35e03ef9906` | `58d5096f8d895388ff354978fb2f14c875f9a424` |
| `docs/simplified_formats/master00-amendment_record.adoc` | `cecfc0a6cc6d2742cf8a79bd30eb82a30c7634328f2a540af1fd6302985396e9` | `c452eb82693001bc52a1182d60c390cfae200582` |
| `docs/simplified_formats/master01-preface.adoc` | `fc448f5dd1b00642d99ee25fe2688a7b4c0f01f96cb10df6ce7f9feac663de66` | `267ccb9a1916965049a5b6a0533df0c810a50a2a` |
| `docs/simplified_formats/master02-overview.adoc` | `335f4456857102dfab55114611f2b5b5b59b461feda9e9362dd1a81c92117dfb` | `c36aa03494918386ae41d15b860d6e12bd191b95` |
| `docs/simplified_formats/master03-design_rationale.adoc` | `d53b17e711f12af6df03e25b2c9438fea7a790d77d132a48a07ed648afdd0a49` | `3484777328f9421aba1e3e5ebeaa6e28c19cf383` |
| `docs/simplified_formats/master04-basic_concepts.adoc` | `fed39484b2c6ea95343967e79dd4d3b4abad980a77a202ed948eaf7e1631c34e` | `6fe34b3584a1890642d7b991c865692ee04ff138` |
| `docs/simplified_formats/master05-rm_mapping.adoc` | `dffa4f155685b9b18d165a433b1db0def65384111ce8b2429386a97076b31142` | `33396b8d93fd9147a7b8664f7b16d25480cbaf32` |
| `docs/simplified_formats/master06-context_information.adoc` | `2d64c8cf6bf76e75b72739b61d56b33f61c664a03d8d9bf16492341e11688101` | `a34f767b35a2bc16505cc7e181e5677eef75c81c` |
