// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The credentials of each upstream, every secret read once from its value
//! or its `_file` sibling.

use secrecy::SecretString;
use std::path::Path;

use super::Error;
use super::section::Cdm;
use super::section::Credentials;

/// The authentication scheme a credentials section resolves to.
pub(super) enum Scheme {
    /// An RFC 6750 bearer token.
    Bearer(SecretString),
    /// RFC 7617 basic authentication.
    Basic {
        /// The user name, which is not a secret.
        user: String,
        /// The password.
        password: SecretString,
    },
}

/// Returns the scheme `credentials` describes, or `None` when it names none.
pub(super) fn resolve_credentials(
    section: &str,
    credentials: &Credentials,
) -> Result<Option<Scheme>, Error> {
    let token = secret(
        &format!("{section}.bearer_token"),
        credentials.bearer_token.as_deref(),
        credentials.bearer_token_file.as_deref(),
    )?;
    let password = secret(
        &format!("{section}.password"),
        credentials.password.as_deref(),
        credentials.password_file.as_deref(),
    )?;
    match (token, credentials.user.as_deref(), password) {
        (Some(_), Some(_), _) | (Some(_), None, Some(_)) => Err(Error::Scheme {
            section: section.to_owned(),
        }),
        (Some(token), None, None) => Ok(Some(Scheme::Bearer(token))),
        (None, Some(user), Some(password)) => Ok(Some(Scheme::Basic {
            user: user.to_owned(),
            password,
        })),
        (None, Some(_), None) => Err(Error::Missing {
            key: format!("{section}.password"),
        }),
        (None, None, Some(_)) => Err(Error::Missing {
            key: format!("{section}.user"),
        }),
        (None, None, None) => Ok(None),
    }
}

/// Returns the key whose value `error` refuses: the CA's for a CA fault, the
/// URL's for every other.
pub(super) fn cdm_connection_key(
    error: &omop_cdm::connection::ConnectionError,
    cdm: &Cdm,
) -> &'static str {
    use omop_cdm::connection::ConnectionError;
    match error {
        ConnectionError::NoCertificate
        | ConnectionError::Pem { .. }
        | ConnectionError::Root { .. }
        | ConnectionError::CaWithoutTls => {
            if cdm.tls_ca_file.is_some() {
                "cdm.tls_ca_file"
            } else {
                "cdm.tls_ca"
            }
        }
        _ => {
            if cdm.url_file.is_some() {
                "cdm.url_file"
            } else {
                "cdm.url"
            }
        }
    }
}

/// Returns the secret `key` names, inline or from its `_file` sibling.
pub(super) fn secret(
    key: &str,
    inline: Option<&str>,
    file: Option<&Path>,
) -> Result<Option<SecretString>, Error> {
    match (inline, file) {
        (Some(_), Some(_)) => Err(Error::Conflict {
            key: key.to_owned(),
        }),
        (Some(value), None) => Ok(Some(SecretString::from(value))),
        (None, Some(path)) => {
            let text = std::fs::read_to_string(path).map_err(|source| Error::Secret {
                key: format!("{key}_file"),
                path: path.to_path_buf(),
                source,
            })?;
            Ok(Some(SecretString::from(text.trim())))
        }
        (None, None) => Ok(None),
    }
}
