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
            crate::log::error("upload", &format!("cannot create {}: {}", store, e));
            return self.error_response(500);
        }

        let content_type = req.headers.get("content-type").map(|s| s.as_str()).unwrap_or("");

        if content_type.contains("multipart/form-data") {
            // Extract boundary
            let boundary = match content_type.split("boundary=").nth(1) {
                Some(b) => b.trim().to_string(),
                None => return self.error_response(400),
            };

            // Analyse multipart au niveau des octets : les fichiers binaires restent intacts.
            fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
                if needle.is_empty() || from >= hay.len() {
                    return None;
                }
                hay[from..].windows(needle.len()).position(|w| w == needle).map(|i| i + from)
            }
            let delimiter = format!("--{}", boundary.trim_matches('"')).into_bytes();
            let body = &req.body[..];
            let mut saved = 0;
            let mut pos = match find(body, &delimiter, 0) {
                Some(p) => p + delimiter.len(),
                None => return self.error_response(400),
            };
            loop {
                // Fin du message : « --boundary-- »
                if body[pos..].starts_with(b"--") {
                    break;
                }
                let next = match find(body, &delimiter, pos) {
                    Some(n) => n,
                    None => break,
                };
                let part = &body[pos..next];
                pos = next + delimiter.len();
                let part = part.strip_prefix(b"\r\n").unwrap_or(part);
                let headers_end = match find(part, b"\r\n\r\n", 0) {
                    Some(h) => h,
                    None => continue,
                };
                let headers_part = String::from_utf8_lossy(&part[..headers_end]);
                let mut content = &part[headers_end + 4..];
                content = content.strip_suffix(b"\r\n").unwrap_or(content);

                let filename = headers_part
                    .lines()
                    .find(|l| l.to_lowercase().starts_with("content-disposition"))
                    .and_then(|l| l.split("filename=").nth(1).map(|f| f.split(';').next().unwrap_or("").trim().trim_matches('"').to_string()))
                    .filter(|f| !f.is_empty());

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
                if std::fs::write(&dest, content).is_ok() {
                    saved += 1;
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
                crate::log::error("cgi", &format!("spawn failed: {}", e));
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
        fn esc(s: &str) -> String {
            s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
        }
        fn size(n: u64) -> String {
            if n < 1024 { format!("{} B", n) } else if n < 1024 * 1024 { format!("{:.1} KB", n as f64 / 1024.0) } else { format!("{:.1} MB", n as f64 / 1048576.0) }
        }
        fn date(t: std::time::SystemTime) -> String {
            let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as libc::time_t).unwrap_or(0);
            let mut tm: libc::tm = unsafe { std::mem::zeroed() };
            unsafe { libc::localtime_r(&secs, &mut tm) };
            format!("{:04}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min)
        }

        let mut items: Vec<(bool, String, u64, String)> = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(path) {
            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let meta = entry.metadata().ok();
                let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let modified = meta.and_then(|m| m.modified().ok()).map(date).unwrap_or_default();
                items.push((is_dir, name, len, modified));
            }
        }
        // Dossiers d'abord, puis ordre alphabétique
        items.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.to_lowercase().cmp(&b.1.to_lowercase())));

        let base = uri.trim_end_matches('/');
        let mut rows = String::new();
        if !base.is_empty() {
            rows.push_str("<tr><td><a href=\"../\">../</a></td><td></td><td></td></tr>\n");
        }
        for (is_dir, name, len, modified) in &items {
            let href = format!("{}/{}{}", base, name, if *is_dir { "/" } else { "" });
            rows.push_str(&format!(
                "<tr><td><a href=\"{}\">{}{}</a></td><td>{}</td><td>{}</td></tr>\n",
                esc(&href),
                esc(name),
                if *is_dir { "/" } else { "" },
                if *is_dir { "—".to_string() } else { size(*len) },
                modified
            ));
        }

        let html = format!(
            "<!DOCTYPE html>\n<html lang=\"fr\">\n<head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
            <title>Index de {uri}</title>\n<style>\
            body{{margin:0;background:#0d0f12;color:#e8e6e3;font:14px/1.6 ui-monospace,'JetBrains Mono',Menlo,monospace}}\
            main{{max-width:900px;margin:0 auto;padding:48px 28px}}h1{{font-size:22px;margin:0 0 6px}}\
            p{{color:#8b9099;margin:0 0 24px}}table{{width:100%;border-collapse:collapse}}\
            th{{text-align:left;color:#8b9099;font-weight:500;border-bottom:1px solid #262a31;padding:8px 10px}}\
            td{{padding:8px 10px;border-bottom:1px solid #1b1e24}}td:nth-child(2),td:nth-child(3){{color:#8b9099;white-space:nowrap}}\
            tr:hover td{{background:#15181d}}a{{color:#e8e6e3;text-decoration:none}}a:hover{{color:#ff8a3d}}\
            footer{{margin-top:28px;color:#4b5059;font-size:12px}}</style>\
            </head>\n<body><main>\n<h1>Index de {uri}</h1><p>{count} élément(s)</p>\n\
            <table><thead><tr><th>Nom</th><th>Taille</th><th>Modifié</th></tr></thead><tbody>\n{rows}</tbody></table>\n\
            <footer>◆ localhost · autoindex</footer></main></body>\n</html>\n",
            uri = esc(uri),
            count = items.len(),
            rows = rows
        );

        let mut res = HttpResponse::new(HttpStatus::Ok);
        res.set_header("Content-Type", "text/html; charset=utf-8");
        res.set_body(html.into_bytes());
        res
    }

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
    
    let mut pipe_in: [c_int; 2] = [0; 2];
    let mut pipe_out: [c_int; 2] = [0; 2];

    unsafe {
        if libc::pipe(pipe_in.as_mut_ptr()) < 0 || libc::pipe(pipe_out.as_mut_ptr()) < 0 {
            crate::log::error("cgi", &format!("pipe() failed: {}", std::io::Error::last_os_error()));
            return Err(std::io::Error::last_os_error());
        }

        let pid = libc::fork();
        if pid < 0 {
            libc::close(pipe_in[0]);
            libc::close(pipe_in[1]);
            libc::close(pipe_out[0]);
            libc::close(pipe_out[1]);
            crate::log::error("cgi", &format!("fork() failed: {}", std::io::Error::last_os_error()));
            return Err(std::io::Error::last_os_error());
        }

        if pid == 0 { 
            // Child process
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
            envs.push(CString::new(format!("SCRIPT_NAME={}", path)).unwrap());
            envs.push(CString::new(format!("SERVER_SOFTWARE=localhost/{}", env!("CARGO_PKG_VERSION"))).unwrap());
            // Sans PATH, un shebang « #!/usr/bin/env python3 » ne trouve pas l'interpréteur.
            let host_path = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string());
            envs.push(CString::new(format!("PATH={}", host_path)).unwrap());
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

            libc::execve(arg0.as_ptr(), argv.as_ptr(), envp.as_ptr());
            libc::_exit(1);
        } else {
            // Parent process
            libc::close(pipe_in[0]);
            libc::close(pipe_out[1]);

            set_nonblocking_cgi(pipe_in[1])?;
            set_nonblocking_cgi(pipe_out[0])?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;

    #[test]
    fn upload_multipart_preserve_les_octets() {
        let dir = std::env::temp_dir().join(format!("lh-test-{}", std::process::id()));
        let store = dir.to_string_lossy().to_string();
        let payload: Vec<u8> = (0u8..=255).cycle().take(4096).collect(); // octets non UTF-8
        let mut body = b"--XyZ\r\nContent-Disposition: form-data; name=\"file\"; filename=\"../../bin.dat\"\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
        body.extend_from_slice(&payload);
        body.extend_from_slice(b"\r\n--XyZ--\r\n");
        let mut req = HttpRequest::new();
        req.method = HttpMethod::Post;
        req.headers.insert("content-type".into(), "multipart/form-data; boundary=XyZ".into());
        req.body = body;
        let router = Router::new(&ServerConfig::default());
        let res = router.handle_upload(&req, &store);
        assert_eq!(res.status, HttpStatus::Created);
        // Le nom est assaini (pas de sortie du dossier) et le contenu est intact
        assert_eq!(std::fs::read(dir.join("bin.dat")).unwrap(), payload);
        let _ = std::fs::remove_dir_all(dir);
    }
}
