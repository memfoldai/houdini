use security_framework::passwords::{get_generic_password, set_generic_password};
use security_framework_sys::base::errSecItemNotFound;
use serde::{Deserialize, Serialize};

const SERVICE: &str = "ai.memfold.houdini";

/// One item holds every secret. macOS prompts once per keychain item on first
/// access, so a single item means a single prompt, where two items meant two.
const SECRETS_ACCOUNT: &str = "secrets";

/// The pre-0.7.3 layout kept these as two separate items. They are read once to
/// migrate into the combined item and then left in place: never rewritten, so a
/// downgrade still finds them, and never re-read, so they never prompt again.
const LEGACY_DB_ACCOUNT: &str = "db-encryption-key";
const LEGACY_ANALYTICS_ACCOUNT: &str = "analytics-api-key";

#[derive(Serialize, Deserialize)]
struct StoredSecrets {
    db_key_hex: String,
    #[serde(default)]
    analytics_key: Option<String>,
}

pub struct Secrets {
    pub db_key: [u8; 32],
    pub analytics_key: Option<String>,
}

/// Reads every secret in one keychain access.
///
/// The database key is never regenerated except on a genuinely first run
/// (`errSecItemNotFound` on both the combined and the legacy item). Any other
/// read failure aborts, because minting a fresh key would silently make the
/// existing encrypted database unreadable.
pub fn load() -> Result<Secrets, String> {
    match get_generic_password(SERVICE, SECRETS_ACCOUNT) {
        Ok(bytes) => decode(&bytes),
        Err(e) if e.code() == errSecItemNotFound => migrate_or_init(),
        Err(e) => Err(format!(
            "keychain: secrets read failed (OSStatus {}); refusing to replace a key that would make the database unreadable",
            e.code()
        )),
    }
}

pub fn set_analytics_key(key: &str) -> Result<(), String> {
    let mut secrets = load()?;
    let key = key.trim();
    secrets.analytics_key = if key.is_empty() {
        None
    } else {
        Some(key.to_string())
    };
    persist(&secrets)
}

fn migrate_or_init() -> Result<Secrets, String> {
    let db_key = match get_generic_password(SERVICE, LEGACY_DB_ACCOUNT) {
        Ok(bytes) => <[u8; 32]>::try_from(bytes.as_slice())
            .map_err(|_| "keychain: stored key is not 32 bytes".to_string())?,
        Err(e) if e.code() == errSecItemNotFound => random_key(),
        Err(e) => {
            return Err(format!(
                "keychain: key read failed (OSStatus {}); refusing to create a replacement key that would make the existing database unreadable",
                e.code()
            ));
        }
    };
    let secrets = Secrets {
        db_key,
        analytics_key: legacy_analytics_key(),
    };
    persist(&secrets)?;
    Ok(secrets)
}

fn legacy_analytics_key() -> Option<String> {
    let raw = match get_generic_password(SERVICE, LEGACY_ANALYTICS_ACCOUNT) {
        Ok(bytes) => String::from_utf8(bytes).ok(),
        Err(_) => option_env!("HOUDINI_ANALYTICS_KEY").map(str::to_string),
    };
    usable_key(raw)
}

fn usable_key(value: Option<String>) -> Option<String> {
    value
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

fn decode(bytes: &[u8]) -> Result<Secrets, String> {
    let stored: StoredSecrets =
        serde_json::from_slice(bytes).map_err(|e| format!("keychain: secrets are corrupt: {e}"))?;
    let raw = decode_hex(&stored.db_key_hex)
        .ok_or_else(|| "keychain: stored key is not valid hex".to_string())?;
    let db_key = <[u8; 32]>::try_from(raw.as_slice())
        .map_err(|_| "keychain: stored key is not 32 bytes".to_string())?;
    Ok(Secrets {
        db_key,
        analytics_key: usable_key(stored.analytics_key),
    })
}

fn persist(secrets: &Secrets) -> Result<(), String> {
    let stored = StoredSecrets {
        db_key_hex: encode_hex(&secrets.db_key),
        analytics_key: secrets.analytics_key.clone(),
    };
    let json = serde_json::to_vec(&stored)
        .map_err(|e| format!("keychain: could not encode secrets: {e}"))?;
    set_generic_password(SERVICE, SECRETS_ACCOUNT, &json)
        .map_err(|e| format!("keychain: could not store secrets: {e}"))
}

fn random_key() -> [u8; 32] {
    use rand::RngCore;
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_a_key() {
        let key = random_key();
        let encoded = encode_hex(&key);
        assert_eq!(encoded.len(), 64);
        assert_eq!(decode_hex(&encoded).unwrap(), key);
    }

    #[test]
    fn decode_rejects_a_short_or_malformed_key() {
        assert!(decode_hex("abc").is_none());
        assert!(decode_hex("zz").is_none());
        let short = serde_json::to_vec(&StoredSecrets {
            db_key_hex: "00ff".to_string(),
            analytics_key: None,
        })
        .unwrap();
        assert!(decode(&short).is_err(), "a key that is not 32 bytes is refused");
    }

    #[test]
    fn a_stored_blob_round_trips_both_secrets() {
        let key = random_key();
        let blob = serde_json::to_vec(&StoredSecrets {
            db_key_hex: encode_hex(&key),
            analytics_key: Some("  sk-abc  ".to_string()),
        })
        .unwrap();
        let secrets = decode(&blob).unwrap();
        assert_eq!(secrets.db_key, key);
        assert_eq!(secrets.analytics_key.as_deref(), Some("sk-abc"), "trimmed");
    }

    #[test]
    fn an_empty_analytics_key_reads_as_absent() {
        let blob = serde_json::to_vec(&StoredSecrets {
            db_key_hex: encode_hex(&random_key()),
            analytics_key: Some("   ".to_string()),
        })
        .unwrap();
        assert!(decode(&blob).unwrap().analytics_key.is_none());
    }

    #[test]
    fn a_blob_without_an_analytics_field_still_loads() {
        let blob = format!("{{\"db_key_hex\":\"{}\"}}", encode_hex(&random_key()));
        let secrets = decode(blob.as_bytes()).unwrap();
        assert!(secrets.analytics_key.is_none());
    }
}
