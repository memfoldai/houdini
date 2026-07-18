use security_framework::passwords::{get_generic_password, set_generic_password};

const SERVICE: &str = "ai.memfold.ai-usage-monitor";
const ACCOUNT: &str = "db-encryption-key";

pub fn db_key() -> [u8; 32] {
    if let Ok(bytes) = get_generic_password(SERVICE, ACCOUNT) {
        if let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice()) {
            return key;
        }
    }
    let key = random_key();
    if let Err(e) = set_generic_password(SERVICE, ACCOUNT, &key) {
        log::error!("keychain: could not persist db key ({e}); this session's store may not reopen");
    }
    key
}

fn random_key() -> [u8; 32] {
    use rand::RngCore;
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}
