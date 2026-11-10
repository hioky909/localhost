#![allow(dead_code)]
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Delete,
    Unknown(String),
}

impl From<&str> for HttpMethod {
    fn from(s: &str) -> Self {
        match s {
            "GET" => HttpMethod::Get,
            "POST" => HttpMethod::Post,
            "DELETE" => HttpMethod::Delete,
            other => HttpMethod::Unknown(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpVersion {
    V1_1,
    Unknown(String),
}

impl From<&str> for HttpVersion {
    fn from(s: &str) -> Self {
        match s {
            "HTTP/1.1" => HttpVersion::V1_1,
            other => HttpVersion::Unknown(other.to_string()),
        }
    }
}

#[derive(Debug)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub uri: String,
    pub path: String,
    pub query: String,
    pub version: HttpVersion,
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn new() -> Self {
        Self {
            method: HttpMethod::Get,
            uri: String::new(),
            path: String::new(),
            query: String::new(),
            version: HttpVersion::V1_1,
            headers: HashMap::new(),
            cookies: HashMap::new(),
            body: Vec::new(),
        }
    }

    /// Parse a complete request from raw bytes
    /// En-têtes complets et « Expect: 100-continue » présent : le client attend notre feu vert.
    pub fn expects_continue(raw: &[u8]) -> bool {
        let end = match raw.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(i) => i,
            None => return false,
        };
        let head = String::from_utf8_lossy(&raw[..end]).to_lowercase();
        head.lines().any(|l| l.starts_with("expect:") && l.contains("100-continue"))
    }

    /// En-têtes reçus en entier mais inexploitables (ligne de requête incomplète, octets non
    /// UTF-8) : attendre davantage ne servirait à rien, il faut répondre 400 tout de suite.
    pub fn is_malformed(raw: &[u8]) -> bool {
        let end = match raw.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(i) => i,
            None => return false,
        };
        match std::str::from_utf8(&raw[..end]) {
            Err(_) => true,
            Ok(head) => head.lines().next().map(|l| l.split_whitespace().count() != 3).unwrap_or(true),
        }
    }

    pub fn parse(raw: &[u8]) -> Option<Self> {
        // Find double CRLF which separates headers from body
        let mut headers_end = 0;
        let mut found = false;
        
        for i in 0..raw.len().saturating_sub(3) {
            if raw[i] == b'\r' && raw[i+1] == b'\n' && raw[i+2] == b'\r' && raw[i+3] == b'\n' {
                headers_end = i;
                found = true;
                break;
            }
        }

        if !found {
            return None; // Incomplete request
        }

        let header_bytes = &raw[0..headers_end];
        let body_bytes = &raw[headers_end + 4..];

        let header_str = std::str::from_utf8(header_bytes).ok()?;
        let mut lines = header_str.lines();

        // Parse Request-Line: Method Request-URI HTTP-Version
        let request_line = lines.next()?;
        let mut parts = request_line.split_whitespace();
        let method = parts.next()?.into();
        let uri = parts.next()?.to_string();
        let version = parts.next()?.into();

        let (path, query) = if let Some(idx) = uri.find('?') {
            (uri[..idx].to_string(), uri[idx + 1..].to_string())
        } else {
            (uri.clone(), String::new())
        };

        let mut req = HttpRequest {
            method,
            uri,
            path,
            query,
            version,
            headers: HashMap::new(),
            cookies: HashMap::new(),
            body: Vec::new(),
        };

        // Parse Headers
        for line in lines {
            if line.is_empty() {
                break;
            }
            if let Some(colon_idx) = line.find(':') {
                let key = line[0..colon_idx].trim().to_string();
                let value = line[colon_idx + 1..].trim().to_string();
                // Store headers in lowercase for easier lookup later
                req.headers.insert(key.to_lowercase(), value);
            }
        }

        // Handle Transfer-Encoding: chunked
        let is_chunked = req.headers.get("transfer-encoding").map(|v| v.to_lowercase() == "chunked").unwrap_or(false);
        if is_chunked {
            let mut current_pos = 0;
            let mut complete = false;
            let mut decoded_body = Vec::new();
            while current_pos < body_bytes.len() {
                // Find CRLF after chunk size
                let mut size_end = current_pos;
                while size_end < body_bytes.len() && (size_end + 1 < body_bytes.len()) {
                    if body_bytes[size_end] == b'\r' && body_bytes[size_end+1] == b'\n' {
                        break;
                    }
                    size_end += 1;
                }
                
                if size_end + 1 >= body_bytes.len() { return None; } // Taille de bloc pas encore reçue
                
                let size_str = std::str::from_utf8(&body_bytes[current_pos..size_end]).ok()?;
                let chunk_size = usize::from_str_radix(size_str.trim(), 16).ok()?;
                
                if chunk_size == 0 {
                    complete = true;
                    break; // End of chunks
                }
                
                let chunk_start = size_end + 2;
                let chunk_end = chunk_start + chunk_size;
                
                if chunk_end > body_bytes.len() {
                    return None; // Incomplete chunk
                }
                
                decoded_body.extend_from_slice(&body_bytes[chunk_start..chunk_end]);
                current_pos = chunk_end + 2; // +2 for the CRLF after the chunk data
            }
            if !complete {
                return None; // Bloc terminal « 0 » pas encore reçu
            }
            req.body = decoded_body;
        } else {
            // Attendre tout le corps annoncé par Content-Length (il peut arriver en plusieurs paquets).
            match req.headers.get("content-length").and_then(|v| v.trim().parse::<usize>().ok()) {
                Some(expected) => {
                    if body_bytes.len() < expected {
                        return None;
                    }
                    req.body = body_bytes[..expected].to_vec();
                }
                None => req.body = body_bytes.to_vec(),
            }
        }

        // Parse cookies from the "cookie" header
        if let Some(cookie_header) = req.headers.get("cookie") {
            for pair in cookie_header.split(';') {
                let mut parts = pair.splitn(2, '=');
                if let (Some(name), Some(value)) = (parts.next(), parts.next()) {
                    req.cookies.insert(name.trim().to_string(), value.trim().to_string());
                }
            }
        }

        Some(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attend_tout_le_corps_annonce() {
        let partial = b"POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\n12345";
        assert!(HttpRequest::parse(partial).is_none());
        let full = b"POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\n1234567890";
        assert_eq!(HttpRequest::parse(full).unwrap().body, b"1234567890");
    }

    #[test]
    fn chunked_incomplet_puis_complet() {
        let partial = b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n";
        assert!(HttpRequest::parse(partial).is_none());
        let full = b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(HttpRequest::parse(full).unwrap().body, b"hello world");
    }

    fn req(raw: &str) -> HttpRequest {
        HttpRequest::parse(raw.as_bytes()).expect("requête complète")
    }

    #[test]
    fn analyse_la_ligne_de_requete() {
        let r = req("GET /index.html HTTP/1.1\r\nHost: a\r\n\r\n");
        assert_eq!(r.method, HttpMethod::Get);
        assert_eq!(r.uri, "/index.html");
        assert_eq!(r.path, "/index.html");
        assert_eq!(r.query, "");
        assert_eq!(r.version, HttpVersion::V1_1);
        assert!(r.body.is_empty());
    }

    #[test]
    fn reconnait_les_methodes() {
        assert_eq!(req("POST / HTTP/1.1\r\nContent-Length: 0\r\n\r\n").method, HttpMethod::Post);
        assert_eq!(req("DELETE /f HTTP/1.1\r\n\r\n").method, HttpMethod::Delete);
        assert_eq!(req("PATCH / HTTP/1.1\r\n\r\n").method, HttpMethod::Unknown("PATCH".into()));
        // Les méthodes HTTP sont sensibles à la casse (RFC 9110).
        assert_eq!(req("get / HTTP/1.1\r\n\r\n").method, HttpMethod::Unknown("get".into()));
    }

    #[test]
    fn version_inconnue_conservee() {
        assert_eq!(req("GET / HTTP/1.0\r\n\r\n").version, HttpVersion::Unknown("HTTP/1.0".into()));
        assert_eq!(req("GET / HTTP/2\r\n\r\n").version, HttpVersion::Unknown("HTTP/2".into()));
    }

    #[test]
    fn separe_chemin_et_query_string() {
        let r = req("GET /cgi/hello.py?nom=Johan&x=1 HTTP/1.1\r\n\r\n");
        assert_eq!(r.path, "/cgi/hello.py");
        assert_eq!(r.query, "nom=Johan&x=1");
        assert_eq!(r.uri, "/cgi/hello.py?nom=Johan&x=1");
        // Seul le premier « ? » sépare : le reste appartient à la query.
        assert_eq!(req("GET /a?b?c HTTP/1.1\r\n\r\n").query, "b?c");
    }

    #[test]
    fn en_tetes_insensibles_a_la_casse_et_nettoyes() {
        let r = req("GET / HTTP/1.1\r\nHOST:   exemple.local  \r\nX-Custom-Header: a:b:c\r\n\r\n");
        assert_eq!(r.headers.get("host").map(String::as_str), Some("exemple.local"));
        // La valeur peut contenir des « : » (seul le premier sépare le nom).
        assert_eq!(r.headers.get("x-custom-header").map(String::as_str), Some("a:b:c"));
    }

    #[test]
    fn ignore_les_lignes_d_en_tete_sans_deux_points() {
        let r = req("GET / HTTP/1.1\r\nligne-invalide\r\nHost: a\r\n\r\n");
        assert_eq!(r.headers.len(), 1);
    }

    #[test]
    fn en_tetes_incomplets_donnent_none() {
        assert!(HttpRequest::parse(b"").is_none());
        assert!(HttpRequest::parse(b"GET / HTTP/1.1\r\nHost: a\r\n").is_none());
        assert!(HttpRequest::parse(b"GET / HTTP/1.1\r\nHost: a\r\n\r").is_none());
    }

    #[test]
    fn ligne_de_requete_invalide_donne_none() {
        assert!(HttpRequest::parse(b"\r\n\r\n").is_none());
        assert!(HttpRequest::parse(b"GET\r\n\r\n").is_none());
        assert!(HttpRequest::parse(b"GET /\r\n\r\n").is_none());
    }

    #[test]
    fn en_tetes_non_utf8_refuses() {
        assert!(HttpRequest::parse(b"GET /\xff HTTP/1.1\r\n\r\n").is_none());
    }

    #[test]
    fn corps_binaire_conserve_tel_quel() {
        let mut raw = b"POST /up HTTP/1.1\r\nContent-Length: 4\r\n\r\n".to_vec();
        raw.extend_from_slice(&[0x00, 0xff, 0x0d, 0x0a]);
        assert_eq!(HttpRequest::parse(&raw).unwrap().body, vec![0x00, 0xff, 0x0d, 0x0a]);
    }

    #[test]
    fn octets_en_trop_ignores_apres_content_length() {
        // Début d'une requête suivante (pipelining) : le corps s'arrête à Content-Length.
        let r = req("POST / HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcGET / HTTP/1.1");
        assert_eq!(r.body, b"abc");
    }

    #[test]
    fn content_length_illisible_prend_tout_le_reste() {
        let r = req("POST / HTTP/1.1\r\nContent-Length: dix\r\n\r\nabc");
        assert_eq!(r.body, b"abc");
    }

    #[test]
    fn content_length_zero_donne_un_corps_vide() {
        assert!(req("POST / HTTP/1.1\r\nContent-Length: 0\r\n\r\n").body.is_empty());
    }

    #[test]
    fn chunked_insensible_a_la_casse_et_tailles_hexadecimales() {
        let body = "a".repeat(26);
        let raw = format!("POST / HTTP/1.1\r\nTransfer-Encoding: CHUNKED\r\n\r\n1A\r\n{}\r\n0\r\n\r\n", body);
        assert_eq!(req(&raw).body, body.as_bytes());
    }

    #[test]
    fn chunked_corps_vide() {
        assert!(req("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n").body.is_empty());
    }

    #[test]
    fn chunked_bloc_tronque_donne_none() {
        assert!(HttpRequest::parse(b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\na\r\nabc").is_none());
    }

    #[test]
    fn chunked_taille_invalide_donne_none() {
        assert!(HttpRequest::parse(b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\nabc\r\n0\r\n\r\n").is_none());
    }

    #[test]
    fn chunked_prioritaire_sur_content_length() {
        let r = req("POST / HTTP/1.1\r\nContent-Length: 100\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nok\r\n0\r\n\r\n");
        assert_eq!(r.body, b"ok");
    }

    #[test]
    fn analyse_les_cookies() {
        let r = req("GET / HTTP/1.1\r\nCookie: session=abc123; theme=sombre ;  vide=\r\n\r\n");
        assert_eq!(r.cookies.get("session").map(String::as_str), Some("abc123"));
        assert_eq!(r.cookies.get("theme").map(String::as_str), Some("sombre"));
        assert_eq!(r.cookies.get("vide").map(String::as_str), Some(""));
    }

    #[test]
    fn cookie_avec_signe_egal_dans_la_valeur() {
        let r = req("GET / HTTP/1.1\r\nCookie: jeton=a=b==\r\n\r\n");
        assert_eq!(r.cookies.get("jeton").map(String::as_str), Some("a=b=="));
    }

    #[test]
    fn expect_continue_attend_la_fin_des_en_tetes() {
        assert!(!HttpRequest::expects_continue(b"POST / HTTP/1.1\r\nExpect: 100-continue\r\n"));
        assert!(HttpRequest::expects_continue(b"POST / HTTP/1.1\r\nEXPECT: 100-Continue\r\n\r\n"));
    }

    #[test]
    fn detecte_les_requetes_malformees() {
        assert!(HttpRequest::is_malformed(b"GET\r\n\r\n"));
        assert!(HttpRequest::is_malformed(b"\r\n\r\n"));
        assert!(HttpRequest::is_malformed(b"GET / HTTP/1.1 en-trop\r\n\r\n"));
        assert!(HttpRequest::is_malformed(b"\x00\xff\xfe x\r\n\r\n"));
    }

    #[test]
    fn requete_valide_ou_incomplete_n_est_pas_malformee() {
        assert!(!HttpRequest::is_malformed(b"GET / HTTP/1.1\r\nHost: a\r\n\r\n"));
        assert!(!HttpRequest::is_malformed(b"GET"));
        // Corps encore attendu : incomplet, pas malformé.
        assert!(!HttpRequest::is_malformed(b"POST / HTTP/1.1\r\nContent-Length: 9\r\n\r\nab"));
    }

    #[test]
    fn nouvelle_requete_par_defaut() {
        let r = HttpRequest::new();
        assert_eq!(r.method, HttpMethod::Get);
        assert_eq!(r.version, HttpVersion::V1_1);
        assert!(r.headers.is_empty() && r.cookies.is_empty() && r.body.is_empty());
    }

    #[test]
    fn detecte_expect_100_continue() {
        assert!(HttpRequest::expects_continue(b"POST / HTTP/1.1\r\nExpect: 100-continue\r\nContent-Length: 9\r\n\r\n"));
        assert!(!HttpRequest::expects_continue(b"POST / HTTP/1.1\r\nContent-Length: 9\r\n\r\n"));
    }
}
