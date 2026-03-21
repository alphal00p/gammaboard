use anyhow::Result;
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
};
use clap::Args;

#[derive(Debug, Args)]
pub struct AuthArgs {
    #[arg(long)]
    pub password: String,
}

pub fn run_auth_hash_command(args: AuthArgs) -> Result<()> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(args.password.as_bytes(), &salt)
        .map_err(|err| anyhow::anyhow!("failed to hash password: {err}"))?
        .to_string();
    println!("{hash}");
    Ok(())
}
