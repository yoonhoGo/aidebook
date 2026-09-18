use aidebook_lib::core::{Core, CoreEndpoint, CoreError, CoreServer};
use std::env;

fn main() {
    if let Err(error) = run() {
        eprintln!("aidebook-core: {}", error);
        std::process::exit(1);
    }
}

fn run() -> Result<(), CoreError> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let db = required(&args, "--db")?;
    let socket = required(&args, "--socket")?;
    let token_file = required(&args, "--token-file")?;
    let endpoint = CoreEndpoint {
        socket_path: socket.into(),
        token_path: token_file.into(),
    };
    let core = Core::open(db)?;
    let server = CoreServer::bind(endpoint, core, None)?;
    server.serve()
}

fn required(args: &[String], flag: &str) -> Result<String, CoreError> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
        .ok_or_else(|| CoreError::InvalidInput {
            field: flag.trim_start_matches('-').to_string(),
            message: "is required".to_string(),
        })
}
