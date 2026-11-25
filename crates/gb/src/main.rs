use clap::Parser;
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    let json = std::env::args_os().any(|a| a == "--json");
    let cli = match gammaboard_client::cli::Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if json
                && !matches!(
                    error.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                )
            {
                eprintln!(
                    "{}",
                    serde_json::json!({"error":{"code":"invalid_arguments","message":error.to_string()}})
                );
            } else {
                let _ = error.print();
            }
            return std::process::ExitCode::from(error.exit_code() as u8);
        }
    };
    match gammaboard_client::cli::execute(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if let Some(exit) = error.downcast_ref::<gammaboard_client::ChildExit>() {
                return std::process::ExitCode::from(exit.0);
            }
            if json {
                eprintln!(
                    "{}",
                    serde_json::json!({"error":{"code":"command_failed","message":format!("{error:#}")}})
                );
            } else {
                eprintln!("{error:#}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}
