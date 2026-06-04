//! Wire protocol and payload utility functions
//!
//! Provides parsing and extraction functions for low-level TLS (SNI/ClientHello)
//! and HTTP (Host headers) protocols to assist routing and address translation.

use anyhow::{Result, anyhow};
use httparse::{EMPTY_HEADER, Request, Status};
use tls_parser::{TlsClientHelloContents, TlsExtension, TlsMessage, TlsMessageHandshake};

#[must_use]
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
        Err(e) => Err(anyhow!("Failed to parse TLS record: {e}")),
    }
}

fn extract_sni_from_hello(hello: &TlsClientHelloContents) -> Result<String> {
    if let Some(extensions) = hello.ext {
        let (_, extensions_list) = tls_parser::parse_tls_extensions(extensions)
            .map_err(|e| anyhow!("Failed to parse TLS extensions: {e}"))?;
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
    let mut headers = [EMPTY_HEADER; 64];
    let mut req = Request::new(&mut headers);
    match req.parse(buf) {
        Ok(Status::Complete(_) | Status::Partial) => {
            for header in req.headers {
                if header.name.eq_ignore_ascii_case("Host") {
                    return Ok(String::from_utf8_lossy(header.value).to_string());
                }
            }
            Err(anyhow!("Host header not found"))
        }
        Err(e) => Err(anyhow!("Failed to parse HTTP request: {e}")),
    }
}

/// Parses the host header or SNI into `(service_id, interface_hash)`.
/// E.g. `nickname-p<pubkeyhash>-i<interfacehash>.syneroym.io`
/// -> `("nickname-p<pubkeyhash>", "<interfacehash>")`.
pub fn parse_target_host(host: &str) -> Option<(String, String)> {
    let mut host_str = host;
    if let Some((h, p)) = host_str.rsplit_once(':')
        && !p.is_empty()
        && p.chars().all(|c| c.is_ascii_digit())
    {
        host_str = h;
    }

    let host_base = host_str.strip_suffix(".localhost").unwrap_or(host_str);
    let subdomain = host_base.split('.').next().unwrap_or(host_base);

    if subdomain == "localhost" || subdomain == "127" {
        return None;
    }

    let mut parts: Vec<&str> = subdomain.split('-').collect();

    let mut interfacehash = None;
    if let Some(last) = parts.last()
        && last.starts_with('i')
        && last.len() > 1
    {
        interfacehash = Some(&last[1..]);
        parts.pop();
    }

    let mut pubkeyhash = None;
    if let Some(last) = parts.last()
        && last.starts_with('p')
        && last.len() > 1
    {
        pubkeyhash = Some(&last[1..]);
        parts.pop();
    }

    let nickname = if parts.is_empty() { None } else { Some(parts.join("-")) };

    let lookup_alias = if let Some(n) = nickname {
        format!("{n}-p{}", pubkeyhash.unwrap_or_default())
    } else {
        format!("p{}", pubkeyhash.unwrap_or_default())
    };

    let interface = interfacehash.unwrap_or("").to_string();

    Some((lookup_alias, interface))
}
