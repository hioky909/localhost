use std::sync::atomic::{AtomicBool, Ordering};
use crate::config::parser::parse_config;
use crate::router::router::Router;
use crate::server::listener::create_listener;
use crate::server::event_loop::EventLoop;
use crate::utils::logger::{log_info, log_error};

mod config;
mod http;
mod network;
mod router;
mod server;
mod cgi;
mod utils;

pub static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_signal(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::Relaxed);
}

fn main() {
    // Read config
    let config_content = match std::fs::read_to_string("config.conf") {
        Ok(s) => s,
        Err(e) => {
            log_error(&format!("Failed to read config.conf: {}", e));
            std::process::exit(1);
        }
    };

    let configs = match parse_config(&config_content) {
        Ok(c) => c,
        Err(e) => {
            log_error(&format!("Config error: {}", e));
            std::process::exit(1);
        }
    };

    if configs.is_empty() {
        log_error("No server configurations found");
        std::process::exit(1);
    }

    let addr = configs[0].listen_addr.clone();

    // Set up signal handlers
    unsafe {
        libc::signal(libc::SIGINT, handle_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handle_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let listener_fd = match create_listener(&addr) {
        Ok(fd) => fd,
        Err(e) => {
            log_error(&format!("Failed to create listener: {}", e));
            std::process::exit(1);
        }
    };

    log_info(&format!("Server listening on http://{}", addr));

    let router = Router::new(configs.clone());

    let mut event_loop = match EventLoop::new(listener_fd, router, configs) {
        Ok(el) => el,
        Err(e) => {
            log_error(&format!("Failed to create event loop: {}", e));
            unsafe { libc::close(listener_fd); }
            std::process::exit(1);
        }
    };

    if let Err(e) = event_loop.run() {
        log_error(&format!("Event loop error: {}", e));
    }

    unsafe { libc::close(listener_fd); }
    log_info("Server stopped");
}
