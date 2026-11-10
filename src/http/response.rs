#![allow(dead_code)]
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpStatus {
    Ok = 200,
    Created = 201,
    Accepted = 202,
    NoContent = 204,
    MovedPermanently = 301,
    Found = 302,
    BadRequest = 400,
    Forbidden = 403,
    NotFound = 404,
    MethodNotAllowed = 405,
    PayloadTooLarge = 413,
    InternalServerError = 500,
    NotImplemented = 501,
    BadGateway = 502,
    HttpVersionNotSupported = 505,
}

impl HttpStatus {
    /// Statut correspondant à un code numérique, s'il est géré par le serveur.
    pub fn from_code(code: u16) -> Option<HttpStatus> {
        Some(match code {
            200 => HttpStatus::Ok,
            201 => HttpStatus::Created,
            202 => HttpStatus::Accepted,
            204 => HttpStatus::NoContent,
            301 => HttpStatus::MovedPermanently,
            302 => HttpStatus::Found,
            400 => HttpStatus::BadRequest,
            403 => HttpStatus::Forbidden,
            404 => HttpStatus::NotFound,
            405 => HttpStatus::MethodNotAllowed,
            413 => HttpStatus::PayloadTooLarge,
            500 => HttpStatus::InternalServerError,
            501 => HttpStatus::NotImplemented,
            502 => HttpStatus::BadGateway,
            505 => HttpStatus::HttpVersionNotSupported,
            _ => return None,
        })
    }

    pub fn reason_phrase(&self) -> &'static str {
        match self {
            HttpStatus::Ok => "OK",
            HttpStatus::Created => "Created",
            HttpStatus::Accepted => "Accepted",
            HttpStatus::NoContent => "No Content",
            HttpStatus::MovedPermanently => "Moved Permanently",
            HttpStatus::Found => "Found",
            HttpStatus::BadRequest => "Bad Request",
            HttpStatus::Forbidden => "Forbidden",
            HttpStatus::NotFound => "Not Found",
            HttpStatus::MethodNotAllowed => "Method Not Allowed",
            HttpStatus::PayloadTooLarge => "Payload Too Large",
            HttpStatus::InternalServerError => "Internal Server Error",
            HttpStatus::NotImplemented => "Not Implemented",
            HttpStatus::BadGateway => "Bad Gateway",
            HttpStatus::HttpVersionNotSupported => "HTTP Version Not Supported",
        }
    }
}

