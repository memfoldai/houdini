use security_framework::passwords::{get_generic_password, set_generic_password};
use security_framework_sys::base::errSecItemNotFound;

const SERVICE: &str = "ai.memfold.houdini";
const ACCOUNT: &str = "db-encryption-key";
const ANALYTICS_ACCOUNT: &str = "analytics-api-key";

pub fn db_key() -> Result<[u8; 32], String> {
    match get_generic_password(SERVICE, ACCOUNT) {
        Ok(bytes) => <[u8; 32]>::try_from(bytes.as_slice())
            .map_err(|_| "keychain: stored key is not 32 bytes".to_string()),
        Err(e) => {
            let code = e.code();
            if code == errSecItemNotFound {
                let key = random_key();
                set_generic_password(SERVICE, ACCOUNT, &key)
                    .map_err(|e| format!("keychain: could not store new key: {e}"))?;
                Ok(key)
            } else {
                Err(format!(
                    "keychain: key read failed (OSStatus {code}); refusing to create a replacement key that would make the existing database unreadable"
                ))
            }
        }
    }
}

fn random_key() -> [u8; 32] {
    use rand::RngCore;
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

pub fn analytics_key() -> Option<String> {
    match get_generic_password(SERVICE, ANALYTICS_ACCOUNT) {
        Ok(bytes) => String::from_utf8(bytes)
            .ok()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty()),
        Err(_) => option_env!("HOUDINI_ANALYTICS_KEY")
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string),
    }
}

pub fn set_analytics_key(key: &str) -> Result<(), String> {
    set_generic_password(SERVICE, ANALYTICS_ACCOUNT, key.trim().as_bytes())
        .map_err(|e| format!("keychain: could not store analytics key: {e}"))
}
