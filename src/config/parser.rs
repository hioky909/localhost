use std::collections::HashMap;
use crate::config::model::{ServerConfig, LocationConfig};
use crate::utils::error::{ServerError, Result};
use crate::utils::logger::log_warn;

pub fn parse_config(input: &str) -> Result<Vec<ServerConfig>> {
    let tokens = tokenize(input);
    let mut pos = 0;
    let mut servers = Vec::new();

    while pos < tokens.len() {
        if tokens[pos] == "server" {
            pos += 1;
            if pos >= tokens.len() || tokens[pos] != "{" {
                return Err(ServerError::Config("Expected '{' after 'server'".to_string()));
            }
            pos += 1;
            let (server, new_pos) = parse_server(&tokens, pos)?;
            servers.push(server);
            pos = new_pos;
        } else {
            pos += 1;
        }
    }

    if servers.is_empty() {
        return Err(ServerError::Config("No server blocks found".to_string()));
    }

    Ok(servers)
}

fn parse_server(tokens: &[String], mut pos: usize) -> Result<(ServerConfig, usize)> {
    let mut config = ServerConfig {
        listen_addr: "127.0.0.1:8080".to_string(),
        server_name: "localhost".to_string(),
        root: "./www".to_string(),
        index: "index.html".to_string(),
        client_max_body_size: 1048576,
        error_pages: HashMap::new(),
        locations: Vec::new(),
    };

    while pos < tokens.len() && tokens[pos] != "}" {
        match tokens[pos].as_str() {
            "listen" => {
                pos += 1;
                if pos < tokens.len() {
                    config.listen_addr = tokens[pos].trim_end_matches(';').to_string();
                }
                pos += 1;
            }
            "server_name" => {
                pos += 1;
                if pos < tokens.len() {
                    config.server_name = tokens[pos].trim_end_matches(';').to_string();
                }
                pos += 1;
            }
            "root" => {
                pos += 1;
                if pos < tokens.len() {
                    config.root = tokens[pos].trim_end_matches(';').to_string();
                }
                pos += 1;
            }
            "index" => {
                pos += 1;
                if pos < tokens.len() {
                    config.index = tokens[pos].trim_end_matches(';').to_string();
                }
                pos += 1;
            }
            "client_max_body_size" => {
                pos += 1;
                if pos < tokens.len() {
                    let val = tokens[pos].trim_end_matches(';');
                    config.client_max_body_size = val.parse().unwrap_or_else(|_| {
                        log_warn(&format!("Invalid client_max_body_size '{}', using default 1048576", val));
                        1048576
                    });
                }
                pos += 1;
            }
            "error_page" => {
                pos += 1;
                if pos + 1 < tokens.len() {
                    let code_str = tokens[pos].trim_end_matches(';');
                    let code: u16 = code_str.parse().unwrap_or(0);
                    pos += 1;
                    let page = tokens[pos].trim_end_matches(';').to_string();
                    if code > 0 {
                        config.error_pages.insert(code, page);
                    }
                }
                pos += 1;
            }
            "location" => {
                pos += 1;
                if pos >= tokens.len() {
                    return Err(ServerError::Config("Expected path after 'location'".to_string()));
                }
                let loc_path = tokens[pos].clone();
                pos += 1;
                if pos >= tokens.len() || tokens[pos] != "{" {
                    return Err(ServerError::Config("Expected '{' after location path".to_string()));
                }
                pos += 1;
                let (location, new_pos) = parse_location(tokens, pos, loc_path)?;
                config.locations.push(location);
                pos = new_pos;
            }
            _ => {
                pos += 1;
            }
        }
    }

    if pos < tokens.len() && tokens[pos] == "}" {
        pos += 1;
    }

    Ok((config, pos))
}

fn parse_location(tokens: &[String], mut pos: usize, path: String) -> Result<(LocationConfig, usize)> {
    let mut loc = LocationConfig {
        path,
        ..LocationConfig::default()
    };

    while pos < tokens.len() && tokens[pos] != "}" {
        match tokens[pos].as_str() {
            "methods" => {
                pos += 1;
                let mut methods = Vec::new();
                while pos < tokens.len() && tokens[pos] != "}" {
                    let tok = tokens[pos].trim_end_matches(';').to_string();
                    if tok.is_empty() {
                        pos += 1;
                        continue;
                    }
                    let ends_semicolon = tokens[pos].ends_with(';');
                    methods.push(tok.to_uppercase());
                    pos += 1;
                    if ends_semicolon {
                        break;
                    }
                }
                loc.methods = methods;
            }
            "autoindex" => {
                pos += 1;
                if pos < tokens.len() {
                    loc.autoindex = tokens[pos].trim_end_matches(';') == "on";
                }
                pos += 1;
            }
            "upload_dir" => {
                pos += 1;
                if pos < tokens.len() {
                    loc.upload_dir = Some(tokens[pos].trim_end_matches(';').to_string());
                }
                pos += 1;
            }
            "root" => {
                pos += 1;
                if pos < tokens.len() {
                    loc.root = Some(tokens[pos].trim_end_matches(';').to_string());
                }
                pos += 1;
            }
            "cgi" => {
                pos += 1;
                if pos + 1 < tokens.len() {
                    let ext = tokens[pos].trim_end_matches(';').to_string();
                    pos += 1;
                    let interp = tokens[pos].trim_end_matches(';').to_string();
                    loc.cgi.insert(ext, interp);
                }
                pos += 1;
            }
            _ => {
                pos += 1;
            }
        }
    }

    if pos < tokens.len() && tokens[pos] == "}" {
        pos += 1;
    }

    Ok((loc, pos))
}

fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '#' => {
                // Comment - skip to end of line
                for c2 in chars.by_ref() {
                    if c2 == '\n' {
                        break;
                    }
                }
            }
            '{' | '}' => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push(c.to_string());
            }
            ' ' | '\t' | '\n' | '\r' => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(c);
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}
