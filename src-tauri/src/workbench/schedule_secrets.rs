//! Do not inherit the legacy vault's plaintext fallback. Fixed inputs are secrets.
use nuphus_workbench::{ApiError, Result};

fn failed() -> ApiError {
    ApiError::new(
        "schedule_secret_unavailable",
        "Cannot protect or read fixed inputs on this machine; re-enter them in the schedule editor",
    )
}

#[cfg(windows)]
pub fn seal(raw: &str) -> Result<String> {
    let sealed = nuphus::cookies::encrypt_secret(raw);
    if !sealed.starts_with("enc:v1:")
        || nuphus::cookies::decrypt_secret(&sealed).as_deref() != Some(raw)
    {
        return Err(failed());
    }
    Ok(sealed)
}

#[cfg(windows)]
pub fn open(sealed: &str) -> Result<String> {
    if !sealed.starts_with("enc:v1:") {
        return Err(failed());
    }
    nuphus::cookies::decrypt_secret(sealed).ok_or_else(failed)
}

// The key stays in the application profile (0600), never in exported project data.
// This protects project copies, not against another process running as the same OS user.
#[cfg(not(windows))]
fn key(create: bool) -> Result<ring::aead::LessSafeKey> {
    use ring::rand::SecureRandom;
    use std::io::{Read, Write};
    use std::os::unix::fs::OpenOptionsExt;
    let path = nuphus::profile::workbench_data_dir().join("schedule-inputs.key");
    let mut bytes = [0u8; 32];
    match std::fs::File::open(&path) {
        Ok(mut file) => {
            file.read_exact(&mut bytes).map_err(|_| failed())?;
        }
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
            ring::rand::SystemRandom::new()
                .fill(&mut bytes)
                .map_err(|_| failed())?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|_| failed())?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| failed())?;
        }
        Err(_) => return Err(failed()),
    }
    Ok(ring::aead::LessSafeKey::new(
        ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, &bytes).map_err(|_| failed())?,
    ))
}

#[cfg(not(windows))]
pub fn seal(raw: &str) -> Result<String> {
    use base64::Engine;
    use ring::{aead, rand::SecureRandom};
    let mut nonce = [0; 12];
    ring::rand::SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| failed())?;
    let mut body = raw.as_bytes().to_vec();
    key(true)?
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(b"workbench-schedule-v1"),
            &mut body,
        )
        .map_err(|_| failed())?;
    let mut data = nonce.to_vec();
    data.extend(body);
    Ok(format!(
        "wbs:v1:{}",
        base64::engine::general_purpose::STANDARD.encode(data)
    ))
}

#[cfg(not(windows))]
pub fn open(sealed: &str) -> Result<String> {
    use base64::Engine;
    use ring::aead;
    let encoded = sealed.strip_prefix("wbs:v1:").ok_or_else(failed)?;
    let mut data = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| failed())?;
    if data.len() < 28 {
        return Err(failed());
    }
    let nonce: [u8; 12] = data[..12].try_into().map_err(|_| failed())?;
    let plain = key(false)?
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(b"workbench-schedule-v1"),
            &mut data[12..],
        )
        .map_err(|_| failed())?;
    String::from_utf8(plain.to_vec()).map_err(|_| failed())
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn fixed_inputs_are_encrypted_and_plaintext_is_rejected() {
        let value = "{\"token\":\"test-only-fixed-input\"}";
        let sealed = super::seal(value).unwrap();
        assert!(!sealed.contains("test-only-fixed-input"));
        assert_eq!(super::open(&sealed).unwrap(), value);
        assert!(super::open(value).is_err());
        assert!(super::open("enc:v1:broken").is_err());
    }
}
