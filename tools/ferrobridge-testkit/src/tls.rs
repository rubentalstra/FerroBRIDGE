// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Throwaway TLS material for the container tests: a CA, a server
//! certificate it signs for `localhost` and `127.0.0.1`, and a second CA that
//! signs nothing.
//!
//! The material is generated at run time with the `openssl` command line, so
//! no key is ever committed. A test that needs it and finds no `openssl` on
//! `PATH` gets [`TlsError::MissingOpenssl`] and says it skipped.
//!
//! No specification governs the harness; it is FerroBRIDGE's own design.

use std::path::Path;
use std::process::Command;

/// The certificates and the key one TLS-only PostgreSQL is started with.
#[derive(Clone)]
pub struct TlsMaterial {
    ca: Vec<u8>,
    stranger: Vec<u8>,
    server_certificate: Vec<u8>,
    server_key: Vec<u8>,
}

impl std::fmt::Debug for TlsMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsMaterial").finish_non_exhaustive()
    }
}

/// The TLS material could not be generated.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TlsError {
    /// No `openssl` command is on `PATH`.
    #[error("no `openssl` command is on PATH")]
    MissingOpenssl,
    /// `openssl` could not be run.
    #[error("`openssl {step}` could not be run")]
    Spawn {
        /// The subcommand that was being run.
        step: &'static str,
        /// What the operating system reported.
        #[source]
        source: std::io::Error,
    },
    /// `openssl` ran and failed.
    #[error("`openssl {step}` failed: {stderr}")]
    Failed {
        /// The subcommand that failed.
        step: &'static str,
        /// What it wrote to standard error.
        stderr: String,
    },
    /// The scratch directory or a generated file could not be used.
    #[error("the scratch directory could not be used")]
    Scratch {
        /// What the file system reported.
        #[source]
        source: std::io::Error,
    },
}

impl TlsMaterial {
    /// Generates a CA, a server certificate it signs, and an unrelated CA.
    ///
    /// # Errors
    ///
    /// Returns [`TlsError::MissingOpenssl`] when no `openssl` is on `PATH`,
    /// and the other [`TlsError`] variants when a step fails.
    pub fn generate() -> Result<Self, TlsError> {
        let scratch = tempfile::tempdir().map_err(|source| TlsError::Scratch { source })?;
        let dir = scratch.path();
        std::fs::write(
            dir.join("server.ext"),
            "subjectAltName=DNS:localhost,IP:127.0.0.1\n\
             basicConstraints=critical,CA:FALSE\n\
             keyUsage=critical,digitalSignature\n\
             extendedKeyUsage=serverAuth\n",
        )
        .map_err(|source| TlsError::Scratch { source })?;
        for name in ["ca", "stranger"] {
            openssl(
                dir,
                "req",
                &[
                    "req",
                    "-x509",
                    "-newkey",
                    "ec",
                    "-pkeyopt",
                    "ec_paramgen_curve:prime256v1",
                    "-nodes",
                    "-days",
                    "2",
                    "-subj",
                    &format!("/CN=ferrobridge test {name}"),
                    "-addext",
                    "basicConstraints=critical,CA:TRUE",
                    "-addext",
                    "keyUsage=critical,keyCertSign,cRLSign",
                    "-keyout",
                    &format!("{name}.key"),
                    "-out",
                    &format!("{name}.crt"),
                ],
            )?;
        }
        openssl(
            dir,
            "req",
            &[
                "req",
                "-new",
                "-newkey",
                "ec",
                "-pkeyopt",
                "ec_paramgen_curve:prime256v1",
                "-nodes",
                "-subj",
                "/CN=localhost",
                "-keyout",
                "server.key",
                "-out",
                "server.csr",
            ],
        )?;
        openssl(
            dir,
            "x509",
            &[
                "x509",
                "-req",
                "-in",
                "server.csr",
                "-CA",
                "ca.crt",
                "-CAkey",
                "ca.key",
                "-set_serial",
                "2",
                "-days",
                "2",
                "-extfile",
                "server.ext",
                "-out",
                "server.crt",
            ],
        )?;
        let read = |name: &str| {
            std::fs::read(dir.join(name)).map_err(|source| TlsError::Scratch { source })
        };
        Ok(Self {
            ca: read("ca.crt")?,
            stranger: read("stranger.crt")?,
            server_certificate: read("server.crt")?,
            server_key: read("server.key")?,
        })
    }

    /// Returns the PEM CA the server certificate chains to.
    #[must_use]
    pub fn ca(&self) -> &[u8] {
        &self.ca
    }

    /// Returns a PEM CA that signed nothing the server presents.
    #[must_use]
    pub fn stranger_ca(&self) -> &[u8] {
        &self.stranger
    }

    /// Returns the PEM server certificate.
    #[must_use]
    pub fn server_certificate(&self) -> &[u8] {
        &self.server_certificate
    }

    /// Returns the PEM server key.
    #[must_use]
    pub fn server_key(&self) -> &[u8] {
        &self.server_key
    }
}

/// Runs `openssl` with `args` in `dir`.
fn openssl(dir: &Path, step: &'static str, args: &[&str]) -> Result<(), TlsError> {
    let output = match Command::new("openssl").args(args).current_dir(dir).output() {
        Ok(output) => output,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(TlsError::MissingOpenssl);
        }
        Err(source) => return Err(TlsError::Spawn { step, source }),
    };
    if output.status.success() {
        Ok(())
    } else {
        Err(TlsError::Failed {
            step,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}
