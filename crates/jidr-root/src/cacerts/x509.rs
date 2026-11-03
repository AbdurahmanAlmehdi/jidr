//! Just enough X.509 to file a CA certificate the way Android's store does:
//! PEM or DER in, the DER bytes, the subject (for people) and the file name.
//!
//! Android names each store file `<subject_hash_old>.<n>`: OpenSSL's old
//! subject hash, the first four bytes of the MD5 of the DER-encoded subject
//! name read as a little-endian number, in eight hex digits. `n` counts
//! certificates whose subjects hash the same (a re-generated Burp CA keeps
//! its subject, so its hash).

use std::ops::Range;

use base64::Engine;
use jidr_core::{JidrError, JidrResult};
use md5::{Digest, Md5};
use sha2::Sha256;

/// A parsed certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaCert {
    pub der: Vec<u8>,
    /// `subject_hash_old`, e.g. `a4ac454c`.
    pub subject_hash: String,
    /// The common name, else the organisation, else the hash.
    pub subject: String,
    /// SHA-256 of the DER bytes, lowercase hex.
    pub sha256: String,
}

const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
const END: &str = "-----END CERTIFICATE-----";

const SEQUENCE: u8 = 0x30;
const SET: u8 = 0x31;
const OID: u8 = 0x06;
/// `id-at-commonName` (2.5.4.3) and `id-at-organizationName` (2.5.4.10).
const CN: &[u8] = &[0x55, 0x04, 0x03];
const O: &[u8] = &[0x55, 0x04, 0x0a];

fn not_a_cert(why: &str) -> JidrError {
    JidrError::usage(format!("not a PEM or DER X.509 certificate: {why}"))
}

/// One DER element: its tag, where it starts (the tag byte), its content.
#[derive(Debug, Clone)]
struct Element {
    tag: u8,
    start: usize,
    content: Range<usize>,
}

impl Element {
    fn end(&self) -> usize {
        self.content.end
    }
}

/// The element at `pos`.
fn element(buf: &[u8], pos: usize) -> Option<Element> {
    let tag = *buf.get(pos)?;
    let first = *buf.get(pos + 1)? as usize;
    let (len, header) = if first < 0x80 {
        (first, 2)
    } else {
        let n = first & 0x7f;
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | *buf.get(pos + 2 + i)? as usize;
        }
        (len, 2 + n)
    };
    let start = pos + header;
    let end = start.checked_add(len)?;
    (end <= buf.len()).then_some(Element {
        tag,
        start: pos,
        content: start..end,
    })
}

/// The elements inside `parent`.
fn children(buf: &[u8], parent: &Element) -> Option<Vec<Element>> {
    let mut out = Vec::new();
    let mut pos = parent.content.start;
    while pos < parent.content.end {
        let el = element(buf, pos)?;
        pos = el.end();
        out.push(el);
    }
    (pos == parent.content.end).then_some(out)
}

/// The DER of the one certificate in a PEM file.
fn pem_der(text: &str) -> JidrResult<Vec<u8>> {
    let count = text.matches(BEGIN).count();
    if count > 1 {
        return Err(JidrError::usage(format!(
            "the file holds {count} certificates; give one CA certificate per file"
        )));
    }
    let body = text
        .split_once(BEGIN)
        .and_then(|(_, rest)| rest.split_once(END))
        .map(|(body, _)| body)
        .ok_or_else(|| not_a_cert("no CERTIFICATE block"))?;
    let b64: String = body.split_whitespace().collect();
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| not_a_cert(&format!("bad base64 ({e})")))
}

/// The certificate's subject name.
fn subject(der: &[u8]) -> Option<Element> {
    let cert = element(der, 0)?;
    if cert.tag != SEQUENCE || cert.end() != der.len() {
        return None;
    }
    let tbs = element(der, cert.content.start)?;
    if tbs.tag != SEQUENCE {
        return None;
    }
    let fields = children(der, &tbs)?;
    // version [0] is optional; then serial, signature, issuer, validity, subject.
    let skip = usize::from(fields.first()?.tag == 0xa0);
    fields
        .get(skip + 4)
        .filter(|name| name.tag == SEQUENCE)
        .cloned()
}

