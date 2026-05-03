#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Delete,
}

#[derive(Debug, Clone)]
pub struct RouteConfig {
    pub path: String,
    pub accepted_methods: Vec<HttpMethod>,
    pub redirection: Option<String>,
    pub root_directory: Option<String>,
    pub index_file: Option<String>,
    pub directory_listing: bool,
    pub cgi_extension: Option<String>,
    pub upload_store: Option<String>,
}

impl Default for RouteConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            accepted_methods: vec![HttpMethod::Get, HttpMethod::Post, HttpMethod::Delete],
            redirection: None,
            root_directory: None,
            index_file: None,
            directory_listing: false,
            cgi_extension: None,
            upload_store: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub server_names: Vec<String>,
    pub host: String,
    pub ports: Vec<u16>,
    pub error_pages: std::collections::HashMap<u16, String>,
    pub client_max_body_size: Option<usize>,
    pub timeout: Option<u64>,
    pub routes: Vec<RouteConfig>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            server_names: Vec::new(),
            host: "127.0.0.1".to_string(),
            ports: Vec::new(),
            error_pages: std::collections::HashMap::new(),
            client_max_body_size: None,
            timeout: Some(30), // Default 30 seconds
            routes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub servers: Vec<ServerConfig>,
}

pub mod parser;

impl Config {
    pub fn new() -> Self {
        Self {
            servers: Vec::new(),
        }
    }
    
    pub fn load_file(path: &str) -> Result<Self, parser::ConfigError> {
        parser::parse_file(path)
    }

    pub fn load_string(content: &str) -> Result<Self, parser::ConfigError> {
        parser::parse_string(content)
    }
}

