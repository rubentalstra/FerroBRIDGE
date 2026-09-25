// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CDM database connection both clients open: the URL, its `sslmode`,
//! and the CA the server certificate is checked against.
//!
//! The concept resolver's `sqlx` pool and the writer's `tokio-postgres`
//! connection read one [`CdmConnection`], so the two clients agree on whether
//! the connection is encrypted and what it trusts. The modes are the four
//! libpq modes that never fall back to a plaintext connection
//! (<https://www.postgresql.org/docs/current/libpq-ssl.html#LIBPQ-SSL-SSLMODE-STATEMENTS>);
//! `prefer` and `allow` are refused, because a client that honours them
//! silently downgrades when the server offers no TLS.
//!
//! No specification governs the client's TLS beyond PostgreSQL's own
//! documentation: our own design.

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore};
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use std::fmt;
use std::sync::Arc;

/// How the connection is secured, after libpq's `sslmode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SslMode {
    /// No TLS.
    Disable,
    /// TLS; the server certificate is checked against the CA only when one is
    /// configured, as libpq does when a root certificate file is present.
    Require,
    /// TLS, and the server certificate chains to a trusted CA.
    VerifyCa,
    /// TLS, the server certificate chains to a trusted CA, and it names the
    /// host the URL connects to.
    VerifyFull,
}

impl SslMode {
    /// Returns the mode as libpq spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }
}

impl fmt::Display for SslMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A connection URL or CA the CDM clients refuse before connecting.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConnectionError {
    /// The URL names a mode that falls back to a plaintext connection.
    #[error(
        "sslmode={mode} falls back to a plaintext connection when the server offers no TLS, \
         which the CDM clients never do; set disable, require, verify-ca or verify-full"
    )]
    FallbackMode {
        /// The mode the URL names.
        mode: String,
    },
    /// The URL names a mode libpq does not define.
    #[error("sslmode={mode} is not a libpq mode; set disable, require, verify-ca or verify-full")]
    UnknownMode {
        /// The mode the URL names.
        mode: String,
    },
    /// The URL names no `sslmode`.
    #[error(
        "the URL names no sslmode, and libpq's default, prefer, falls back to plaintext; \
         set disable, require, verify-ca or verify-full"
    )]
    MissingMode,
    /// The environment sets a libpq TLS variable the pool's client would read
    /// and the writer's would not.
    #[error(
        "the environment sets {name}, which the pool's client reads and the writer's does not; \
         unset it and configure the CA instead"
    )]
    Environment {
        /// The variable the environment sets.
        name: &'static str,
    },
    /// The URL names `sslmode` more than once.
    #[error("the URL names sslmode more than once")]
    RepeatedMode,
    /// The URL carries a TLS parameter the CDM clients do not read.
    #[error(
        "the URL carries `{name}`, which the CDM clients do not read; the CA comes from the \
         configuration and no client certificate is sent"
    )]
    Parameter {
        /// The parameter the URL carries.
        name: String,
    },
    /// A CA is configured for a connection that uses no TLS.
    #[error("a CA is configured and the URL says sslmode=disable")]
    CaWithoutTls,
    /// The CA holds no certificate.
    #[error("the CA holds no PEM certificate")]
    NoCertificate,
    /// The CA is not PEM the parser reads.
    #[error("the CA is not a PEM certificate")]
    Pem {
        /// What the PEM parser reported.
        #[source]
        source: rustls::pki_types::pem::Error,
    },
    /// A certificate of the CA is not one a trust anchor can be made from.
    #[error("a certificate of the CA cannot be trusted as a root")]
    Root {
        /// What rustls reported.
        #[source]
        source: rustls::Error,
    },
    /// The TLS configuration could not be built.
    #[error("the TLS configuration could not be built")]
    Tls {
        /// What rustls reported.
        #[source]
        source: rustls::Error,
    },
    /// The certificate verifier could not be built.
    #[error("the certificate verifier could not be built")]
    Verifier {
        /// What rustls reported.
        #[source]
        source: rustls::client::VerifierBuilderError,
    },
    /// The writer's client refused the URL.
    #[error("the URL is not a PostgreSQL connection URL")]
    Url {
        /// What `tokio-postgres` reported.
        #[source]
        source: tokio_postgres::Error,
    },
    /// The pool's client refused the URL.
    #[error("the URL is not a PostgreSQL connection URL the pool reads")]
    PoolUrl {
        /// What `sqlx` reported.
        #[source]
        source: sqlx::Error,
    },
}

