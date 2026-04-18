//! TLS configuration for the server.
//!
//! Two constructors:
//! - [`TlsConfig::from_pem_files`] — load a cert chain and private key from PEM files.
//!   Accepts PKCS#8, RSA (PKCS#1), and SEC1 (EC) private keys.
//! - [`TlsConfig::from_rustls`] — wrap a pre-built [`rustls::ServerConfig`].
//!
//! Both constructors set ALPN to `["http/1.1"]` — v0.2 is HTTP/1-only.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig as RustlsServerConfig;

/// TLS configuration passed to [`ServerConfig::tls`](crate::ServerConfig).
#[derive(Clone)]
pub struct TlsConfig(pub(crate) Arc<RustlsServerConfig>);

impl TlsConfig {
    /// Load a certificate chain and private key from PEM files.
    ///
    /// The key file may contain a PKCS#8, RSA (PKCS#1), or SEC1 (EC) private key.
    /// The first supported private-key item in the file is used.
    pub fn from_pem_files(
        cert_path: impl AsRef<Path>,
        key_path: impl AsRef<Path>,
    ) -> Result<Self, TlsError> {
        let cert_chain = load_certs(cert_path.as_ref())?;
        let key = load_private_key(key_path.as_ref())?;

        let mut config = RustlsServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, key)
            .map_err(TlsError::InvalidCertOrKey)?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];

        Ok(Self(Arc::new(config)))
    }

    /// Wrap a pre-built rustls [`ServerConfig`].
    ///
    /// ALPN is forced to `["http/1.1"]` — v0.2 is HTTP/1-only. Any ALPN
    /// value the caller set on `config` is overwritten.
    pub fn from_rustls(config: Arc<RustlsServerConfig>) -> Self {
        // Mutate the inner ALPN to guarantee http/1.1 — take a mut reference
        // through Arc::make_mut only if needed, else clone-mutate-rewrap.
        let config = match Arc::try_unwrap(config) {
            Ok(mut cfg) => {
                cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
                Arc::new(cfg)
            }
            Err(arc) => {
                let mut cfg = (*arc).clone();
                cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
                Arc::new(cfg)
            }
        };
        Self(config)
    }

    /// Access the inner rustls config.
    pub(crate) fn inner(&self) -> Arc<RustlsServerConfig> {
        self.0.clone()
    }

    #[cfg(test)]
    pub(crate) fn alpn_protocols(&self) -> &[Vec<u8>] {
        &self.0.alpn_protocols
    }
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let file = File::open(path).map_err(TlsError::Io)?;
    let mut reader = BufReader::new(file);
    let mut certs = Vec::new();
    for item in rustls_pemfile::certs(&mut reader) {
        certs.push(item.map_err(TlsError::Io)?);
    }
    if certs.is_empty() {
        return Err(TlsError::NoCertificates);
    }
    Ok(certs)
}

fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>, TlsError> {
    let file = File::open(path).map_err(TlsError::Io)?;
    let mut reader = BufReader::new(file);
    let mut saw_unsupported = false;
    for item in rustls_pemfile::read_all(&mut reader) {
        let item = item.map_err(TlsError::Io)?;
        match item {
            rustls_pemfile::Item::Pkcs8Key(key) => return Ok(PrivateKeyDer::Pkcs8(key)),
            rustls_pemfile::Item::Pkcs1Key(key) => return Ok(PrivateKeyDer::Pkcs1(key)),
            rustls_pemfile::Item::Sec1Key(key) => return Ok(PrivateKeyDer::Sec1(key)),
            rustls_pemfile::Item::X509Certificate(_) | rustls_pemfile::Item::Crl(_) => continue,
            _ => {
                saw_unsupported = true;
            }
        }
    }
    if saw_unsupported {
        Err(TlsError::UnsupportedKeyFormat)
    } else {
        Err(TlsError::NoPrivateKey)
    }
}

/// Errors produced while loading or building a [`TlsConfig`].
#[derive(Debug)]
pub enum TlsError {
    /// The cert file contained no PEM-encoded certificates.
    NoCertificates,
    /// The key file contained no private-key items of any kind.
    NoPrivateKey,
    /// The key file contained a private-key item in an unrecognized format.
    UnsupportedKeyFormat,
    /// rustls rejected the supplied certificate/key pair.
    InvalidCertOrKey(rustls::Error),
    /// I/O or PEM-parse error reading the cert/key file.
    Io(std::io::Error),
}

impl std::fmt::Display for TlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCertificates => write!(f, "no certificates found in cert file"),
            Self::NoPrivateKey => write!(f, "no private key found in key file"),
            Self::UnsupportedKeyFormat => {
                write!(f, "private key file contains an unsupported key format")
            }
            Self::InvalidCertOrKey(err) => write!(f, "invalid certificate or key: {err}"),
            Self::Io(err) => write!(f, "I/O error: {err}"),
        }
    }
}

impl std::error::Error for TlsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidCertOrKey(err) => Some(err),
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn generate_self_signed_pem() -> (tempfile::NamedTempFile, tempfile::NamedTempFile) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("rcgen should succeed");
        let mut cert_file = tempfile::NamedTempFile::new().expect("tempfile");
        cert_file
            .write_all(cert.cert.pem().as_bytes())
            .expect("write cert");
        let mut key_file = tempfile::NamedTempFile::new().expect("tempfile");
        key_file
            .write_all(cert.key_pair.serialize_pem().as_bytes())
            .expect("write key");
        (cert_file, key_file)
    }

    #[test]
    fn from_pem_files_forces_http1_alpn() {
        let (cert_file, key_file) = generate_self_signed_pem();
        let tls =
            TlsConfig::from_pem_files(cert_file.path(), key_file.path()).expect("load pem");
        assert_eq!(tls.alpn_protocols(), &[b"http/1.1".to_vec()]);
    }

    #[test]
    fn from_rustls_overwrites_alpn() {
        let (cert_file, key_file) = generate_self_signed_pem();
        let certs = load_certs(cert_file.path()).expect("certs");
        let key = load_private_key(key_file.path()).expect("key");
        let mut cfg = RustlsServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .expect("rustls cfg");
        // Caller sets ALPN that should be overwritten.
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        let tls = TlsConfig::from_rustls(Arc::new(cfg));
        assert_eq!(tls.alpn_protocols(), &[b"http/1.1".to_vec()]);
    }

    #[test]
    fn missing_cert_returns_no_certificates() {
        let mut key_file = tempfile::NamedTempFile::new().expect("tempfile");
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("rcgen should succeed");
        key_file
            .write_all(cert.key_pair.serialize_pem().as_bytes())
            .expect("write key");
        let empty_cert = tempfile::NamedTempFile::new().expect("tempfile");
        match TlsConfig::from_pem_files(empty_cert.path(), key_file.path()) {
            Err(TlsError::NoCertificates) => {}
            Err(other) => panic!("expected NoCertificates, got {other}"),
            Ok(_) => panic!("expected error"),
        }
    }

    #[test]
    fn missing_private_key_returns_no_private_key() {
        let (cert_file, _) = generate_self_signed_pem();
        let empty_key = tempfile::NamedTempFile::new().expect("tempfile");
        match TlsConfig::from_pem_files(cert_file.path(), empty_key.path()) {
            Err(TlsError::NoPrivateKey) => {}
            Err(other) => panic!("expected NoPrivateKey, got {other}"),
            Ok(_) => panic!("expected error"),
        }
    }
}
