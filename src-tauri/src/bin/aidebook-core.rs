use aidebook_lib::core::{local_sync::LocalSync, plugins::PluginRegistry};
use aidebook_lib::core::{Core, CoreEndpoint, CoreError, CoreServer};
use std::{
    env,
    sync::{atomic::AtomicBool, Arc, Mutex},
};

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
    let server = CoreServer::bind(endpoint.clone(), core.clone(), None)?;
    let registry_path = endpoint
        .socket_path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("connections.json");
    let registry = Arc::new(Mutex::new(PluginRegistry::open(registry_path)?));
    let sync = Arc::new(LocalSync::new(core, registry));
    let server = server.with_plugins(sync.clone());
    std::thread::spawn(move || sync.run(Arc::new(AtomicBool::new(false))));
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