/// The CDM database both clients connect to, with its TLS settled.
///
/// Built once from the URL and the configured CA, so a mode the clients
/// cannot honour or a CA that does not parse is refused before any
/// connection is tried.
#[derive(Clone)]
pub struct CdmConnection {
    mode: SslMode,
    ca: Option<Vec<u8>>,
    writer: tokio_postgres::Config,
    pool: PgConnectOptions,
    tls: Arc<ClientConfig>,
}

impl fmt::Debug for CdmConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The URL carries the password, so neither parsed form is shown.
        f.debug_struct("CdmConnection")
            .field("mode", &self.mode)
            .field("ca", &self.ca.is_some())
            .finish_non_exhaustive()
    }
}

impl CdmConnection {
    /// Reads `url` and the PEM CA `ca`, when one is configured.
    ///
    /// The URL must name its `sslmode`: libpq's default when it names none is
    /// `prefer`, which these clients never honour.
    ///
    /// Without a CA, `verify-ca` and `verify-full` trust the webpki root set,
    /// the one the pool's client carries. sqlx reads `PGSSLROOTCERT`,
    /// `PGSSLCERT` and `PGSSLKEY` from the environment whatever the options
    /// say, so a process that sets one is refused, which keeps the two clients
    /// on one trust set.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectionError::MissingMode`] for a URL without `sslmode`,
    /// [`ConnectionError::Environment`] when a libpq TLS variable is set,
    /// [`ConnectionError::FallbackMode`] for `prefer` and `allow`,
    /// [`ConnectionError::UnknownMode`] and [`ConnectionError::RepeatedMode`]
    /// for a mode that is not one of the four, [`ConnectionError::Parameter`]
    /// for any other `ssl` parameter, [`ConnectionError::CaWithoutTls`] for a
    /// CA with `disable`, the CA errors when `ca` does not parse, and
    /// [`ConnectionError::Url`] or [`ConnectionError::PoolUrl`] when a client
    /// refuses the URL.
    pub fn new(url: &str, ca: Option<&[u8]>) -> Result<Self, ConnectionError> {
        // NOTE: rustls docs, `CryptoProvider`: a process with two providers
        // compiled in has no default, and sqlx 0.9's verify-ca check reads it.
        if CryptoProvider::get_default().is_none() {
            // An `Err` means another caller installed one first, which serves.
            drop(CryptoProvider::install_default(
                rustls::crypto::aws_lc_rs::default_provider(),
            ));
        }
        if let Some(name) = SQLX_TLS_ENVIRONMENT
            .into_iter()
            .find(|name| std::env::var_os(name).is_some())
        {
            return Err(ConnectionError::Environment { name });
        }
        let (bare, mode) = split_ssl(url)?;
        if mode == SslMode::Disable && ca.is_some() {
            return Err(ConnectionError::CaWithoutTls);
        }
        let roots = match ca {
            Some(pem) => Some(roots_of(pem)?),
            None => None,
        };
        let tls = Arc::new(client_config(mode, roots)?);
        let mut writer: tokio_postgres::Config = bare
            .parse()
            .map_err(|source| ConnectionError::Url { source })?;
        writer.ssl_mode(match mode {
            SslMode::Disable => tokio_postgres::config::SslMode::Disable,
            SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => {
                tokio_postgres::config::SslMode::Require
            }
        });
        let mut pool: PgConnectOptions = bare
            .parse()
            .map_err(|source| ConnectionError::PoolUrl { source })?;
        // NOTE: sqlx 0.9 `PgSslMode::Require` never checks the certificate, so
        // a configured CA asks it for `VerifyCa`, the check libpq makes.
        pool = pool.ssl_mode(match (mode, ca) {
            (SslMode::Disable, _) => PgSslMode::Disable,
            (SslMode::Require, None) => PgSslMode::Require,
            (SslMode::Require | SslMode::VerifyCa, _) => PgSslMode::VerifyCa,
            (SslMode::VerifyFull, _) => PgSslMode::VerifyFull,
        });
        if let Some(pem) = ca {
            pool = pool.ssl_root_cert_from_pem(pem.to_vec());
        }
        Ok(Self {
            mode,
            ca: ca.map(<[u8]>::to_vec),
            writer,
            pool,
            tls,
        })
    }