/// The value of the first attribute with `oid` in a Name, as text.
fn attribute(der: &[u8], name: &Element, oid: &[u8]) -> Option<String> {
    for rdn in children(der, name)?.iter().filter(|e| e.tag == SET) {
        for atv in children(der, rdn)?.iter().filter(|e| e.tag == SEQUENCE) {
            let parts = children(der, atv)?;
            let (key, value) = (parts.first()?, parts.get(1)?);
            if key.tag == OID && der[key.content.clone()] == *oid {
                return std::str::from_utf8(&der[value.content.clone()])
                    .ok()
                    .map(str::to_string);
            }
        }
    }
    None
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parses a PEM or DER certificate.
pub fn parse(bytes: &[u8]) -> JidrResult<CaCert> {
    let der = match std::str::from_utf8(bytes) {
        Ok(text) if text.contains("-----BEGIN") => pem_der(text)?,
        _ => bytes.to_vec(),
    };
    let name = subject(&der).ok_or_else(|| not_a_cert("no subject name"))?;
    // The hash covers the whole encoding: tag, length and content.
    let md5 = Md5::digest(&der[name.start..name.end()]);
    let hash = u32::from_le_bytes([md5[0], md5[1], md5[2], md5[3]]);
    let subject_hash = format!("{hash:08x}");
    let subject = attribute(&der, &name, CN)
        .or_else(|| attribute(&der, &name, O))
        .unwrap_or_else(|| subject_hash.clone());
    Ok(CaCert {
        sha256: hex(&Sha256::digest(&der)),
        subject_hash,
        subject,
        der,
    })
}

/// The certificate as PEM, the form Android's store files use.
pub fn to_pem(der: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = format!("{BEGIN}\n");
    for line in b64.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(line));
        out.push('\n');
    }
    out.push_str(END);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEM: &[u8] = include_bytes!("../../tests/fixtures/jidr-test-ca.pem");
    const DER: &[u8] = include_bytes!("../../tests/fixtures/jidr-test-ca.der");
    const RENEWED: &[u8] = include_bytes!("../../tests/fixtures/jidr-test-ca-renewed.pem");
    const EC: &[u8] = include_bytes!("../../tests/fixtures/jidr-test-ca-ec.pem");

    /// Expected values from `openssl x509 -noout -subject_hash_old` and
    /// `openssl x509 -outform der | shasum -a 256`.
    #[test]
    fn names_certificates_like_openssl() {
        let c = parse(PEM).unwrap();
        assert_eq!(c.subject_hash, "a4ac454c");
        assert_eq!(c.subject, "Jidr Test CA");
        assert_eq!(
            c.sha256,
            "3fe6832f2aaf0c8332457615a00913826ea8748fe164e164417e525e36db1cd2"
        );
        let ec = parse(EC).unwrap();
        assert_eq!(ec.subject_hash, "f7ecd4f3");
        assert_eq!(ec.subject, "Jidr Org Only", "no CN: the organisation");
    }

    #[test]
    fn der_and_pem_are_the_same_certificate() {
        let (pem, der) = (parse(PEM).unwrap(), parse(DER).unwrap());
        assert_eq!(pem, der);
        assert_eq!(parse(to_pem(&der.der).as_bytes()).unwrap(), der);
    }

    #[test]
    fn a_renewed_ca_keeps_its_subject_hash() {
        let (old, new) = (parse(PEM).unwrap(), parse(RENEWED).unwrap());
        assert_eq!(old.subject_hash, new.subject_hash);
        assert_ne!(old.sha256, new.sha256);
    }

    #[test]
    fn refuses_what_isnt_one_certificate() {
        for bad in [
            &b""[..],
            b"hello",
            b"-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----\n",
            b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
            &DER[..DER.len() - 1],
        ] {
            let e = parse(bad).unwrap_err();
            assert_eq!(e.code, jidr_core::ErrorCode::Usage, "{e}");
        }
        let two = [PEM, RENEWED].concat();
        assert!(parse(&two).unwrap_err().message.contains("2 certificates"));
    }
}
