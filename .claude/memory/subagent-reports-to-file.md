---
name: subagent-reports-to-file
description: A subagent's long report must be written to a scratchpad file by the agent itself; a completed agent cannot be resumed in this harness and a truncated result is lost
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

On 2026-09-12 two long research reports came back truncated in the tool
result, and SendMessage to the completed agents failed with "No transcript
found for agent ID", so half of each report had to be recovered from the
transcript JSONL by script.

**Why:** the orchestrator only sees the final message, which is cut at a
size limit, and a finished agent is not resumable.

**How to apply:** every research or register prompt tells the agent to Write
its full report to a named file under the session scratchpad and to reply
with the path and line count; the orchestrator reads the file.