    /// Returns the mode the connection is secured with.
    #[must_use]
    pub fn ssl_mode(&self) -> SslMode {
        self.mode
    }

    /// Returns whether a CA is configured.
    #[must_use]
    pub fn has_ca(&self) -> bool {
        self.ca.is_some()
    }

    /// Returns the options a [`crate::database::CdmPool`] connects with.
    #[must_use]
    pub fn pool_options(&self) -> PgConnectOptions {
        self.pool.clone()
    }

    /// Returns the writer's client configuration and its TLS connector.
    pub(crate) fn writer(
        &self,
    ) -> (
        &tokio_postgres::Config,
        tokio_postgres_rustls::MakeRustlsConnect,
    ) {
        (
            &self.writer,
            tokio_postgres_rustls::MakeRustlsConnect::new(ClientConfig::clone(&self.tls)),
        )
    }
}

/// The libpq TLS variables sqlx 0.9 reads into every `PgConnectOptions`
/// (<https://www.postgresql.org/docs/current/libpq-envars.html>).
const SQLX_TLS_ENVIRONMENT: [&str; 3] = ["PGSSLROOTCERT", "PGSSLCERT", "PGSSLKEY"];

/// Returns `url` without its `sslmode`, and the mode it named.
fn split_ssl(url: &str) -> Result<(String, SslMode), ConnectionError> {
    let Some((address, query)) = url.split_once('?') else {
        return Err(ConnectionError::MissingMode);
    };
    let mut mode = None;
    let mut kept = Vec::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == "sslmode" {
            if mode.is_some() {
                return Err(ConnectionError::RepeatedMode);
            }
            mode = Some(mode_of(value)?);
        } else if key.starts_with("ssl") {
            return Err(ConnectionError::Parameter {
                name: key.to_owned(),
            });
        } else {
            kept.push(pair);
        }
    }
    let bare = if kept.is_empty() {
        address.to_owned()
    } else {
        format!("{address}?{}", kept.join("&"))
    };
    Ok((bare, mode.ok_or(ConnectionError::MissingMode)?))
}

/// Returns the mode libpq spells `value`.
fn mode_of(value: &str) -> Result<SslMode, ConnectionError> {
    match value {
        "disable" => Ok(SslMode::Disable),
        "require" => Ok(SslMode::Require),
        "verify-ca" => Ok(SslMode::VerifyCa),
        "verify-full" => Ok(SslMode::VerifyFull),
        "prefer" | "allow" => Err(ConnectionError::FallbackMode {
            mode: value.to_owned(),
        }),
        _ => Err(ConnectionError::UnknownMode {
            mode: value.to_owned(),
        }),
    }
}

/// Returns the trust anchors of the PEM CA `pem`.
fn roots_of(pem: &[u8]) -> Result<RootCertStore, ConnectionError> {
    let mut roots = RootCertStore::empty();
    for certificate in CertificateDer::pem_slice_iter(pem) {
        let certificate = certificate.map_err(|source| ConnectionError::Pem { source })?;
        roots
            .add(certificate)
            .map_err(|source| ConnectionError::Root { source })?;
    }
    if roots.is_empty() {
        return Err(ConnectionError::NoCertificate);
    }
    Ok(roots)
}

/// Returns the writer's TLS configuration for `mode`, trusting `roots` when a
/// CA is configured and the webpki root set otherwise.
fn client_config(
    mode: SslMode,
    roots: Option<RootCertStore>,
) -> Result<ClientConfig, ConnectionError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|source| ConnectionError::Tls { source })?;
    let checked = match (mode, roots) {
        (SslMode::Disable | SslMode::Require, None) => None,
        (SslMode::Require | SslMode::VerifyCa, Some(roots)) => Some((roots, false)),
        (SslMode::VerifyCa, None) => Some((webpki(), false)),
        (SslMode::VerifyFull, roots) => Some((roots.unwrap_or_else(webpki), true)),
        (SslMode::Disable, Some(_)) => return Err(ConnectionError::CaWithoutTls),
    };
    let config = match checked {
        None => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyCertificate { provider }))
            .with_no_client_auth(),
        Some((roots, true)) => builder.with_root_certificates(roots).with_no_client_auth(),
        Some((roots, false)) => {
            let chain = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
                .build()
                .map_err(|source| ConnectionError::Verifier { source })?;
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AnyName { chain }))
                .with_no_client_auth()
        }
    };
    Ok(config)
}

