use anyhow::Result;
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
};
use clap::{Args, Subcommand};
use std::io::{self, Read};

#[derive(Debug, Args)]
pub struct AuthArgs {
    #[command(subcommand)]
    command: AuthCommand,
}

#[derive(Debug, Subcommand)]
enum AuthCommand {
    /// Generate an Argon2 password hash without exposing the password in process arguments
    HashPassword {
        /// Read the password from standard input instead of prompting on the terminal
        #[arg(long)]
        password_stdin: bool,
    },
}

pub fn run_auth_command(args: AuthArgs) -> Result<()> {
    match args.command {
        AuthCommand::HashPassword { password_stdin } => {
            hash_password(read_password(password_stdin)?)
        }
    }
}

fn read_password(password_stdin: bool) -> Result<String> {
    if password_stdin {
        let mut password = String::new();
        io::stdin()
            .read_to_string(&mut password)
            .map_err(|err| anyhow::anyhow!("failed reading password from standard input: {err}"))?;
        let password = password.trim_end_matches(['\r', '\n']).to_string();
        if password.is_empty() {
            anyhow::bail!("password read from standard input must not be empty");
        }
        return Ok(password);
    }

    let password = rpassword::prompt_password("Password: ")?;
    if password.is_empty() {
        anyhow::bail!("password must not be empty");
    }
    Ok(password)
}

fn hash_password(password: String) -> Result<()> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| anyhow::anyhow!("failed to hash password: {err}"))?
        .to_string();
    println!("{hash}");
    Ok(())
}
