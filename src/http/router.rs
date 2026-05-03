use std::path::{Path, PathBuf};
use crate::config::{HttpMethod as ConfigMethod, RouteConfig, ServerConfig};
use crate::http::request::{HttpMethod, HttpRequest};
use crate::http::response::{HttpResponse, HttpStatus};
use crate::http::mime;
use super::CgiInfo;
use std::ffi::CString;
use std::ptr;
use libc::{c_int, c_char};


pub struct Router {
    routes: Vec<RouteConfig>,
    error_pages: std::collections::HashMap<u16, String>,
    client_max_body_size: Option<usize>,
}


impl Router {
    pub fn new(config: &ServerConfig) -> Self {
        Self {
            routes: config.routes.clone(),
            error_pages: config.error_pages.clone(),
            client_max_body_size: config.client_max_body_size,
        }
    }

    /// Main entry: route a request and return a response
    pub fn handle_request(&self, req: &HttpRequest) -> Result<HttpResponse, CgiInfo> {
        // 1. Unknown methods → 501
        if let HttpMethod::Unknown(_) = &req.method {
            return Ok(self.error_response(501));
        }

        // 2. Check body size limit
        if let Some(max) = self.client_max_body_size {
            if req.body.len() > max {
                return Ok(self.error_response(413));
            }
        }

        // 3. Find the best matching route (longest prefix)
        let best = self
            .routes
            .iter()
            .filter(|r| req.path.starts_with(&r.path))
            .max_by_key(|r| r.path.len());

        let route = match best {
            Some(r) => r,
            None => return Ok(self.error_response(404)),
        };

        // 4. Check method is allowed for this route
        let method_allowed = route.accepted_methods.iter().any(|m| match (m, &req.method) {
            (ConfigMethod::Get, HttpMethod::Get) => true,
            (ConfigMethod::Post, HttpMethod::Post) => true,
            (ConfigMethod::Delete, HttpMethod::Delete) => true,
            _ => false,
        });
        if !method_allowed {
            return Ok(self.error_response(405));
        }

        // 5. Handle redirection
        if let Some(redirect_url) = &route.redirection {
            let mut res = HttpResponse::new(HttpStatus::MovedPermanently);
            res.set_header("Location", redirect_url);
            res.set_header("Content-Length", "0");
            return Ok(res);
        }

        // 6. Handle file upload (POST to route with upload_store)
        if req.method == HttpMethod::Post {
            if let Some(store) = &route.upload_store {
                return Ok(self.handle_upload(req, store));
            }
        }

        // 7. Serve static files
        self.serve_static(req, route)
    }

    fn handle_upload(&self, req: &HttpRequest, store: &str) -> HttpResponse {
        // Ensure the store directory exists
        if let Err(e) = std::fs::create_dir_all(store) {
            eprintln!("Cannot create upload dir {}: {}", store, e);
            return self.error_response(500);
        }

        let content_type = req.headers.get("content-type").map(|s| s.as_str()).unwrap_or("");

        if content_type.contains("multipart/form-data") {
            // Extract boundary
            let boundary = match content_type.split("boundary=").nth(1) {
                Some(b) => b.trim().to_string(),
                None => return self.error_response(400),
            };

            // Very simple multipart parser: find each part by boundary
            let delimiter = format!("--{}", boundary);
            let body_str = String::from_utf8_lossy(&req.body);
            let mut saved = 0;

            for part in body_str.split(&delimiter) {
                if part.trim().is_empty() || part.trim() == "--" {
                    continue;
                }
                // Part has headers then double CRLF then content
                if let Some(headers_end) = part.find("\r\n\r\n") {
                    let headers_part = &part[..headers_end];
                    let content_part = &part[headers_end + 4..];
                    // Remove trailing \r\n before next boundary
                    let content_part = content_part.trim_end_matches("\r\n");

                    // Extract filename from Content-Disposition
                    let filename = headers_part
                        .lines()
                        .find(|l| l.to_lowercase().starts_with("content-disposition"))
                        .and_then(|l| {
                            l.split("filename=").nth(1).map(|f| {
                                f.trim().trim_matches('"').to_string()
                            })
                        });

                    let filename = filename.unwrap_or_else(|| {
                        format!("upload_{}", std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis())
                            .unwrap_or(0))
                    });

                    // Sanitize: prevent path traversal
                    let safe_name = Path::new(&filename)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "upload".to_string());

                    let dest = PathBuf::from(store).join(&safe_name);
                    if std::fs::write(&dest, content_part.as_bytes()).is_ok() {
                        saved += 1;
                    }
                }
            }

