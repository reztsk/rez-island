//! Chiffrement des clés API avec DPAPI Windows (lié au compte utilisateur).
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};

const PREFIX: &str = "dpapi:";

fn run(data: &[u8], protect: bool) -> Option<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        let r = if protect {
            CryptProtectData(&input, PCWSTR::null(), None, None, None, 0, &mut out)
        } else {
            CryptUnprotectData(&input, None, None, None, None, 0, &mut out)
        };
        r.ok()?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(out.pbData as _));
        Some(v)
    }
}

pub fn encrypt(plain: &str) -> String {
    if plain.is_empty() {
        return String::new();
    }
    match run(plain.as_bytes(), true) {
        Some(b) => format!("{PREFIX}{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>()),
        None => String::new(),
    }
}

pub fn decrypt(stored: &str) -> String {
    let Some(hex) = stored.strip_prefix(PREFIX) else { return stored.to_string() };
    let bytes: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|i| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()))
        .collect();
    bytes
        .and_then(|b| run(&b, false))
        .and_then(|p| String::from_utf8(p).ok())
        .unwrap_or_default()
}
