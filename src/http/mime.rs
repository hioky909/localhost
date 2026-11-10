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

#[cfg(test)]
mod tests {
    use super::from_extension;

    #[test]
    fn types_courants() {
        assert_eq!(from_extension("html"), "text/html; charset=utf-8");
        assert_eq!(from_extension("htm"), "text/html; charset=utf-8");
        assert_eq!(from_extension("css"), "text/css");
        assert_eq!(from_extension("js"), "application/javascript");
        assert_eq!(from_extension("json"), "application/json");
        assert_eq!(from_extension("txt"), "text/plain; charset=utf-8");
        assert_eq!(from_extension("pdf"), "application/pdf");
        assert_eq!(from_extension("mp4"), "video/mp4");
    }

    #[test]
    fn images() {
        assert_eq!(from_extension("png"), "image/png");
        assert_eq!(from_extension("jpg"), "image/jpeg");
        assert_eq!(from_extension("jpeg"), "image/jpeg");
        assert_eq!(from_extension("gif"), "image/gif");
        assert_eq!(from_extension("svg"), "image/svg+xml");
        assert_eq!(from_extension("ico"), "image/x-icon");
    }

    #[test]
    fn insensible_a_la_casse() {
        assert_eq!(from_extension("HTML"), "text/html; charset=utf-8");
        assert_eq!(from_extension("JpG"), "image/jpeg");
    }

    #[test]
    fn inconnu_en_octet_stream() {
        assert_eq!(from_extension("exe"), "application/octet-stream");
        assert_eq!(from_extension(""), "application/octet-stream");
        assert_eq!(from_extension("tar.gz"), "application/octet-stream");
    }
}
