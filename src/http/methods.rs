use std::path::Path;
use crate::config::model::{ServerConfig, LocationConfig};
use crate::http::request::HttpRequest;
use crate::http::response::HttpResponse;
use crate::utils::logger::{log_info, log_warn};

pub fn mime_type(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext.to_lowercase().as_str() {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "txt" => "text/plain",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn resolve_path(root: &str, req_path: &str) -> Option<std::path::PathBuf> {
    // Prevent path traversal
    let normalized = req_path.replace("//", "/");
    let rel = normalized.trim_start_matches('/');
    let candidate = Path::new(root).join(rel);
    let canonical_root = std::fs::canonicalize(root).ok()?;
    let canonical = std::fs::canonicalize(&candidate).ok();
    if let Some(ref c) = canonical {
        if !c.starts_with(&canonical_root) {
            return None;
        }
    }
    Some(candidate)
}

pub fn handle_get(req: &HttpRequest, server: &ServerConfig, location: &LocationConfig, root: &str) -> HttpResponse {
    let file_path = resolve_path(root, &req.path);
    let file_path = match file_path {
        Some(p) => p,
        None => return HttpResponse::error(403, "Forbidden"),
    };

    if file_path.is_dir() {
        // Try index file
        let index_path = file_path.join(&server.index);
        if index_path.is_file() {
            return serve_file(&index_path);
        }
        if location.autoindex {
            return generate_autoindex(&file_path, &req.path);
        }
        return HttpResponse::error(403, "Directory listing disabled");
    }

    if file_path.is_file() {
        return serve_file(&file_path);
    }

    HttpResponse::error(404, "Not Found")
}

pub fn handle_head(req: &HttpRequest, server: &ServerConfig, location: &LocationConfig, root: &str) -> HttpResponse {
    let mut resp = handle_get(req, server, location, root);
    // Keep Content-Length as the actual body size (per RFC 7231), just clear the body
    resp.body = Vec::new();
    resp
}

pub fn handle_post(req: &HttpRequest, _server: &ServerConfig, location: &LocationConfig, _root: &str) -> HttpResponse {
    if let Some(upload_dir) = &location.upload_dir {
        // Save the body to a file
        let dir = Path::new(upload_dir);
        if !dir.exists() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                log_warn(&format!("Failed to create upload dir: {}", e));
                return HttpResponse::error(500, "Failed to create upload directory");
            }
        }
        // Use content-disposition or fallback name
        let filename = req.headers.get("content-disposition")
            .and_then(|h| {
                h.split(';').find_map(|part| {
                    let part = part.trim();
                    if part.starts_with("filename=") {
                        Some(part[9..].trim_matches('"').to_string())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| format!("upload_{}", timestamp_ms()));

        // Sanitize filename — reject dot-only names and names starting with a dot
        let safe_name: String = filename.chars()
            .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
            .collect();
        let safe_name = if safe_name.is_empty()
            || safe_name.chars().all(|c| c == '.')
            || safe_name.starts_with('.')
        {
            format!("upload_{}", timestamp_ms())
        } else {
            safe_name
        };

        let target = dir.join(&safe_name);
        // Path traversal check
        let canonical_dir = match std::fs::canonicalize(dir) {
            Ok(d) => d,
            Err(_) => return HttpResponse::error(500, "Upload dir error"),
        };
        // Write the file
        match std::fs::write(&target, &req.body) {
            Ok(_) => {
                // Verify after write
                if let Ok(c) = std::fs::canonicalize(&target) {
                    if !c.starts_with(&canonical_dir) {
                        let _ = std::fs::remove_file(&target);
                        return HttpResponse::error(403, "Forbidden");
                    }
                }
                log_info(&format!("Uploaded file: {}", safe_name));
                HttpResponse::new(201).with_body(b"Created".to_vec(), "text/plain")
            }
            Err(e) => {
                log_warn(&format!("Upload failed: {}", e));
                HttpResponse::error(500, "Upload failed")
            }
        }
    } else {
        // No upload dir, just 200 ok for POST
        HttpResponse::new(200).with_body(b"OK".to_vec(), "text/plain")
    }
}

pub fn handle_delete(req: &HttpRequest, _server: &ServerConfig, _location: &LocationConfig, root: &str) -> HttpResponse {
    let file_path = match resolve_path(root, &req.path) {
        Some(p) => p,
        None => return HttpResponse::error(403, "Forbidden"),
    };

    if !file_path.exists() {
        return HttpResponse::error(404, "Not Found");
    }
    if file_path.is_dir() {
        return HttpResponse::error(403, "Cannot delete directory");
    }

    match std::fs::remove_file(&file_path) {
        Ok(_) => {
            log_info(&format!("Deleted: {}", file_path.display()));
            HttpResponse::new(204)
        }
        Err(e) => {
            log_warn(&format!("Delete failed: {}", e));
            HttpResponse::error(500, "Delete failed")
        }
    }
}

fn serve_file(path: &Path) -> HttpResponse {
    match std::fs::read(path) {
        Ok(data) => {
            let ct = mime_type(path.to_str().unwrap_or(""));
            HttpResponse::new(200).with_body(data, ct)
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            HttpResponse::error(403, "Forbidden")
        }
        Err(_) => HttpResponse::error(404, "Not Found"),
    }
}

fn generate_autoindex(dir: &Path, req_path: &str) -> HttpResponse {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return HttpResponse::error(500, "Cannot read directory"),
    };

    let mut html = format!(
        "<html><head><title>Index of {}</title></head><body><h1>Index of {}</h1><ul>",
        req_path, req_path
    );
    if req_path != "/" {
        html.push_str("<li><a href=\"../\">..</a></li>");
    }
    let mut items: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let suffix = if is_dir { "/" } else { "" };
        items.push(format!(
            "<li><a href=\"{}{}\">{}{}</a></li>",
            name_str, suffix, name_str, suffix
        ));
    }
    items.sort();
    for item in items {
        html.push_str(&item);
    }
    html.push_str("</ul></body></html>");
    HttpResponse::new(200).with_body(html.into_bytes(), "text/html")
}

fn timestamp_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
