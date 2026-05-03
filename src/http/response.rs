#![allow(dead_code)]
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpStatus {
    Ok = 200,
    Created = 201,
    Accepted = 202,
    NoContent = 204,
    MovedPermanently = 301,
    Found = 302,
    BadRequest = 400,
    Forbidden = 403,
    NotFound = 404,
    MethodNotAllowed = 405,
    PayloadTooLarge = 413,
    InternalServerError = 500,
    NotImplemented = 501,
    HttpVersionNotSupported = 505,
}

impl HttpStatus {
    pub fn reason_phrase(&self) -> &'static str {
        match self {
            HttpStatus::Ok => "OK",
            HttpStatus::Created => "Created",
            HttpStatus::Accepted => "Accepted",
            HttpStatus::NoContent => "No Content",
            HttpStatus::MovedPermanently => "Moved Permanently",
            HttpStatus::Found => "Found",
            HttpStatus::BadRequest => "Bad Request",
            HttpStatus::Forbidden => "Forbidden",
            HttpStatus::NotFound => "Not Found",
            HttpStatus::MethodNotAllowed => "Method Not Allowed",
            HttpStatus::PayloadTooLarge => "Payload Too Large",
            HttpStatus::InternalServerError => "Internal Server Error",
            HttpStatus::NotImplemented => "Not Implemented",
            HttpStatus::HttpVersionNotSupported => "HTTP Version Not Supported",
        }
    }
}

#[derive(Clone)]
pub struct HttpResponse {
    pub version: String,
    pub status: HttpStatus,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: HttpStatus) -> Self {
        Self {
            version: "HTTP/1.1".to_string(),
            status,
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    pub fn set_header(&mut self, key: &str, value: &str) {
        self.headers.insert(key.to_string(), value.to_string());
    }

    pub fn set_body(&mut self, body: Vec<u8>) {
        self.set_header("Content-Length", &body.len().to_string());
        self.body = body;
    }

    pub fn set_cookie(&mut self, name: &str, value: &str, options: &str) {
        let cookie_str = if options.is_empty() {
            format!("{}={}", name, value)
        } else {
            format!("{}={}; {}", name, value, options)
        };
        // Note: In a real server, we might want multiple Set-Cookie headers.
        // For simplicity here, we append if already present.
        let existing = self.headers.get("Set-Cookie").cloned();
        match existing {
            Some(prev) => self.headers.insert("Set-Cookie".to_string(), format!("{}, {}", prev, cookie_str)),
            None => self.headers.insert("Set-Cookie".to_string(), cookie_str),
        };
    }

    /// Serializes the response into a raw byte vector ready to send over the socket
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut response = Vec::new();
        
        // Status line
        let status_line = format!(
            "{} {} {}\r\n",
            self.version,
            self.status.clone() as u16,
            self.status.reason_phrase()
        );
        response.extend_from_slice(status_line.as_bytes());

        // Headers
        for (key, value) in &self.headers {
            let header_line = format!("{}: {}\r\n", key, value);
            response.extend_from_slice(header_line.as_bytes());
        }

        // Empty line to separate headers from body
        response.extend_from_slice(b"\r\n");

        // Body
        response.extend_from_slice(&self.body);

        response
    }
}
