#![allow(dead_code)]

mod cli;
mod config;
mod http;
mod log;
mod server;
mod stats;
mod term;

fn main() {
    std::process::exit(cli::main());
}
