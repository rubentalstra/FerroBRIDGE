# ferrobridge-server

The one binary. `main.rs` is thin over `lib.rs` so the integration tests drive
the real run path; a binary-only crate cannot be imported from `tests/`
(<https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-testing.html>).

- **`main.rs` never grows.** It calls `run(std::env::args())` and returns the
  `ExitCode`. Everything else lives in a module of the library.
- **Every configuration default lives inline in its struct's `Default` impl**,
  with container-level `#[serde(default, deny_unknown_fields)]`
  (`.claude/rules/rust-style.md` §Default values). The per-field
  `#[serde(default = "path")]` form is banned, and so is a `fn default_x()`.
- **A new configuration key is three changes in one edit**: the field with its
  default, the Operate page's variable table in `website/book/src/operate/`,
  and a case in `tests/it/config.rs`. A key with no documented variable name is
  a key an operator cannot set.
- **A secret is reachable through a `<key>_file` sibling, read once at boot**,
  and lives in a `SecretString` after that. A value and its `_file` together is
  a boot error, and so is an unknown key. The refusal names the key.
- **No clinical content in any log line.** The request log carries the matched
  route, never the raw URL, and a query value only from the allowlist. A body
  never reaches the log, a `Debug` rendering, or an error message. Adding a
  field to `request_log` needs the same judgement as adding one to a wire body.
- **An upstream failure is a typed error carrying the upstream status.** A
  readiness indicator reports a reason; it never reports up because a call
  failed quietly.
- **The panic catcher exists because the release profile pins
  `panic = "unwind"`.** Keep both: an `abort` regression would make the `500`
  untestable by construction
  (<https://doc.rust-lang.org/cargo/reference/profiles.html#panic>).
- **A status is a `StatusCode`**, compared as one. The facade's own status
  table arrives with the facade.
- **Tests are one binary**, `tests/it/main.rs` plus one module per topic, and
  they drive the public seams: `run`, `Config::from_sources`, `router`,
  `with_middleware`, `serve_until`. A test that needs its own route builds one
  and hands it to `with_middleware`, so the stack under test is the shipped
  stack.
- The upstream stubs are `wiremock`; every fixture is synthetic.
