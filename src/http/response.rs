use std::collections::HashMap;

pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        _ => "Unknown",
    }
}

impl HttpResponse {
    pub fn new(status: u16) -> Self {
        let mut headers = HashMap::new();
        headers.insert("Server".to_string(), "localhost/0.1.0".to_string());
        headers.insert("Connection".to_string(), "keep-alive".to_string());
        HttpResponse { status, headers, body: Vec::new() }
    }

    pub fn with_body(mut self, body: Vec<u8>, content_type: &str) -> Self {
        self.headers.insert("Content-Type".to_string(), content_type.to_string());
        self.headers.insert("Content-Length".to_string(), body.len().to_string());
        self.body = body;
        self
    }

    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let status_line = format!("HTTP/1.1 {} {}\r\n", self.status, status_text(self.status));
        let mut out = status_line.into_bytes();
        for (k, v) in &self.headers {
            out.extend_from_slice(k.as_bytes());
            out.extend_from_slice(b": ");
            out.extend_from_slice(v.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&self.body);
        out
    }

    pub fn error(status: u16, message: &str) -> Self {
        let body = format!(
            "<html><body><h1>{} {}</h1><p>{}</p></body></html>",
            status, status_text(status), message
        );
        HttpResponse::new(status).with_body(body.into_bytes(), "text/html")
    }

    #[allow(dead_code)]
    pub fn redirect(location: &str) -> Self {
        HttpResponse::new(302).with_header("Location", location)
    }
}