/// Returns the webpki root set.
fn webpki() -> RootCertStore {
    RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned())
}

/// The `require` check without a CA: any certificate, with the handshake
/// signatures still verified, so the channel is encrypted to whoever holds
/// the certificate's key.
#[derive(Debug)]
struct AnyCertificate {
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // NOTE: PostgreSQL docs, libpq "SSL Mode Descriptions": require without
        // a root certificate encrypts and does not verify the server.
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The `verify-ca` check: the chain to a trusted CA, whatever host the
/// certificate names.
#[derive(Debug)]
struct AnyName {
    chain: Arc<WebPkiServerVerifier>,
}

impl ServerCertVerifier for AnyName {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.chain.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            outcome => outcome,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.chain.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.chain.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.chain.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::{CdmConnection, ConnectionError, SslMode};

    const URL: &str = "postgres://bridge:secret@db.invalid:5432/cdm";

    #[test]
    fn an_absent_mode_is_refused_naming_the_four_and_each_libpq_mode_reads() {
        for url in [URL.to_owned(), format!("{URL}?application_name=x")] {
            let absent = CdmConnection::new(&url, None).expect_err("a URL names its mode");
            assert!(matches!(absent, ConnectionError::MissingMode), "{absent:?}");
            assert!(
                absent
                    .to_string()
                    .contains("disable, require, verify-ca or verify-full"),
                "{absent}"
            );
        }
        for mode in [
            SslMode::Disable,
            SslMode::Require,
            SslMode::VerifyCa,
            SslMode::VerifyFull,
        ] {
            let connection =
                CdmConnection::new(&format!("{URL}?sslmode={mode}&application_name=x"), None)
                    .expect("the mode reads");
            assert_eq!(mode, connection.ssl_mode());
        }
    }

    #[test]
    fn a_fallback_mode_is_refused_naming_it() {
        for mode in ["prefer", "allow"] {
            let error = CdmConnection::new(&format!("{URL}?sslmode={mode}"), None)
                .expect_err("a fallback mode is refused");
            assert!(
                matches!(&error, ConnectionError::FallbackMode { mode: named } if named == mode),
                "{error:?}"
            );
            assert!(error.to_string().contains(mode), "{error}");
        }
    }

    #[test]
    fn an_unknown_or_repeated_mode_and_another_ssl_parameter_are_refused() {
        let unknown = CdmConnection::new(&format!("{URL}?sslmode=strict"), None);
        assert!(matches!(unknown, Err(ConnectionError::UnknownMode { .. })));
        let repeated = CdmConnection::new(&format!("{URL}?sslmode=require&sslmode=disable"), None);
        assert!(matches!(repeated, Err(ConnectionError::RepeatedMode)));
        let root = CdmConnection::new(&format!("{URL}?sslmode=verify-full&sslrootcert=/ca"), None);
        assert!(
            matches!(&root, Err(ConnectionError::Parameter { name }) if name == "sslrootcert"),
            "{root:?}"
        );
    }

    #[test]
    fn a_ca_that_is_not_a_certificate_or_that_meets_disable_is_refused() {
        let empty = CdmConnection::new(&format!("{URL}?sslmode=verify-full"), Some(b"not pem"));
        assert!(
            matches!(empty, Err(ConnectionError::NoCertificate)),
            "{empty:?}"
        );
        let disabled = CdmConnection::new(&format!("{URL}?sslmode=disable"), Some(b"x"));
        assert!(matches!(disabled, Err(ConnectionError::CaWithoutTls)));
    }

    #[test]
    fn the_debug_form_carries_no_password() {
        let connection =
            CdmConnection::new(&format!("{URL}?sslmode=disable"), None).expect("the URL reads");
        assert!(!format!("{connection:?}").contains("secret"));
    }
}
