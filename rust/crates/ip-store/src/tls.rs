//! 连数据库的 TLS（backlog D-6 第 4 步）。
//!
//! 照 libpq 的写法，从连接串里读 `sslmode` 和 `sslrootcert`：
//!
//! | `sslmode` | 加密 | 查证书链 | 查主机名 |
//! | --- | --- | --- | --- |
//! | `disable` | 不加密 | — | — |
//! | `prefer`（不写时） | 服务器支持就加密，不支持就明文 | 不查 | 不查 |
//! | `require` | 必须加密 | 不查 | 不查 |
//! | `verify-ca` | 必须加密 | 查 | 不查 |
//! | `verify-full` | 必须加密 | 查 | 查 |
//!
//! - `sslrootcert=<PEM 文件>`：用这个文件里的 CA 查证书链（Supabase、RDS 的证书由它们自己的 CA 签，
//!   要下载它们的 CA 文件）。`sslrootcert=system` 或不写：用系统的根证书（认 `SSL_CERT_FILE`、`SSL_CERT_DIR`）。
//!   和 libpq 不同的地方：libpq 不写时读 `~/.postgresql/root.crt`，这里读系统根证书。
//! - 给了 `sslrootcert` 而 `sslmode` 是 `prefer` 或 `require`：按 `verify-ca` 做。libpq 对 `require` 也是这样；
//!   `prefer` 时 libpq 握手失败会退回明文，这里不退，给了 CA 就是要加密。
//! - `prefer`、`require` 不查证书，只防被动窃听，挡不住冒充服务器。连外部库建议 `verify-full`。
//! - `verify-ca`、`verify-full`、`sslrootcert` 只认 `postgres://` 形式的连接串；`host=... dbname=...`
//!   形式只认 `disable`、`prefer`、`require`。
//! - 不支持客户端证书（`sslcert`、`sslkey`）。

use std::path::PathBuf;
use std::sync::Arc;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio_postgres::config::SslMode;
pub(crate) use tokio_postgres_rustls::MakeRustlsConnect as Tls;

use crate::{Result, StoreError};

/// 连接串里的 `sslmode`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TlsMode {
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RootCert {
    System,
    File(PathBuf),
}

/// 解析连接串，返回 tokio-postgres 的配置、TLS 连接器和最终用的模式。
pub(crate) fn parse(url: &str) -> Result<(tokio_postgres::Config, Tls, TlsMode)> {
    let (rest, mode, root) = split_tls_params(url)?;
    let mut config: tokio_postgres::Config = rest.parse().map_err(|e: tokio_postgres::Error| {
        let is_url = url.starts_with("postgres://") || url.starts_with("postgresql://");
        let hint = if !is_url && (url.contains("verify-") || url.contains("sslrootcert")) {
            "（verify-ca、verify-full、sslrootcert 只认 postgres:// 形式的连接串）"
        } else {
            ""
        };
        // 不带连接串原文：里面有密码。
        StoreError::Config(format!("连接串解析不了：{e}{hint}"))
    })?;
    let mut mode = mode.unwrap_or(match config.get_ssl_mode() {
        SslMode::Disable => TlsMode::Disable,
        SslMode::Require => TlsMode::Require,
        _ => TlsMode::Prefer,
    });
    if root.is_some() && matches!(mode, TlsMode::Prefer | TlsMode::Require) {
        mode = TlsMode::VerifyCa;
    }
    config.ssl_mode(match mode {
        TlsMode::Disable => SslMode::Disable,
        TlsMode::Prefer => SslMode::Prefer,
        _ => SslMode::Require,
    });
    let tls = connector(mode, root.unwrap_or(RootCert::System))?;
    Ok((config, tls, mode))
}

