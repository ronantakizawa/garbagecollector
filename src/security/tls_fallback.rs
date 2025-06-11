// src/security/tls_fallback.rs - Fallback TLS types when TLS feature is disabled

use anyhow::Result;

/// Fallback Certificate type when TLS feature is disabled
#[derive(Debug, Clone)]
pub struct Certificate {
    _data: Vec<u8>,
}

impl Certificate {
    pub fn from_pem(pem: &[u8]) -> Self {
        Self {
            _data: pem.to_vec(),
        }
    }
}

/// Fallback Identity type when TLS feature is disabled
#[derive(Debug, Clone)]
pub struct Identity {
    _cert: Vec<u8>,
    _key: Vec<u8>,
}

impl Identity {
    pub fn from_pem(cert: &[u8], key: &[u8]) -> Self {
        Self {
            _cert: cert.to_vec(),
            _key: key.to_vec(),
        }
    }
}

/// Fallback ServerTlsConfig type when TLS feature is disabled
#[derive(Debug, Clone)]
pub struct ServerTlsConfig {
    _identity: Option<Identity>,
    _client_ca_root: Option<Certificate>,
}

impl ServerTlsConfig {
    pub fn new() -> Self {
        Self {
            _identity: None,
            _client_ca_root: None,
        }
    }

    pub fn identity(mut self, identity: Identity) -> Self {
        self._identity = Some(identity);
        self
    }

    pub fn client_ca_root(mut self, cert: Certificate) -> Self {
        self._client_ca_root = Some(cert);
        self
    }
}

/// Fallback ClientTlsConfig type when TLS feature is disabled
#[derive(Debug, Clone)]
pub struct ClientTlsConfig {
    _domain: String,
    _ca_certificate: Option<Certificate>,
    _identity: Option<Identity>,
}

impl ClientTlsConfig {
    pub fn new() -> Self {
        Self {
            _domain: String::new(),
            _ca_certificate: None,
            _identity: None,
        }
    }

    pub fn domain_name<T: Into<String>>(mut self, domain: T) -> Self {
        self._domain = domain.into();
        self
    }

    pub fn ca_certificate(mut self, cert: Certificate) -> Self {
        self._ca_certificate = Some(cert);
        self
    }

    pub fn identity(mut self, identity: Identity) -> Self {
        self._identity = Some(identity);
        self
    }
}