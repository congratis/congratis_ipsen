use argon2::{password_hash::{PasswordHasher, SaltString}, Argon2};
use rand::{rngs::OsRng, RngCore};
use std::env;

fn main() -> anyhow::Result<()> {
    let password = env::var("ADMIN_PASSWORD")
        .map_err(|_| anyhow::anyhow!("set ADMIN_PASSWORD in the environment"))?;
    anyhow::ensure!(password.len() >= 14, "ADMIN_PASSWORD must be at least 14 bytes");
    let mut salt_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|e| anyhow::anyhow!("salt encoding failed: {e}"))?;
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))?
        .to_string();
    println!("{hash}");
    Ok(())
}
