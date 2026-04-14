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
    let content_length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    if buf.len() < body_start + content_length {
        return ParseResult::Incomplete;
    }

    let body = buf[body_start..body_start + content_length].to_vec();

    ParseResult::Complete(HttpRequest {
        method,
        path,
        version,
        headers,
        body,
        query_string,
    })
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
}
