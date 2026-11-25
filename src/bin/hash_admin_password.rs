use argon2::{password_hash::{rand_core::OsRng, SaltString}, Argon2, PasswordHasher};
use std::env;

fn main() -> anyhow::Result<()> {
    let password = env::var("ADMIN_PASSWORD")
        .map_err(|_| anyhow::anyhow!("set ADMIN_PASSWORD in the environment"))?;
    anyhow::ensure!(password.len() >= 14, "ADMIN_PASSWORD must be at least 14 bytes");
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))?
        .to_string();
    println!("{hash}");
    Ok(())
}