#[derive(Clone)]
pub struct HttpResponse {
    pub version: String,
    pub status: HttpStatus,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: HttpStatus) -> Self {
        Self {
            version: "HTTP/1.1".to_string(),
            status,
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    pub fn set_header(&mut self, key: &str, value: &str) {
        self.headers.insert(key.to_string(), value.to_string());
    }

    pub fn set_body(&mut self, body: Vec<u8>) {
        self.set_header("Content-Length", &body.len().to_string());
        self.body = body;
    }

    pub fn set_cookie(&mut self, name: &str, value: &str, options: &str) {
        let cookie_str = if options.is_empty() {
            format!("{}={}", name, value)
        } else {
            format!("{}={}; {}", name, value, options)
        };
        // Note: In a real server, we might want multiple Set-Cookie headers.
        // For simplicity here, we append if already present.
        let existing = self.headers.get("Set-Cookie").cloned();
        match existing {
            Some(prev) => self.headers.insert("Set-Cookie".to_string(), format!("{}, {}", prev, cookie_str)),
            None => self.headers.insert("Set-Cookie".to_string(), cookie_str),
        };
    }

    /// Serializes the response into a raw byte vector ready to send over the socket
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut response = Vec::new();
        
        // Status line
        let status_line = format!(
            "{} {} {}\r\n",
            self.version,
            self.status.clone() as u16,
            self.status.reason_phrase()
        );
        response.extend_from_slice(status_line.as_bytes());

        // Headers
        for (key, value) in &self.headers {
            let header_line = format!("{}: {}\r\n", key, value);
            response.extend_from_slice(header_line.as_bytes());
        }

        // Empty line to separate headers from body
        response.extend_from_slice(b"\r\n");

        // Body
        response.extend_from_slice(&self.body);

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOUS: [HttpStatus; 15] = [
        HttpStatus::Ok, HttpStatus::Created, HttpStatus::Accepted, HttpStatus::NoContent,
        HttpStatus::MovedPermanently, HttpStatus::Found, HttpStatus::BadRequest, HttpStatus::Forbidden,
        HttpStatus::NotFound, HttpStatus::MethodNotAllowed, HttpStatus::PayloadTooLarge,
        HttpStatus::InternalServerError, HttpStatus::NotImplemented, HttpStatus::BadGateway,
        HttpStatus::HttpVersionNotSupported,
    ];

    fn texte(res: &HttpResponse) -> String {
        String::from_utf8(res.to_bytes()).unwrap()
    }

    #[test]
    fn codes_numeriques_conformes() {
        let codes: Vec<u16> = TOUS.iter().map(|s| s.clone() as u16).collect();
        assert_eq!(codes, vec![200, 201, 202, 204, 301, 302, 400, 403, 404, 405, 413, 500, 501, 502, 505]);
    }

    #[test]
    fn from_code_est_l_inverse_du_code_numerique() {
        for s in TOUS.iter() {
            assert_eq!(HttpStatus::from_code(s.clone() as u16).as_ref(), Some(s));
        }
        assert_eq!(HttpStatus::from_code(418), None);
        assert_eq!(HttpStatus::from_code(0), None);
    }

    #[test]
    fn chaque_statut_a_une_raison() {
        for s in TOUS.iter() {
            assert!(!s.reason_phrase().is_empty(), "{:?}", s);
        }
        assert_eq!(HttpStatus::NotFound.reason_phrase(), "Not Found");
        assert_eq!(HttpStatus::PayloadTooLarge.reason_phrase(), "Payload Too Large");
    }

    #[test]
    fn ligne_d_etat() {
        let res = HttpResponse::new(HttpStatus::NotFound);
        assert!(texte(&res).starts_with("HTTP/1.1 404 Not Found\r\n"));
    }

    #[test]
    fn reponse_vide_se_termine_par_une_ligne_vide() {
        assert_eq!(texte(&HttpResponse::new(HttpStatus::NoContent)), "HTTP/1.1 204 No Content\r\n\r\n");
    }

    #[test]
    fn set_body_met_a_jour_content_length() {
        let mut res = HttpResponse::new(HttpStatus::Ok);
        res.set_body(b"bonjour".to_vec());
        assert_eq!(res.headers.get("Content-Length").map(String::as_str), Some("7"));
        res.set_body(Vec::new());
        assert_eq!(res.headers.get("Content-Length").map(String::as_str), Some("0"));
    }

    #[test]
    fn serialise_en_tetes_et_corps() {
        let mut res = HttpResponse::new(HttpStatus::Ok);
        res.set_header("Content-Type", "text/plain");
        res.set_body(b"abc".to_vec());
        let t = texte(&res);
        assert!(t.contains("\r\nContent-Type: text/plain\r\n"));
        assert!(t.contains("\r\nContent-Length: 3\r\n"));
        assert!(t.ends_with("\r\n\r\nabc"));
    }

    #[test]
    fn corps_binaire_intact() {
        let mut res = HttpResponse::new(HttpStatus::Ok);
        let data: Vec<u8> = (0u8..=255).collect();
        res.set_body(data.clone());
        assert!(res.to_bytes().ends_with(&data));
    }

    #[test]
    fn set_header_remplace_la_valeur() {
        let mut res = HttpResponse::new(HttpStatus::Ok);
        res.set_header("X-A", "1");
        res.set_header("X-A", "2");
        assert_eq!(res.headers.get("X-A").map(String::as_str), Some("2"));
        assert_eq!(texte(&res).matches("X-A:").count(), 1);
    }

    #[test]
    fn cookies_simple_et_avec_options() {
        let mut res = HttpResponse::new(HttpStatus::Ok);
        res.set_cookie("a", "1", "");
        assert_eq!(res.headers.get("Set-Cookie").map(String::as_str), Some("a=1"));
        let mut res = HttpResponse::new(HttpStatus::Ok);
        res.set_cookie("session", "xyz", "Path=/; HttpOnly");
        assert_eq!(res.headers.get("Set-Cookie").map(String::as_str), Some("session=xyz; Path=/; HttpOnly"));
    }

    #[test]
    fn plusieurs_cookies_cumules() {
        let mut res = HttpResponse::new(HttpStatus::Ok);
        res.set_cookie("a", "1", "");
        res.set_cookie("b", "2", "Path=/");
        assert_eq!(res.headers.get("Set-Cookie").map(String::as_str), Some("a=1, b=2; Path=/"));
    }
}
