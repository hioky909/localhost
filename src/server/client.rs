use std::io::{Read, Write};
use std::os::unix::io::{RawFd, FromRawFd};

use crate::http::request::HttpRequest;
use crate::http::response::{HttpResponse, HttpStatus};
use crate::http::CgiInfo;
use crate::config::ServerConfig;
use crate::http::router::{Router};

#[derive(Debug, PartialEq)]
pub enum ClientState {
    ReadingRequest,
    Processing,
    CgiWritingStdin,
    CgiReadingStdout,
    WritingResponse,
    Done,
}

pub struct Client {
    pub fd: RawFd,
    pub state: ClientState,
    pub server_config: ServerConfig,
    pub read_buffer: Vec<u8>,
    pub write_buffer: Vec<u8>,
    pub written_bytes: usize,
    pub last_activity: std::time::Instant,
    pub request: Option<HttpRequest>,
    pub response: Option<HttpResponse>,
    pub cgi_pid: Option<i32>,
    pub cgi_stdin_fd: Option<i32>,
    pub cgi_stdout_fd: Option<i32>,
    /// Adresse du client (ip:port), pour le journal.
    pub peer: String,
    /// Premier octet reçu : début de la mesure de durée.
    pub started_at: Option<std::time::Instant>,
    /// Requête interne (/__localhost/status) : ni journalisée ni comptée.
    pub internal: bool,
    /// Socket d'écoute qui a accepté la connexion (sélection de l'hôte virtuel).
    pub listener_fd: RawFd,
    /// « 100 Continue » déjà envoyé pour cette requête.
    pub sent_continue: bool,
}

impl Client {
    pub fn new(fd: RawFd, server_config: ServerConfig, peer: String) -> Self {
        Self {
            fd,
            state: ClientState::ReadingRequest,
            server_config,
            read_buffer: Vec::new(),
            write_buffer: Vec::new(),
            written_bytes: 0,
            last_activity: std::time::Instant::now(),
            request: None,
            response: None,
            cgi_pid: None,
            cgi_stdin_fd: None,
            cgi_stdout_fd: None,
            peer,
            started_at: None,
            internal: false,
            listener_fd: -1,
            sent_continue: false,
        }
    }

    /// Code de statut réellement envoyé (lu sur la ligne d'état de la réponse).
    pub fn status_code(&self) -> u16 {
        let head = &self.write_buffer[..self.write_buffer.len().min(16)];
        std::str::from_utf8(head)
            .ok()
            .and_then(|s| s.split_whitespace().nth(1))
            .and_then(|c| c.parse().ok())
            .unwrap_or(0)
    }

    /// Méthode et chemin, y compris pour une requête rejetée avant d'être analysée (413…).
    pub fn method_and_path(&self) -> (String, String) {
        if let Some(req) = &self.request {
            let m = match &req.method {
                crate::http::request::HttpMethod::Get => "GET".to_string(),
                crate::http::request::HttpMethod::Post => "POST".to_string(),
                crate::http::request::HttpMethod::Delete => "DELETE".to_string(),
                crate::http::request::HttpMethod::Unknown(x) => x.clone(),
            };
            let path = if req.query.is_empty() { req.path.clone() } else { format!("{}?{}", req.path, req.query) };
            return (m, path);
        }
        let head = &self.read_buffer[..self.read_buffer.len().min(512)];
        let line = String::from_utf8_lossy(head);
        let mut parts = line.lines().next().unwrap_or("").split_whitespace();
        (parts.next().unwrap_or("-").to_string(), parts.next().unwrap_or("-").to_string())
    }

