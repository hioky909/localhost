use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub server_name: String,
    pub root: String,
    pub index: String,
    pub client_max_body_size: usize,
    pub error_pages: HashMap<u16, String>,
    pub locations: Vec<LocationConfig>,
}

#[derive(Debug, Clone)]
pub struct LocationConfig {
    pub path: String,
    pub methods: Vec<String>,
    pub autoindex: bool,
    pub upload_dir: Option<String>,
    pub root: Option<String>,
    pub cgi: HashMap<String, String>,
}

impl Default for LocationConfig {
    fn default() -> Self {
        LocationConfig {
            path: "/".to_string(),
            methods: vec!["GET".to_string()],
            autoindex: false,
            upload_dir: None,
            root: None,
            cgi: HashMap::new(),
        }
    }
}
