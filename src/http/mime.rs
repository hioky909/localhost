/// Return the MIME type string for a given file extension.
pub fn from_extension(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css"          => "text/css",
        "js"           => "application/javascript",
        "json"         => "application/json",
        "png"          => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif"          => "image/gif",
        "svg"          => "image/svg+xml",
        "ico"          => "image/x-icon",
        "txt"          => "text/plain; charset=utf-8",
        "pdf"          => "application/pdf",
        "mp4"          => "video/mp4",
        _              => "application/octet-stream",
    }
}
