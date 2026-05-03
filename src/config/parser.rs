use super::{Config, HttpMethod, RouteConfig, ServerConfig};
// use std::collections::HashMap;

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    ParseError(String),
}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e)
    }
}

pub fn parse_file(path: &str) -> Result<Config, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    parse_string(&content)
}

pub fn parse_string(content: &str) -> Result<Config, ConfigError> {
    let mut tokens = tokenize(content);
    let mut config = Config::new();
    
    while let Some(token) = tokens.next() {
        if token == "server" {
            let server_config = parse_server(&mut tokens)?;
            config.servers.push(server_config);
        } else {
            return Err(ConfigError::ParseError(format!("Expected 'server' block, found '{}'", token)));
        }
    }
    
    Ok(config)
}

fn tokenize(content: &str) -> impl Iterator<Item = String> + '_ {
    content
        .replace("{", " { ")
        .replace("}", " } ")
        .replace(";", " ; ")
        .split_whitespace()
        .map(|s| s.to_string())
        .collect::<Vec<String>>()
        .into_iter()
}

fn parse_server(tokens: &mut impl Iterator<Item = String>) -> Result<ServerConfig, ConfigError> {
    expect_token(tokens, "{")?;
    let mut server = ServerConfig::default();

    while let Some(token) = tokens.next() {
        if token == "}" {
            break;
        }

        match token.as_str() {
            "listen" => {
                let port_str = expect_value(tokens)?;
                let port = port_str.parse::<u16>().map_err(|_| ConfigError::ParseError(format!("Invalid port: {}", port_str)))?;
                server.ports.push(port);
                expect_token(tokens, ";")?;
            }
            "host" => {
                server.host = expect_value(tokens)?;
                expect_token(tokens, ";")?;
            }
            "server_name" => {
                while let Some(name) = tokens.next() {
                    if name == ";" { break; }
                    server.server_names.push(name);
                }
            }
            "error_page" => {
                let code_str = expect_value(tokens)?;
                let code = code_str.parse::<u16>().map_err(|_| ConfigError::ParseError(format!("Invalid status code: {}", code_str)))?;
                let path = expect_value(tokens)?;
                server.error_pages.insert(code, path);
                expect_token(tokens, ";")?;
            }
            "client_max_body_size" => {
                let size_str = expect_value(tokens)?;
                let size = size_str.parse::<usize>().map_err(|_| ConfigError::ParseError(format!("Invalid size: {}", size_str)))?;
                server.client_max_body_size = Some(size);
                expect_token(tokens, ";")?;
            }
            "timeout" => {
                let timeout_str = expect_value(tokens)?;
                let timeout = timeout_str.parse::<u64>().map_err(|_| ConfigError::ParseError(format!("Invalid timeout: {}", timeout_str)))?;
                server.timeout = Some(timeout);
                expect_token(tokens, ";")?;
            }
            "location" => {
                let location = parse_location(tokens)?;
                server.routes.push(location);
            }
            _ => return Err(ConfigError::ParseError(format!("Unknown server directive: {}", token))),
        }
    }

    Ok(server)
}

fn parse_location(tokens: &mut impl Iterator<Item = String>) -> Result<RouteConfig, ConfigError> {
    let mut route = RouteConfig::default();
    route.path = expect_value(tokens)?;
    expect_token(tokens, "{")?;

    while let Some(token) = tokens.next() {
        if token == "}" {
            break;
        }

        match token.as_str() {
            "allow_methods" => {
                route.accepted_methods.clear();
                while let Some(method_str) = tokens.next() {
                    if method_str == ";" { break; }
                    match method_str.to_uppercase().as_str() {
                        "GET" => route.accepted_methods.push(HttpMethod::Get),
                        "POST" => route.accepted_methods.push(HttpMethod::Post),
                        "DELETE" => route.accepted_methods.push(HttpMethod::Delete),
                        _ => return Err(ConfigError::ParseError(format!("Unknown method: {}", method_str))),
                    }
                }
            }
            "return" => {
                route.redirection = Some(expect_value(tokens)?);
                expect_token(tokens, ";")?;
            }
            "root" => {
                route.root_directory = Some(expect_value(tokens)?);
                expect_token(tokens, ";")?;
            }
            "index" => {
                route.index_file = Some(expect_value(tokens)?);
                expect_token(tokens, ";")?;
            }
            "autoindex" => {
                let val = expect_value(tokens)?;
                route.directory_listing = val == "on";
                expect_token(tokens, ";")?;
            }
            "cgi_extension" => {
                route.cgi_extension = Some(expect_value(tokens)?);
                expect_token(tokens, ";")?;
            }
            "upload_store" => {
                route.upload_store = Some(expect_value(tokens)?);
                expect_token(tokens, ";")?;
            }
            _ => return Err(ConfigError::ParseError(format!("Unknown location directive: {}", token))),
        }
    }

    Ok(route)
}

fn expect_token(tokens: &mut impl Iterator<Item = String>, expected: &str) -> Result<(), ConfigError> {
    match tokens.next() {
        Some(t) if t == expected => Ok(()),
        Some(t) => Err(ConfigError::ParseError(format!("Expected '{}', found '{}'", expected, t))),
        None => Err(ConfigError::ParseError(format!("Expected '{}', found EOF", expected))),
    }
}

fn expect_value(tokens: &mut impl Iterator<Item = String>) -> Result<String, ConfigError> {
    match tokens.next() {
        Some(t) if t == ";" || t == "{" || t == "}" => Err(ConfigError::ParseError(format!("Expected value, found '{}'", t))),
        Some(t) => Ok(t),
        None => Err(ConfigError::ParseError("Expected value, found EOF".to_string())),
    }
}