/// 从 `postgres://` 连接串里拿出 `sslmode`、`sslrootcert`（tokio-postgres 不认 `verify-*` 和 `sslrootcert`），
/// 其余参数原样留下。另一种形式原样返回，由 tokio-postgres 解析。
fn split_tls_params(url: &str) -> Result<(String, Option<TlsMode>, Option<RootCert>)> {
    let is_url = url.starts_with("postgres://") || url.starts_with("postgresql://");
    let Some((base, query)) = url.split_once('?').filter(|_| is_url) else {
        return Ok((url.to_string(), None, None));
    };
    let (mut mode, mut root) = (None, None);
    let mut kept = Vec::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "sslmode" => {
                mode = Some(match percent_decode(value)?.as_str() {
                    "disable" => TlsMode::Disable,
                    // allow：libpq 是“先试明文”，这里按 prefer 处理，都是不查证书。
                    "allow" | "prefer" => TlsMode::Prefer,
                    "require" => TlsMode::Require,
                    "verify-ca" => TlsMode::VerifyCa,
                    "verify-full" => TlsMode::VerifyFull,
                    other => {
                        return Err(StoreError::Config(format!(
                            "sslmode={other} 不认识，可选 disable、prefer、require、verify-ca、verify-full"
                        )));
                    }
                });
            }
            "sslrootcert" => {
                let value = percent_decode(value)?;
                root = Some(if value == "system" {
                    RootCert::System
                } else {
                    RootCert::File(PathBuf::from(value))
                });
            }
            _ => kept.push(pair),
        }
    }
    let rest = if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    };
    Ok((rest, mode, root))
}

fn percent_decode(value: &str) -> Result<String> {
    let bad = || StoreError::Config("连接串参数里的 % 转义不对".into());
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = value.get(i + 1..i + 3).ok_or_else(bad)?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| bad())?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| bad())
}

fn connector(mode: TlsMode, root: RootCert) -> Result<Tls> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| StoreError::Config(format!("TLS 配置：{e}")))?;
    let verifier: Arc<dyn ServerCertVerifier> = match mode {
        // disable 时不会用到，给一个不读根证书的，省掉启动时读系统证书。
        TlsMode::Disable | TlsMode::Prefer | TlsMode::Require => Arc::new(NoVerify(provider)),
        TlsMode::VerifyCa | TlsMode::VerifyFull => {
            let full = WebPkiServerVerifier::builder_with_provider(Arc::new(roots(&root)?), provider)
                .build()
                .map_err(|e| StoreError::Config(format!("TLS 根证书：{e}")))?;
            if mode == TlsMode::VerifyFull {
                full
            } else {
                Arc::new(IgnoreHostname(full))
            }
        }
    };
    let mut config = builder
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    // PostgreSQL 17 的 sslnegotiation=direct 要求 ALPN；走普通协商时也不碍事。
    config.alpn_protocols = vec![b"postgresql".to_vec()];
    Ok(Tls::new(config))
}

fn roots(root: &RootCert) -> Result<RootCertStore> {
    let mut store = RootCertStore::empty();
    match root {
        RootCert::File(path) => {
            let certs = CertificateDer::pem_file_iter(path)
                .and_then(|certs| certs.collect::<std::result::Result<Vec<_>, _>>())
                .map_err(|e| StoreError::Config(format!("读不了 sslrootcert {}：{e}", path.display())))?;
            let (added, _) = store.add_parsable_certificates(certs);
            if added == 0 {
                return Err(StoreError::Config(format!(
                    "sslrootcert {} 里没有能用的证书（要 PEM 格式）",
                    path.display()
                )));
            }
        }
        RootCert::System => {
            let loaded = rustls_native_certs::load_native_certs();
            let (added, _) = store.add_parsable_certificates(loaded.certs);
            if added == 0 {
                let errors: Vec<String> = loaded.errors.iter().map(ToString::to_string).collect();
                return Err(StoreError::Config(format!(
                    "读不到系统根证书，用 sslrootcert=<CA 文件> 指定：{}",
                    errors.join("；")
                )));
            }
        }
    }
    Ok(store)
}

/// `prefer`、`require`：不查证书，但握手签名照常验（证明对方持有它出示的证书的私钥）。
#[derive(Debug)]
struct NoVerify(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// `verify-ca`：证书链照常查，只放过主机名不符。rustls 先查链、链通过后才查主机名，
/// 所以拿到“主机名不符”时链已经查过了。
#[derive(Debug)]
struct IgnoreHostname(Arc<WebPkiServerVerifier>);

impl ServerCertVerifier for IgnoreHostname {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        match self
            .0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
        {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests;
