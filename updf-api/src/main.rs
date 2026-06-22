// Binary entry point for the updf HTTP API.
//
// Reads its configuration from the environment (UPDF_API_ADDR, UPDF_BIN) and serves
// until stopped. The root `ultimate-pdf` supervisor normally launches this process.

use std::process::ExitCode;

use updf_api::Config;

#[tokio::main]
async fn main() -> ExitCode {
    let config = Config::from_env();
    println!("updf-api: surface over `{}`", config.updf_bin.display());
    println!("listening on http://{} (try GET /health)", config.addr);

    match updf_api::serve(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("updf-api error: {e}");
            ExitCode::FAILURE
        }
    }
}
