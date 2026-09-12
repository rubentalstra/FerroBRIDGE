---
name: mermaid-diagrams
description: The design of record carries mermaid diagrams; validate every fence with mermaid-cli against the installed Chrome before a pull request, and remember a semicolon ends a sequence-diagram message
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

Owner request 2026-09-12: `docs/architecture.md` explains the design with
mermaid diagrams as well as text, because a picture is faster to check than
prose. GitHub and the mdBook site (`mdbook-mermaid`) both render them.

**Why:** the first pass shipped one diagram that did not parse (a semicolon in
a sequence-diagram message ends the statement), and GitHub shows a parse error
where the picture should be.

**How to apply:** extract every ```` ```mermaid ```` fence and render each with
`npx -y -p @mermaid-js/mermaid-cli mmdc -p <puppeteer.json> -i x.mmd -o x.svg`,
where the puppeteer config points `executablePath` at the installed Google
Chrome (the browser extension and a bundled Chromium are not available). Use
`<br/>` for line breaks inside labels, never `\n`; keep `;` out of sequence
messages; quote any label carrying parentheses, colons or braces.