            if saved > 0 {
                let mut res = HttpResponse::new(HttpStatus::Created);
                let body = format!("{{\"saved\":{}}}", saved);
                res.set_header("Content-Type", "application/json");
                res.set_body(body.into_bytes());
                return res;
            }
            return self.error_response(400);
        }

        // For non-multipart: save raw body directly
        let filename = format!("upload_{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0));
        let dest = PathBuf::from(store).join(&filename);
        match std::fs::write(&dest, &req.body) {
            Ok(_) => {
                let mut res = HttpResponse::new(HttpStatus::Created);
                res.set_header("Content-Type", "application/json");
                res.set_body(format!("{{\"file\":\"{}\"}}", filename).into_bytes());
                res
            }
            Err(_) => self.error_response(500),
        }
    }

    fn serve_static(&self, req: &HttpRequest, route: &RouteConfig) -> Result<HttpResponse, CgiInfo> {
        let root = match &route.root_directory {
            Some(r) => r.clone(),
            None => return Ok(self.error_response(500)),
        };

        // Build the filesystem path
        // Strip the route prefix from the path, then append to root
        let suffix = req.path.strip_prefix(&route.path).unwrap_or("");
        let suffix = suffix.trim_start_matches('/');

        let mut fs_path = PathBuf::from(&root);
        if !suffix.is_empty() {
            fs_path.push(suffix);
        }

        // If path is a directory: try index file, then directory listing
        if fs_path.is_dir() {
            if let Some(index) = &route.index_file {
                let index_path = fs_path.join(index);
                if index_path.is_file() {
                    let mut res = self.serve_file(&index_path);
                    res.set_cookie("last_visit", "12345", "Path=/; Max-Age=3600");
                    return Ok(res);
                }
            }
            if route.directory_listing {
                return Ok(self.serve_directory_listing(&fs_path, &req.path));
            }
            return Ok(self.error_response(403));
        }

        if fs_path.is_file() {
            // Handle DELETE method
            if req.method == crate::http::request::HttpMethod::Delete {
                match std::fs::remove_file(&fs_path) {
                    Ok(_) => {
                        let res = HttpResponse::new(HttpStatus::NoContent);
                        return Ok(res);
                    }
                    Err(_) => return Ok(self.error_response(404)),
                }
            }

            // Check for CGI
            if let Some(ext) = fs_path.extension().and_then(|e| e.to_str()) {
                if let Some(cgi_ext) = &route.cgi_extension {
                    if ext == cgi_ext.trim_start_matches('.') {
                        return self.serve_cgi(&fs_path, req);
                    }
                }
            }
            return Ok(self.serve_file(&fs_path));
        }

        Ok(self.error_response(404))
    }

    fn serve_cgi(&self, path: &Path, req: &HttpRequest) -> Result<HttpResponse, CgiInfo> {
        let script_path = path.to_string_lossy();
        let method_str = match &req.method {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Unknown(s) => s.as_str(),
        };

        match spawn_cgi(
            &script_path,
            method_str,
            &req.path,
            &req.query,
            &req.headers,
            &req.body,
        ) {
            Ok(info) => Err(info),
            Err(e) => {
                eprintln!("CGI Spawn error: {}", e);
                Ok(self.error_response(500))
            }
        }
    }

    fn serve_file(&self, path: &Path) -> HttpResponse {
        match std::fs::read(path) {
            Ok(content) => {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("bin");
                let content_type = mime::from_extension(ext);
                let mut res = HttpResponse::new(HttpStatus::Ok);
                res.set_header("Content-Type", content_type);
                res.set_body(content);
                res
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                self.error_response(403)
            }
            Err(_) => self.error_response(404),
        }
    }

    fn serve_directory_listing(&self, path: &Path, uri: &str) -> HttpResponse {
        let mut entries = String::new();
        if let Ok(read_dir) = std::fs::read_dir(path) {
            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let display = if is_dir {
                    format!("{}/", name)
                } else {
                    name.clone()
                };
                let href = format!(
                    "{}/{}{}",
                    uri.trim_end_matches('/'),
                    name,
                    if is_dir { "/" } else { "" }
                );
                entries.push_str(&format!(
                    "  <li><a href=\"{}\">{}</a></li>\n",
                    href, display
                ));
            }
        }

        let html = format!(
            "<!DOCTYPE html>\n<html>\n<head>\
            <meta charset=\"utf-8\"><title>Index of {}</title>\n\
            <style>body{{font-family:monospace;padding:1rem}}li{{margin:.3rem 0}}</style>\
            </head>\n<body>\n<h1>Index of {}</h1>\n<ul>\n{}</ul>\n</body>\n</html>\n",
            uri, uri, entries
        );

        let mut res = HttpResponse::new(HttpStatus::Ok);
        res.set_header("Content-Type", "text/html; charset=utf-8");
        res.set_body(html.into_bytes());
        res
    }

    /// Build a response for a given error code, using custom error pages if configured
    fn error_response(&self, code: u16) -> HttpResponse {
        let status = match code {
            400 => HttpStatus::BadRequest,
            403 => HttpStatus::Forbidden,
            404 => HttpStatus::NotFound,
            405 => HttpStatus::MethodNotAllowed,
            413 => HttpStatus::PayloadTooLarge,
            500 => HttpStatus::InternalServerError,
            501 => HttpStatus::NotImplemented,
            _ => HttpStatus::InternalServerError,
        };

        // Try custom error page
        if let Some(page_path) = self.error_pages.get(&code) {
            if let Ok(content) = std::fs::read(page_path.trim_start_matches('/')) {
                let mut res = HttpResponse::new(status);
                res.set_header("Content-Type", "text/html; charset=utf-8");
                res.set_body(content);
                return res;
            }
            // Try absolute path
            if let Ok(content) = std::fs::read(page_path) {
                let mut res = HttpResponse::new(status);
                res.set_header("Content-Type", "text/html; charset=utf-8");
                res.set_body(content);
                return res;
            }
        }

        // Fallback: built-in error page
        let html = format!(
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
            <title>{} {}</title>\
            <style>body{{font-family:sans-serif;text-align:center;padding:4rem}}\
            h1{{font-size:4rem;color:#c0392b}}p{{color:#555}}</style>\
            </head><body><h1>{}</h1><p>{}</p><hr><p>localhost/1.0</p></body></html>",
            code,
            status.reason_phrase(),
            code,
            status.reason_phrase()
        );
        let mut res = HttpResponse::new(status);
        res.set_header("Content-Type", "text/html; charset=utf-8");
        res.set_body(html.into_bytes());
        res
    }
}

pub fn spawn_cgi(
    script_path: &str,
    method: &str,
    path: &str,
    query: &str,
    headers: &std::collections::HashMap<String, String>,
    body: &[u8],
) -> std::io::Result<CgiInfo> {
    eprintln!("[CGI] spawn_cgi called for script: {}", script_path);
    
    let mut pipe_in: [c_int; 2] = [0; 2];
    let mut pipe_out: [c_int; 2] = [0; 2];

    unsafe {
        if libc::pipe(pipe_in.as_mut_ptr()) < 0 || libc::pipe(pipe_out.as_mut_ptr()) < 0 {
            eprintln!("[CGI] Failed to create pipes: {}", std::io::Error::last_os_error());
            return Err(std::io::Error::last_os_error());
        }
        eprintln!("[CGI] Pipes created successfully");

        let pid = libc::fork();
        if pid < 0 {
            libc::close(pipe_in[0]);
            libc::close(pipe_in[1]);
            libc::close(pipe_out[0]);
            libc::close(pipe_out[1]);
            eprintln!("[CGI] Fork failed: {}", std::io::Error::last_os_error());
            return Err(std::io::Error::last_os_error());
        }

        if pid == 0 { 
            // Child process
            eprintln!("[CGI] Child process created (PID: {})", pid);
            libc::close(pipe_in[1]);
            libc::close(pipe_out[0]);

            libc::dup2(pipe_in[0], libc::STDIN_FILENO);
            libc::close(pipe_in[0]);

            libc::dup2(pipe_out[1], libc::STDOUT_FILENO);
            libc::close(pipe_out[1]);

            let mut envs = Vec::new();
            envs.push(CString::new(format!("REQUEST_METHOD={}", method).to_uppercase()).unwrap());
            envs.push(CString::new(format!("PATH_INFO={}", path)).unwrap());
            envs.push(CString::new("SERVER_PROTOCOL=HTTP/1.1").unwrap());
            envs.push(CString::new("GATEWAY_INTERFACE=CGI/1.1").unwrap());
            
            if let Some(content_length) = headers.get("content-length") {
                envs.push(CString::new(format!("CONTENT_LENGTH={}", content_length)).unwrap());
            } else if !body.is_empty() {
                envs.push(CString::new(format!("CONTENT_LENGTH={}", body.len())).unwrap());
            }
            
            if let Some(content_type) = headers.get("content-type") {
                envs.push(CString::new(format!("CONTENT_TYPE={}", content_type)).unwrap());
            }
            
            envs.push(CString::new(format!("QUERY_STRING={}", query)).unwrap());

            for (key, value) in headers {
                let env_key = key.to_uppercase().replace("-", "_");
                envs.push(CString::new(format!("HTTP_{}={}", env_key, value)).unwrap());
            }

            let mut envp: Vec<*const c_char> = envs.iter().map(|s| s.as_ptr()).collect();
            envp.push(ptr::null());

            let arg0 = CString::new(script_path).unwrap();
            let argv: [*const c_char; 2] = [arg0.as_ptr(), ptr::null()];

            eprintln!("[CGI] Attempting execve for: {}", script_path);
            eprintln!("[CGI] Method={}, Path={}, Query={}", method, path, query);
            libc::execve(arg0.as_ptr(), argv.as_ptr(), envp.as_ptr());
            eprintln!("[CGI] execve failed, exiting child with error");
            libc::_exit(1);
        } else {
            // Parent process
            eprintln!("[CGI] Parent process continuing, child PID: {}", pid);
            libc::close(pipe_in[0]);
            libc::close(pipe_out[1]);

            set_nonblocking_cgi(pipe_in[1])?;
            set_nonblocking_cgi(pipe_out[0])?;
            eprintln!("[CGI] Non-blocking flags set on pipes");

            Ok(CgiInfo { pid, stdin_fd: pipe_in[1], stdout_fd: pipe_out[0] })
        }
    }
}

fn set_nonblocking_cgi(fd: i32) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let res = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if res < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
