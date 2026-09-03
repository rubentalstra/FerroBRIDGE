<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Machine review (SonarQube Cloud): advisory, never authority

Every pull request and every push to `main` is analyzed by SonarQube Cloud
(`.github/workflows/sonar.yml`; scope in `sonar-project.properties`; project
`rubentalstra_FerroBRIDGE`, organization `rubentalstra`, the built-in "Sonar
way" quality gate). It exists as a second opinion beside the local gates and
CodeQL, and it also reads the trees the Rust gates never see: shell, workflow
YAML, and JSON.

The repository is in its design phase, so today the lane runs the
multi-language sweep only. Rust is analyzed first-party once a workspace exists
(the analyzer runs Clippy itself over the workspace), and the coverage import
joins the lane in the same change (`ci-cd.md`).

It is a **second opinion**. It is not authority, and it gates no merge.

## Precedence: a finding never outranks the sources

1. The FHIR, FHIRconnect, OMOP, OMOCL, and openEHR specifications
   (`spec-adherence.md`): the oracle.
2. The hard rules: `CLAUDE.md` and the `.claude/rules/*.md` files.
3. The local gates: `shellcheck`, `actionlint`, `zizmor`, and once code
   exists `cargo fmt`, `clippy`, `cargo nextest`, `cargo deny`, plus the CI
   guards.
4. The analyzer.

A finding that contradicts a spec citation, or asks for something the rules
forbid, is wrong by construction. Nothing it reports relaxes `testing.md`:
never weaken a test because a finding suggested it.

## Rules

- **It gates no merge.** The quality gate is informational; merges are decided
  by the local gates and review.
- **Findings are acted on by hand, in a normal change**, never applied through
  a UI that would attribute a commit to a bot. The no-AI-attribution rule has
  no exceptions.
- **Automatic Analysis stays OFF.** CI-based analysis and SonarQube Cloud's
  Automatic Analysis cannot both run; `sonar.yml` is the analysis path.
- **A wrong finding is data, not a silent suppression:** record it on the
  tracker, or adjust `sonar-project.properties` when the scope is genuinely
  wrong, never to make a number go down.

CodeQL (`.github/workflows/codeql.yml`) runs separately as the security
scanner and is advisory here too, until a precision case is made to gate on it.

## Official documentation (durable citations)

- <https://docs.sonarsource.com/sonarqube-cloud/>
- <https://docs.sonarsource.com/sonarqube-cloud/analyzing-source-code/languages/rust>
