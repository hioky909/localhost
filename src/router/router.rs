use crate::config::model::{ServerConfig, LocationConfig};
use crate::http::request::HttpRequest;

pub struct Router {
    pub servers: Vec<ServerConfig>,
}

pub struct RouteResult<'a> {
    pub server: &'a ServerConfig,
    pub location: &'a LocationConfig,
    pub effective_root: String,
}

impl Router {
    pub fn new(servers: Vec<ServerConfig>) -> Self {
        Router { servers }
    }

    pub fn route<'a>(&'a self, request: &HttpRequest, _server_addr: &str) -> Option<RouteResult<'a>> {
        // Find matching server (first one for now)
        let server = self.servers.first()?;

        // Find best matching location (longest prefix match)
        let mut best: Option<&LocationConfig> = None;
        let mut best_len = 0;

        for location in &server.locations {
            if request.path.starts_with(&location.path) {
                let loc_len = location.path.len();
                if loc_len > best_len {
                    best_len = loc_len;
                    best = Some(location);
                }
            }
        }

        let location = best?;
        let effective_root = location.root.clone().unwrap_or_else(|| server.root.clone());

        Some(RouteResult { server, location, effective_root })
    }
}
