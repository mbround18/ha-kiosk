//! Trust-on-first-use certificate capture for Home Assistant instances behind a private CA
//! (e.g. a UniFi/UDM controller's own local CA for LAN-only HTTPS) - see `rest.rs`'s
//! `verify_connection` for how this fits into the overall connection cascade. Portable across
//! networks by design: rather than requiring a build-time `BUNDLED_CA_PATH` baked in for one
//! specific home network, the device captures whatever certificate chain the server actually
//! presents, at runtime, on first connection - then asks a human to confirm it via the on-screen
//! tap-to-trust prompt (`net::ap_portal`'s pending-trust flow) before ever pinning it, since
//! nothing here can verify the cert is legitimate rather than a MITM's own.
//!
//! This performs its own minimal one-shot TLS handshake directly against mbedtls (bypassing
//! esp-tls/esp_http_client entirely) with certificate verification disabled
//! (`MBEDTLS_SSL_VERIFY_NONE`), purely to capture the peer's certificate chain - the connection
//! is torn down immediately afterward and never used to exchange any real data. The captured
//! chain is then handed back to `rest.rs`, which (if the human approves) re-verifies it via a
//! normal `try_connect` pinned to exactly what was captured before treating it as trusted.

use core::ffi::{c_int, c_void};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use esp_idf_svc::sys::{
    mbedtls_ctr_drbg_context, mbedtls_ctr_drbg_free, mbedtls_ctr_drbg_init, mbedtls_ctr_drbg_random,
    mbedtls_ctr_drbg_seed, mbedtls_entropy_context, mbedtls_entropy_free, mbedtls_entropy_func,
    mbedtls_entropy_init, mbedtls_sha256, mbedtls_ssl_close_notify, mbedtls_ssl_conf_authmode,
    mbedtls_ssl_conf_rng, mbedtls_ssl_config, mbedtls_ssl_config_defaults, mbedtls_ssl_config_free,
    mbedtls_ssl_config_init, mbedtls_ssl_context, mbedtls_ssl_free, mbedtls_ssl_get_peer_cert,
    mbedtls_ssl_handshake, mbedtls_ssl_init, mbedtls_ssl_set_bio, mbedtls_ssl_set_hostname,
    mbedtls_ssl_setup, mbedtls_x509_dn_gets, mbedtls_x509_crt, MBEDTLS_ERR_SSL_WANT_READ,
    MBEDTLS_ERR_SSL_WANT_WRITE, MBEDTLS_SSL_IS_CLIENT, MBEDTLS_SSL_PRESET_DEFAULT,
    MBEDTLS_SSL_TRANSPORT_STREAM, MBEDTLS_SSL_VERIFY_NONE,
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Short per-call socket timeout so the blocking handshake loop below can periodically re-check
/// `HANDSHAKE_TIMEOUT` rather than potentially blocking forever on a socket that never responds.
const SOCKET_IO_TIMEOUT: Duration = Duration::from_millis(300);

/// Owns every mbedtls context involved in the probe handshake, each individually heap-allocated
/// (`Box`) so their addresses stay stable regardless of where this guard itself ends up - `ssl`
/// internally stores a pointer to `conf`, which would dangle if `conf` moved after
/// `mbedtls_ssl_setup`. Dropping this frees all of them together, so every early-return path
/// (via `?`) cleans up correctly without duplicated manual teardown.
struct TlsProbe {
    entropy: Box<mbedtls_entropy_context>,
    ctr_drbg: Box<mbedtls_ctr_drbg_context>,
    conf: Box<mbedtls_ssl_config>,
    ssl: Box<mbedtls_ssl_context>,
    // Boxed so its address (passed to mbedtls as the bio context pointer) is stable; owns the
    // socket, closed automatically when this whole guard drops.
    stream: Box<TcpStream>,
}

impl Drop for TlsProbe {
    fn drop(&mut self) {
        unsafe {
            // Best-effort; a probe connection being torn down abruptly is expected and fine.
            let _ = mbedtls_ssl_close_notify(&mut *self.ssl);
            mbedtls_ssl_free(&mut *self.ssl);
            mbedtls_ssl_config_free(&mut *self.conf);
            mbedtls_ctr_drbg_free(&mut *self.ctr_drbg);
            mbedtls_entropy_free(&mut *self.entropy);
        }
    }
}

/// What `fetch_peer_cert` captured, for both pinning (if approved) and showing a human what
/// they'd be trusting.
pub struct CapturedCert {
    /// PEM-encoded trust anchor built from whatever the server presented: certificates marked
    /// as a CA (`X509v3 Basic Constraints: CA:TRUE`) if any are in the chain, so the pin survives
    /// the leaf certificate's own renewal, or the bare leaf certificate itself otherwise
    /// (exact-match pinning - fine for a single self-signed server certificate that never
    /// rotates under a CA).
    pub pem: Vec<u8>,
    /// The leaf certificate's issuer, in printable DN form (e.g. "CN=UniFi SSL Certificate
    /// Authority,O=Ubiquiti Inc."), for display in the on-screen trust prompt.
    pub issuer: String,
    /// SHA-256 fingerprint of the leaf certificate's raw DER bytes, hex-encoded with colon
    /// separators (e.g. "AB:CD:..."), for display in the on-screen trust prompt.
    pub fingerprint_sha256: String,
}

/// Connects to `host:port` and performs a TLS handshake with certificate verification disabled
/// purely to see what the server presents - see the `CapturedCert` fields for what's captured.
pub fn fetch_peer_cert(host: &str, port: u16) -> anyhow::Result<CapturedCert> {
    crate::debug_log::event(format!("tofu: fetch_peer_cert({host}:{port}) entered"));
    let stream = TcpStream::connect((host, port))?;
    stream.set_read_timeout(Some(SOCKET_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(SOCKET_IO_TIMEOUT))?;

    let mut probe = TlsProbe {
        entropy: Box::new(unsafe { std::mem::zeroed() }),
        ctr_drbg: Box::new(unsafe { std::mem::zeroed() }),
        conf: Box::new(unsafe { std::mem::zeroed() }),
        ssl: Box::new(unsafe { std::mem::zeroed() }),
        stream: Box::new(stream),
    };

    unsafe {
        mbedtls_entropy_init(&mut *probe.entropy);
        mbedtls_ctr_drbg_init(&mut *probe.ctr_drbg);
        let seed = b"ha_kiosk_tofu";
        let ret = mbedtls_ctr_drbg_seed(
            &mut *probe.ctr_drbg,
            Some(mbedtls_entropy_func),
            &mut *probe.entropy as *mut _ as *mut c_void,
            seed.as_ptr(),
            seed.len(),
        );
        anyhow::ensure!(ret == 0, "mbedtls_ctr_drbg_seed failed: -0x{:04X}", -ret);

        mbedtls_ssl_config_init(&mut *probe.conf);
        let ret = mbedtls_ssl_config_defaults(
            &mut *probe.conf,
            MBEDTLS_SSL_IS_CLIENT as c_int,
            MBEDTLS_SSL_TRANSPORT_STREAM as c_int,
            MBEDTLS_SSL_PRESET_DEFAULT as c_int,
        );
        anyhow::ensure!(ret == 0, "mbedtls_ssl_config_defaults failed: -0x{:04X}", -ret);
        mbedtls_ssl_conf_authmode(&mut *probe.conf, MBEDTLS_SSL_VERIFY_NONE as c_int);
        mbedtls_ssl_conf_rng(
            &mut *probe.conf,
            Some(mbedtls_ctr_drbg_random),
            &mut *probe.ctr_drbg as *mut _ as *mut c_void,
        );

        mbedtls_ssl_init(&mut *probe.ssl);
        let ret = mbedtls_ssl_setup(&mut *probe.ssl, &*probe.conf);
        anyhow::ensure!(ret == 0, "mbedtls_ssl_setup failed: -0x{:04X}", -ret);

        let hostname = std::ffi::CString::new(host)?;
        mbedtls_ssl_set_hostname(&mut *probe.ssl, hostname.as_ptr());

        let bio_ctx = &mut *probe.stream as *mut TcpStream as *mut c_void;
        mbedtls_ssl_set_bio(&mut *probe.ssl, bio_ctx, Some(bio_send), Some(bio_recv), None);

        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        loop {
            let ret = mbedtls_ssl_handshake(&mut *probe.ssl);
            if ret == 0 {
                break;
            }
            if ret == MBEDTLS_ERR_SSL_WANT_READ || ret == MBEDTLS_ERR_SSL_WANT_WRITE {
                anyhow::ensure!(Instant::now() < deadline, "TLS handshake timed out");
                continue;
            }
            anyhow::bail!("TLS handshake failed: -0x{:04X}", -ret);
        }

        let peer = mbedtls_ssl_get_peer_cert(&*probe.ssl);
        anyhow::ensure!(!peer.is_null(), "server presented no certificate");

        let leaf: &mbedtls_x509_crt = &*peer;
        let leaf_der = std::slice::from_raw_parts(leaf.raw.p, leaf.raw.len);
        let issuer = x509_dn_to_string(&leaf.issuer);
        let fingerprint_sha256 = sha256_fingerprint(leaf_der);

        // Pin the entire chain the server sent (leaf included), not just certificates flagged
        // CA:TRUE. Filtering down to only the CA-flagged entries seemed like the more
        // future-proof choice (survives the leaf's own renewal), but risks pinning an
        // incomplete chain - e.g. if the server's chain is leaf -> intermediate CA -> root CA
        // and the intermediate isn't itself self-signed, mbedtls may still need every
        // presented certificate to complete verification. Pinning everything is strictly safer;
        // a redundant leaf entry in the trusted-CA list is harmless.
        let mut pem = String::new();
        let mut cur = peer;
        while !cur.is_null() {
            let crt: &mbedtls_x509_crt = &*cur;
            let der = std::slice::from_raw_parts(crt.raw.p, crt.raw.len);
            pem.push_str(&der_to_pem(der));
            cur = crt.next;
        }
        anyhow::ensure!(!pem.is_empty(), "no usable certificate in peer chain");
        pem.push('\0');

        Ok(CapturedCert {
            pem: pem.into_bytes(),
            issuer,
            fingerprint_sha256,
        })
    }
}

/// Formats an `mbedtls_x509_name` (e.g. a certificate's issuer) as a printable DN string via
/// `mbedtls_x509_dn_gets`.
unsafe fn x509_dn_to_string(dn: &esp_idf_svc::sys::mbedtls_x509_name) -> String {
    let mut buf = [0u8; 256];
    let len = mbedtls_x509_dn_gets(buf.as_mut_ptr() as *mut _, buf.len(), dn);
    if len <= 0 {
        return String::from("<unknown>");
    }
    String::from_utf8_lossy(&buf[..len as usize]).into_owned()
}

fn sha256_fingerprint(der: &[u8]) -> String {
    let mut digest = [0u8; 32];
    unsafe {
        mbedtls_sha256(der.as_ptr(), der.len(), digest.as_mut_ptr(), 0);
    }
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

unsafe extern "C" fn bio_send(ctx: *mut c_void, buf: *const u8, len: usize) -> c_int {
    let stream = unsafe { &mut *(ctx as *mut TcpStream) };
    let data = unsafe { std::slice::from_raw_parts(buf, len) };
    match stream.write(data) {
        Ok(n) => n as c_int,
        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
            MBEDTLS_ERR_SSL_WANT_WRITE
        }
        Err(_) => -1,
    }
}

unsafe extern "C" fn bio_recv(ctx: *mut c_void, buf: *mut u8, len: usize) -> c_int {
    let stream = unsafe { &mut *(ctx as *mut TcpStream) };
    let data = unsafe { std::slice::from_raw_parts_mut(buf, len) };
    match stream.read(data) {
        Ok(n) => n as c_int,
        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
            MBEDTLS_ERR_SSL_WANT_READ
        }
        Err(_) => -1,
    }
}

fn der_to_pem(der: &[u8]) -> String {
    let b64 = base64_encode(der);
    let mut out = String::with_capacity(der.len() * 2);
    out.push_str("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).expect("base64 output is ASCII"));
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(ALPHABET[(n >> 18 & 0x3F) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6 & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}
