#![allow(dead_code)]

mod config;
mod http;
mod server;

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let config_path = if args.len() > 1 {
        &args[1]
    } else {
        "server.conf"
    };

    println!("Loading configuration from {}", config_path);
    let config = match config::Config::load_file(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load config: {:?}", e);
            std::process::exit(1);
        }
    };

    // Initialize the server
    let mut srv = server::Server::new(config)?;
    srv.setup_listeners()?;
    
    // Start the event loop (this will block)
    srv.run()
}