    pub fn host(&self) -> String {
        self.request
            .as_ref()
            .and_then(|r| r.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("host")).map(|(_, v)| v.clone()))
            .unwrap_or_else(|| self.server_config.server_names.first().cloned().unwrap_or_default())
    }

    pub fn read_data(&mut self) -> std::io::Result<bool> {
        let mut buf = [0u8; 4096];
        let mut file = unsafe { std::fs::File::from_raw_fd(self.fd) };

        match file.read(&mut buf) {
            Ok(0) => {
                self.state = ClientState::Done;
                std::mem::forget(file);
                return Ok(true);
            }
            Ok(n) => {
                if self.started_at.is_none() {
                    self.started_at = Some(std::time::Instant::now());
                }
                self.read_buffer.extend_from_slice(&buf[0..n]);
                self.last_activity = std::time::Instant::now(); // Update activity
                
                // Check body size limit before parsing complete request
                if let Some(max_size) = self.server_config.client_max_body_size {
                    if self.read_buffer.len() > max_size {
                        // Send 413 Payload Too Large response
                        let mut res = HttpResponse::new(HttpStatus::PayloadTooLarge);
                        res.set_header("Content-Length", "0");
                        self.write_buffer = res.to_bytes();
                        self.state = ClientState::WritingResponse;
                        std::mem::forget(file);
                        return Ok(true);
                    }
                }
                
                if let Some(req) = HttpRequest::parse(&self.read_buffer) {
                    self.request = Some(req);
                    self.state = ClientState::Processing;
                    std::mem::forget(file);
                    return Ok(true);
                }
                if !self.sent_continue && HttpRequest::expects_continue(&self.read_buffer) {
                    // curl & co. attendent ce feu vert avant d'envoyer un corps volumineux.
                    let _ = file.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
                    self.sent_continue = true;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => {
                std::mem::forget(file);
                return Err(e);
            }
        }
        
        std::mem::forget(file);
        Ok(false)
    }

    pub fn process_request(&mut self) -> Result<HttpResponse, CgiInfo> {
        if let Some(req) = &self.request {
            let router = Router::new(&self.server_config);
            let result = router.handle_request(req);
            
            match &result {
                Ok(res) => {
                    self.write_buffer = res.to_bytes();
                    self.response = Some(res.clone());
                    self.state = ClientState::WritingResponse;
                }
                Err(info) => {
                    self.cgi_pid = Some(info.pid);
                    self.cgi_stdin_fd = Some(info.stdin_fd);
                    self.cgi_stdout_fd = Some(info.stdout_fd);
                    if req.body.is_empty() {
                         self.state = ClientState::CgiReadingStdout;
                    } else {
                         self.state = ClientState::CgiWritingStdin;
                    }
                }
            }
            result
        } else {
            let mut bad_req = HttpResponse::new(HttpStatus::BadRequest);
            bad_req.set_body(b"Bad Request".to_vec());
            self.write_buffer = bad_req.to_bytes();
            self.state = ClientState::WritingResponse;
            Ok(bad_req)
        }
    }

    pub fn write_cgi_stdin(&mut self) -> std::io::Result<bool> {
        if let (Some(stdin_fd), Some(req)) = (self.cgi_stdin_fd, &self.request) {
            let mut file = unsafe { std::fs::File::from_raw_fd(stdin_fd) };
            
            match file.write(&req.body) {
                Ok(_) => {
                    self.state = ClientState::CgiReadingStdout;
                    std::mem::forget(file);
                    return Ok(true);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    std::mem::forget(file);
                    return Err(e);
                }
            }
            std::mem::forget(file);
        }
        Ok(false)
    }

    pub fn read_cgi_stdout(&mut self) -> std::io::Result<bool> {
        if let Some(stdout_fd) = self.cgi_stdout_fd {
            let mut buf = [0u8; 4096];
            let mut file = unsafe { std::fs::File::from_raw_fd(stdout_fd) };

            match file.read(&mut buf) {
                Ok(0) => {
                    // EOF: finished reading CGI output
                    self.parse_cgi_response();
                    self.state = ClientState::WritingResponse;
                    std::mem::forget(file);
                    return Ok(true);
                }
                Ok(n) => {
                    self.write_buffer.extend_from_slice(&buf[0..n]);
                    std::mem::forget(file);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::mem::forget(file);
                }
                Err(e) => {
                    std::mem::forget(file);
                    return Err(e);
                }
            }
        }
        Ok(false)
    }

    fn parse_cgi_response(&mut self) {
        let output = std::mem::take(&mut self.write_buffer);
        let output_str = String::from_utf8_lossy(&output);
        let mut res = HttpResponse::new(HttpStatus::Ok);
        
        let (headers_len, body_split) = if let Some(idx) = output_str.find("\r\n\r\n") {
            (idx, 4)
        } else if let Some(idx) = output_str.find("\n\n") {
            (idx, 2)
        } else {
            (0, 0)
        };

        if body_split > 0 {
            let headers_part = &output_str[..headers_len];
            let body = &output[headers_len + body_split..];
            
            for line in headers_part.lines() {
                if let Some(colon_idx) = line.find(':') {
                    let key = line[0..colon_idx].trim();
                    let value = line[colon_idx + 1..].trim();
                    if key.to_lowercase() == "status" {
                        if let Some(code) = value.split_whitespace().next().and_then(|s| s.parse::<u16>().ok()) {
                            res.status = match code {
                                200 => HttpStatus::Ok,
                                201 => HttpStatus::Created,
                                400 => HttpStatus::BadRequest,
                                403 => HttpStatus::Forbidden,
                                404 => HttpStatus::NotFound,
                                500 => HttpStatus::InternalServerError,
                                _ => HttpStatus::Ok,
                            };
                        }
                    } else {
                        res.set_header(key, value);
                    }
                }
            }
            res.set_body(body.to_vec());
        } else {
            res.set_body(output);
        }
        
        self.write_buffer = res.to_bytes();
        self.response = Some(res);
    }

    pub fn write_data(&mut self) -> std::io::Result<bool> {
        if self.written_bytes >= self.write_buffer.len() {
            self.state = ClientState::Done;
            return Ok(true);
        }

        let mut file = unsafe { std::fs::File::from_raw_fd(self.fd) };
        let to_write = &self.write_buffer[self.written_bytes..];

        match file.write(to_write) {
            Ok(n) => {
                self.written_bytes += n;
                self.last_activity = std::time::Instant::now(); // Update activity
                if self.written_bytes >= self.write_buffer.len() {
                    self.state = ClientState::Done;
                    std::mem::forget(file);
                    return Ok(true);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => {
                std::mem::forget(file);
                return Err(e);
            }
        }

        std::mem::forget(file);
        Ok(false)
    }
}
