<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Fuzz targets

`cargo fuzz` (libFuzzer) over the two parsers that read text a caller
controls before anything else does: the YAML mapping loader and the openEHR
mapping-path parser of `openehr-mapping-core` (#155). This crate sits outside
the workspace because cargo-fuzz needs a nightly toolchain for its sanitizer
flags; nothing built here ships.

| Target | Entry point | Seeds |
|---|---|---|
| `mapping_loader` | `loader::parse_str` and `loader::load_str` | `seeds/mapping_loader/`: the synthetic malformed fixtures and a sample of the vendored mapping library |
| `mapping_path` | `MappingPath::from_str` then `resolve` against a fixed anchor | `seeds/mapping_path/`: the path shapes the library and the tests use |

A finding is a panic, an abort or a hang. An `Err` is the parser doing its
job and is never a finding. Run one target locally for a minute:

```sh
mkdir -p fuzz/corpus/mapping_loader && cp fuzz/seeds/mapping_loader/* fuzz/corpus/mapping_loader/
cargo +nightly fuzz run mapping_loader fuzz/corpus/mapping_loader -- -max_total_time=60
```

The scheduled lane is `.github/workflows/fuzz.yml`; a reproducing input is
uploaded as a run artifact and becomes a `bug` issue with the input attached.
