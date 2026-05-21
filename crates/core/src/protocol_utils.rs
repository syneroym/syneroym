//! Wire protocol and payload utility functions
//!
//! Provides parsing and extraction functions for low-level TLS (SNI/ClientHello)
//! and HTTP (Host headers) protocols to assist routing and address translation.

use anyhow::{Result, anyhow};
use tls_parser::{TlsClientHelloContents, TlsExtension, TlsMessage, TlsMessageHandshake};

pub fn is_tls_client_hello(buf: &[u8]) -> bool {
    if buf.len() < 5 {
        return false;
    }
    // TLS record header: content type 22 (handshake), version (3, x), length
    buf[0] == 0x16 && buf[1] == 0x03
}

pub fn extract_sni(buf: &[u8]) -> Result<String> {
    match tls_parser::parse_tls_plaintext(buf) {
        Ok((_, record)) => {
            for msg in record.msg {
                if let TlsMessage::Handshake(TlsMessageHandshake::ClientHello(hello)) = msg {
                    return extract_sni_from_hello(&hello);
                }
            }
            Err(anyhow!("No ClientHello found in TLS record"))
        }
        Err(e) => Err(anyhow!("Failed to parse TLS record: {}", e)),
    }
}

fn extract_sni_from_hello(hello: &TlsClientHelloContents) -> Result<String> {
    if let Some(extensions) = hello.ext {
        let (_, extensions_list) = tls_parser::parse_tls_extensions(extensions)
            .map_err(|e| anyhow!("Failed to parse TLS extensions: {}", e))?;
        for ext in extensions_list {
            if let TlsExtension::SNI(sni) = ext
                && let Some((_, name)) = sni.into_iter().next()
            {
                return Ok(String::from_utf8_lossy(name).to_string());
            }
        }
    }
    Err(anyhow!("No SNI extension found in ClientHello"))
}

pub fn extract_host_from_http(buf: &[u8]) -> Result<String> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);
    match req.parse(buf) {
        Ok(httparse::Status::Complete(_)) | Ok(httparse::Status::Partial) => {
            for header in req.headers {
                if header.name.eq_ignore_ascii_case("Host") {
                    return Ok(String::from_utf8_lossy(header.value).to_string());
                }
            }
            Err(anyhow!("Host header not found"))
        }
        Err(e) => Err(anyhow!("Failed to parse HTTP request: {}", e)),
    }
}

pub fn extract_service_from_host(host: &str) -> Result<String> {
    let hostname = host.split(':').next().unwrap_or(host);

    if hostname == "localhost" || hostname == "127.0.0.1" {
        return Ok(hostname.to_string());
    }

    // Expected format: <alias-servicehash-interfacehash>.<domain>
    match hostname.split_once('.') {
        Some((service, _)) => Ok(service.to_string()),
        None => Ok(hostname.to_string()),
    }
}
