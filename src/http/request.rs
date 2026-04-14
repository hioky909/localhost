use std::collections::HashMap;

pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub version: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub query_string: String,
}

pub enum ParseResult {
    Complete(HttpRequest),
    Incomplete,
    Error(u16),
}

pub fn parse_request(buf: &[u8]) -> ParseResult {
    // Find end of headers (\r\n\r\n)
    let header_end = match find_crlf_crlf(buf) {
        Some(pos) => pos,
        None => return ParseResult::Incomplete,
    };

    let header_bytes = &buf[..header_end];
    let header_str = match std::str::from_utf8(header_bytes) {
        Ok(s) => s,
        Err(_) => return ParseResult::Error(400),
    };

    let mut lines = header_str.split("\r\n");

    // Parse request line
    let request_line = match lines.next() {
        Some(l) => l,
        None => return ParseResult::Error(400),
    };

    let mut parts = request_line.splitn(3, ' ');
    let method = match parts.next() {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => return ParseResult::Error(400),
    };
    let raw_path = match parts.next() {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => return ParseResult::Error(400),
    };
    let version = match parts.next() {
        Some(v) if v.starts_with("HTTP/") => v.to_string(),
        _ => return ParseResult::Error(400),
    };

    // Split path and query string
    let (path, query_string) = if let Some(q) = raw_path.find('?') {
        (raw_path[..q].to_string(), raw_path[q+1..].to_string())
    } else {
        (raw_path, String::new())
    };

    // Parse headers
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_lowercase();
            let value = line[colon+1..].trim().to_string();
            headers.insert(key, value);
        } else {
            return ParseResult::Error(400);
        }
    }

    // Body
    let body_start = header_end + 4; // after \r\n\r\n

    // Check transfer-encoding: chunked
    let is_chunked = headers.get("transfer-encoding")
        .map(|v| v.to_lowercase().contains("chunked"))
        .unwrap_or(false);

    let body = if is_chunked {
        // Decode chunked body
        let chunk_data = &buf[body_start..];
        match decode_chunked(chunk_data) {
            Some(decoded) => decoded,
            None => return ParseResult::Incomplete,
        }
    } else {
        let content_length: usize = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        if buf.len() < body_start + content_length {
            return ParseResult::Incomplete;
        }
        buf[body_start..body_start + content_length].to_vec()
    };

    ParseResult::Complete(HttpRequest {
        method,
        path,
        version,
        headers,
        body,
        query_string,
    })
}

/// Decode HTTP chunked transfer encoding.
/// Returns None if the data is incomplete, Some(decoded) when a final chunk (0\r\n\r\n) is found.
fn decode_chunked(data: &[u8]) -> Option<Vec<u8>> {
    let mut result = Vec::new();
    let mut pos = 0;

    loop {
        // Find end of chunk size line
        let line_end = find_crlf(&data[pos..])?;
        let size_line = std::str::from_utf8(&data[pos..pos + line_end]).ok()?;
        // Strip chunk extensions (semicolon)
        let size_str = size_line.split(';').next().unwrap_or("").trim();
        let chunk_size = usize::from_str_radix(size_str, 16).ok()?;
        pos += line_end + 2; // skip \r\n

        if chunk_size == 0 {
            // Trailing CRLF
            return Some(result);
        }

        if pos + chunk_size + 2 > data.len() {
            return None; // Incomplete
        }

        result.extend_from_slice(&data[pos..pos + chunk_size]);
        pos += chunk_size + 2; // skip chunk data + \r\n
    }
}

fn find_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\r\n")
}

fn find_crlf_crlf(buf: &[u8]) -> Option<usize> {
    let needle = b"\r\n\r\n";
    buf.windows(4).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_get() {
        let raw = b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";
        match parse_request(raw) {
            ParseResult::Complete(req) => {
                assert_eq!(req.method, "GET");
                assert_eq!(req.path, "/");
                assert_eq!(req.version, "HTTP/1.1");
                assert_eq!(req.headers.get("host"), Some(&"localhost".to_string()));
            }
            _ => panic!("Expected Complete"),
        }
    }

    #[test]
    fn test_parse_post_with_body() {
        let raw = b"POST /submit HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\r\nhello";
        match parse_request(raw) {
            ParseResult::Complete(req) => {
                assert_eq!(req.method, "POST");
                assert_eq!(req.path, "/submit");
                assert_eq!(req.body, b"hello");
            }
            _ => panic!("Expected Complete"),
        }
    }

    #[test]
    fn test_parse_incomplete() {
        let raw = b"GET / HTTP/1.1\r\nHost: localh";
        match parse_request(raw) {
            ParseResult::Incomplete => {}
            _ => panic!("Expected Incomplete"),
        }
    }

    #[test]
    fn test_parse_malformed() {
        let raw = b"BADREQUEST\r\n\r\n";
        match parse_request(raw) {
            ParseResult::Error(400) => {}
            _ => panic!("Expected Error(400)"),
        }
    }

    #[test]
    fn test_parse_chunked_body() {
        // Chunked encoding: "5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"
        let raw = b"POST /upload HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        match parse_request(raw) {
            ParseResult::Complete(req) => {
                assert_eq!(req.method, "POST");
                assert_eq!(req.body, b"hello world");
            }
            _ => panic!("Expected Complete"),
        }
    }

    #[test]
    fn test_parse_query_string() {
        let raw = b"GET /search?q=hello&lang=en HTTP/1.1\r\nHost: localhost\r\n\r\n";
        match parse_request(raw) {
            ParseResult::Complete(req) => {
                assert_eq!(req.path, "/search");
                assert_eq!(req.query_string, "q=hello&lang=en");
            }
            _ => panic!("Expected Complete"),
        }
    }

    #[test]
    fn test_parse_incomplete_body() {
        // Content-Length says 10 but we only have 5 bytes of body
        let raw = b"POST /submit HTTP/1.1\r\nContent-Length: 10\r\n\r\nhello";
        match parse_request(raw) {
            ParseResult::Incomplete => {}
            _ => panic!("Expected Incomplete"),
        }
    }
}
