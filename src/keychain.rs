use security_framework::passwords::{get_generic_password, set_generic_password};
use security_framework_sys::base::errSecItemNotFound;

const SERVICE: &str = "ai.memfold.houdini";
const ACCOUNT: &str = "db-encryption-key";

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
