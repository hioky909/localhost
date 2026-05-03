#![allow(dead_code)]
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Delete,
    Unknown(String),
}

impl From<&str> for HttpMethod {
    fn from(s: &str) -> Self {
        match s {
            "GET" => HttpMethod::Get,
            "POST" => HttpMethod::Post,
            "DELETE" => HttpMethod::Delete,
            other => HttpMethod::Unknown(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpVersion {
    V1_1,
    Unknown(String),
}

impl From<&str> for HttpVersion {
    fn from(s: &str) -> Self {
        match s {
            "HTTP/1.1" => HttpVersion::V1_1,
            other => HttpVersion::Unknown(other.to_string()),
        }
    }
}

#[derive(Debug)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub uri: String,
    pub path: String,
    pub query: String,
    pub version: HttpVersion,
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn new() -> Self {
        Self {
            method: HttpMethod::Get,
            uri: String::new(),
            path: String::new(),
            query: String::new(),
            version: HttpVersion::V1_1,
            headers: HashMap::new(),
            cookies: HashMap::new(),
            body: Vec::new(),
        }
    }

    /// Parse a complete request from raw bytes
    pub fn parse(raw: &[u8]) -> Option<Self> {
        // Find double CRLF which separates headers from body
        let mut headers_end = 0;
        let mut found = false;
        
        for i in 0..raw.len().saturating_sub(3) {
            if raw[i] == b'\r' && raw[i+1] == b'\n' && raw[i+2] == b'\r' && raw[i+3] == b'\n' {
                headers_end = i;
                found = true;
                break;
            }
        }

        if !found {
            return None; // Incomplete request
        }

        let header_bytes = &raw[0..headers_end];
        let body_bytes = &raw[headers_end + 4..];

        let header_str = std::str::from_utf8(header_bytes).ok()?;
        let mut lines = header_str.lines();

        // Parse Request-Line: Method Request-URI HTTP-Version
        let request_line = lines.next()?;
        let mut parts = request_line.split_whitespace();
        let method = parts.next()?.into();
        let uri = parts.next()?.to_string();
        let version = parts.next()?.into();

        let (path, query) = if let Some(idx) = uri.find('?') {
            (uri[..idx].to_string(), uri[idx + 1..].to_string())
        } else {
            (uri.clone(), String::new())
        };

        let mut req = HttpRequest {
            method,
            uri,
            path,
            query,
            version,
            headers: HashMap::new(),
            cookies: HashMap::new(),
            body: Vec::new(),
        };

        // Parse Headers
        for line in lines {
            if line.is_empty() {
                break;
            }
            if let Some(colon_idx) = line.find(':') {
                let key = line[0..colon_idx].trim().to_string();
                let value = line[colon_idx + 1..].trim().to_string();
                // Store headers in lowercase for easier lookup later
                req.headers.insert(key.to_lowercase(), value);
            }
        }

        // Handle Transfer-Encoding: chunked
        let is_chunked = req.headers.get("transfer-encoding").map(|v| v.to_lowercase() == "chunked").unwrap_or(false);
        if is_chunked {
            let mut current_pos = 0;
            let mut decoded_body = Vec::new();
            while current_pos < body_bytes.len() {
                // Find CRLF after chunk size
                let mut size_end = current_pos;
                while size_end < body_bytes.len() && (size_end + 1 < body_bytes.len()) {
                    if body_bytes[size_end] == b'\r' && body_bytes[size_end+1] == b'\n' {
                        break;
                    }
                    size_end += 1;
                }
                
                if size_end >= body_bytes.len() - 1 { break; } // Unexpected end
                
                let size_str = std::str::from_utf8(&body_bytes[current_pos..size_end]).ok()?;
                let chunk_size = usize::from_str_radix(size_str.trim(), 16).ok()?;
                
                if chunk_size == 0 {
                    break; // End of chunks
                }
                
                let chunk_start = size_end + 2;
                let chunk_end = chunk_start + chunk_size;
                
                if chunk_end > body_bytes.len() {
                    return None; // Incomplete chunk
                }
                
                decoded_body.extend_from_slice(&body_bytes[chunk_start..chunk_end]);
                current_pos = chunk_end + 2; // +2 for the CRLF after the chunk data
            }
            req.body = decoded_body;
        } else {
            req.body = body_bytes.to_vec();
        }

        // Parse cookies from the "cookie" header
        if let Some(cookie_header) = req.headers.get("cookie") {
            for pair in cookie_header.split(';') {
                let mut parts = pair.splitn(2, '=');
                if let (Some(name), Some(value)) = (parts.next(), parts.next()) {
                    req.cookies.insert(name.trim().to_string(), value.trim().to_string());
                }
            }
        }

        Some(req)
    }
}
