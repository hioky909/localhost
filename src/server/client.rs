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
                if HttpRequest::is_malformed(&self.read_buffer) {
                    let mut res = HttpResponse::new(HttpStatus::BadRequest);
                    res.set_header("Content-Type", "text/plain; charset=utf-8");
                    res.set_body(b"Bad Request".to_vec());
                    self.write_buffer = res.to_bytes();
                    self.state = ClientState::WritingResponse;
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
                            // Code non géré : on garde 200 plutôt que d'envoyer un statut inventé.
                            res.status = HttpStatus::from_code(code).unwrap_or(HttpStatus::Ok);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Paire de sockets connectées : `ours` joue le client distant, `fd` est donné au serveur.
    struct Paire {
        fd: RawFd,
        ours: RawFd,
    }
    impl Paire {
        fn new() -> Self {
            let mut fds = [0; 2];
            assert_eq!(unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_NONBLOCK, 0, fds.as_mut_ptr()) }, 0);
            Paire { fd: fds[0], ours: fds[1] }
        }
        fn envoyer(&self, data: &[u8]) {
            assert_eq!(unsafe { libc::write(self.ours, data.as_ptr() as *const _, data.len()) }, data.len() as isize);
        }
        fn recevoir(&self) -> Vec<u8> {
            let mut buf = vec![0u8; 65536];
            let n = unsafe { libc::read(self.ours, buf.as_mut_ptr() as *mut _, buf.len()) };
            buf.truncate(n.max(0) as usize);
            buf
        }
        fn fermer_distant(&mut self) {
            unsafe { libc::close(self.ours) };
            self.ours = -1;
        }
    }
    impl Drop for Paire {
        fn drop(&mut self) {
            unsafe {
                libc::close(self.fd);
                if self.ours >= 0 {
                    libc::close(self.ours);
                }
            }
        }
    }

    fn client(cfg: ServerConfig) -> Client {
        Client::new(-1, cfg, "127.0.0.1:50000".into())
    }

    fn avec_sortie_cgi(sortie: &[u8]) -> Client {
        let mut c = client(ServerConfig::default());
        c.write_buffer = sortie.to_vec();
        c.parse_cgi_response();
        c
    }

    #[test]
    fn etat_initial() {
        let c = client(ServerConfig::default());
        assert_eq!(c.state, ClientState::ReadingRequest);
        assert_eq!(c.peer, "127.0.0.1:50000");
        assert!(c.started_at.is_none() && !c.internal && !c.sent_continue);
    }

    #[test]
    fn statut_lu_sur_la_ligne_d_etat() {
        let mut c = client(ServerConfig::default());
        assert_eq!(c.status_code(), 0);
        c.write_buffer = b"HTTP/1.1 404 Not Found\r\n\r\n".to_vec();
        assert_eq!(c.status_code(), 404);
        c.write_buffer = b"n'importe quoi".to_vec();
        assert_eq!(c.status_code(), 0);
    }

    #[test]
    fn methode_et_chemin_depuis_la_requete_analysee() {
        let mut c = client(ServerConfig::default());
        c.request = HttpRequest::parse(b"DELETE /f.txt?force=1 HTTP/1.1\r\n\r\n");
        assert_eq!(c.method_and_path(), ("DELETE".into(), "/f.txt?force=1".into()));
        c.request = HttpRequest::parse(b"PUT /x HTTP/1.1\r\n\r\n");
        assert_eq!(c.method_and_path(), ("PUT".into(), "/x".into()));
    }

    #[test]
    fn methode_et_chemin_depuis_les_octets_bruts() {
        let mut c = client(ServerConfig::default());
        assert_eq!(c.method_and_path(), ("-".into(), "-".into()));
        c.read_buffer = b"POST /upload HTTP/1.1\r\nContent-Length: 999999".to_vec();
        assert_eq!(c.method_and_path(), ("POST".into(), "/upload".into()));
    }

    #[test]
    fn hote_depuis_l_en_tete_ou_le_server_name() {
        let cfg = ServerConfig { server_names: vec!["defaut.local".into()], ..Default::default() };
        let mut c = client(cfg);
        assert_eq!(c.host(), "defaut.local");
        c.request = HttpRequest::parse(b"GET / HTTP/1.1\r\nHost: b.local:8080\r\n\r\n");
        assert_eq!(c.host(), "b.local:8080");
        assert_eq!(client(ServerConfig::default()).host(), "");
    }

    #[test]
    fn sortie_cgi_avec_en_tetes_crlf() {
        let c = avec_sortie_cgi(b"Content-Type: text/plain\r\nX-Script: oui\r\n\r\nbonjour");
        let res = c.response.unwrap();
        assert_eq!(res.status, HttpStatus::Ok);
        assert_eq!(res.headers.get("Content-Type").map(String::as_str), Some("text/plain"));
        assert_eq!(res.headers.get("X-Script").map(String::as_str), Some("oui"));
        assert_eq!(res.body, b"bonjour");
        assert!(c.write_buffer.starts_with(b"HTTP/1.1 200 OK\r\n"));
    }

    #[test]
    fn sortie_cgi_avec_simples_sauts_de_ligne() {
        let res = avec_sortie_cgi(b"Content-Type: text/html\n\n<p>ok</p>").response.unwrap();
        assert_eq!(res.headers.get("Content-Type").map(String::as_str), Some("text/html"));
        assert_eq!(res.body, b"<p>ok</p>");
    }

    #[test]
    fn sortie_cgi_avec_en_tete_status() {
        let res = avec_sortie_cgi(b"Status: 404 Not Found\r\n\r\nintrouvable").response.unwrap();
        assert_eq!(res.status, HttpStatus::NotFound);
        assert!(!res.headers.contains_key("Status"));
        let res = avec_sortie_cgi(b"Status: 302\r\nLocation: /ailleurs\r\n\r\n").response.unwrap();
        assert_eq!(res.status, HttpStatus::Found);
        assert_eq!(res.headers.get("Location").map(String::as_str), Some("/ailleurs"));
    }

    #[test]
    fn sortie_cgi_status_non_gere_reste_a_200() {
        let res = avec_sortie_cgi(b"Status: 418 I'm a teapot\r\n\r\n").response.unwrap();
        assert_eq!(res.status, HttpStatus::Ok);
    }

    #[test]
    fn sortie_cgi_sans_en_tetes_devient_le_corps() {
        let res = avec_sortie_cgi(b"juste du texte").response.unwrap();
        assert_eq!(res.body, b"juste du texte");
        assert_eq!(res.headers.get("Content-Length").map(String::as_str), Some("14"));
    }

    #[test]
    fn sortie_cgi_corps_binaire_preserve() {
        let mut sortie = b"Content-Type: application/octet-stream\r\n\r\n".to_vec();
        sortie.extend_from_slice(&[0, 159, 255, 13, 10]);
        assert_eq!(avec_sortie_cgi(&sortie).response.unwrap().body, vec![0, 159, 255, 13, 10]);
    }

    #[test]
    fn lit_une_requete_complete() {
        let p = Paire::new();
        let mut c = Client::new(p.fd, ServerConfig::default(), "x".into());
        p.envoyer(b"GET /a HTTP/1.1\r\nHost: t\r\n\r\n");
        assert!(c.read_data().unwrap());
        assert_eq!(c.state, ClientState::Processing);
        assert_eq!(c.request.as_ref().unwrap().path, "/a");
        assert!(c.started_at.is_some());
    }

    #[test]
    fn requete_en_plusieurs_morceaux() {
        let p = Paire::new();
        let mut c = Client::new(p.fd, ServerConfig::default(), "x".into());
        p.envoyer(b"POST /f HTTP/1.1\r\nContent-Length: 6\r\n\r\nabc");
        assert!(!c.read_data().unwrap());
        assert_eq!(c.state, ClientState::ReadingRequest);
        assert!(!c.read_data().unwrap()); // rien de nouveau : WouldBlock sans erreur
        p.envoyer(b"def");
        assert!(c.read_data().unwrap());
        assert_eq!(c.request.as_ref().unwrap().body, b"abcdef");
    }

    #[test]
    fn deconnexion_du_client() {
        let mut p = Paire::new();
        let mut c = Client::new(p.fd, ServerConfig::default(), "x".into());
        p.fermer_distant();
        assert!(c.read_data().unwrap());
        assert_eq!(c.state, ClientState::Done);
    }

    #[test]
    fn requete_trop_volumineuse_413_sans_attendre_la_fin() {
        let p = Paire::new();
        let cfg = ServerConfig { client_max_body_size: Some(64), ..Default::default() };
        let mut c = Client::new(p.fd, cfg, "x".into());
        let mut raw = b"POST / HTTP/1.1\r\nContent-Length: 100000\r\n\r\n".to_vec();
        raw.extend(std::iter::repeat(b'a').take(100));
        p.envoyer(&raw);
        assert!(c.read_data().unwrap());
        assert_eq!(c.state, ClientState::WritingResponse);
        assert_eq!(c.status_code(), 413);
    }

    #[test]
    fn requete_malformee_400_immediat() {
        let p = Paire::new();
        let mut c = Client::new(p.fd, ServerConfig::default(), "x".into());
        p.envoyer(b"GET\r\n\r\n");
        assert!(c.read_data().unwrap());
        assert_eq!(c.state, ClientState::WritingResponse);
        assert_eq!(c.status_code(), 400);
    }

    #[test]
    fn envoie_100_continue_une_seule_fois() {
        let p = Paire::new();
        let mut c = Client::new(p.fd, ServerConfig::default(), "x".into());
        p.envoyer(b"POST / HTTP/1.1\r\nExpect: 100-continue\r\nContent-Length: 4\r\n\r\n");
        assert!(!c.read_data().unwrap());
        assert_eq!(p.recevoir(), b"HTTP/1.1 100 Continue\r\n\r\n");
        p.envoyer(b"ab");
        assert!(!c.read_data().unwrap());
        assert!(p.recevoir().is_empty());
        p.envoyer(b"cd");
        assert!(c.read_data().unwrap());
        assert_eq!(c.request.as_ref().unwrap().body, b"abcd");
    }

    #[test]
    fn traitement_sans_requete_400() {
        let mut c = client(ServerConfig::default());
        let res = c.process_request().ok().unwrap();
        assert_eq!(res.status, HttpStatus::BadRequest);
        assert_eq!(c.state, ClientState::WritingResponse);
        assert_eq!(c.status_code(), 400);
    }

    #[test]
    fn traitement_prepare_la_reponse() {
        let mut c = client(ServerConfig::default());
        c.request = HttpRequest::parse(b"GET /rien HTTP/1.1\r\n\r\n");
        let res = c.process_request().ok().unwrap();
        assert_eq!(res.status, HttpStatus::NotFound);
        assert_eq!(c.state, ClientState::WritingResponse);
        assert_eq!(c.status_code(), 404);
        assert!(c.response.is_some());
    }

    #[test]
    fn ecrit_toute_la_reponse_puis_termine() {
        let p = Paire::new();
        let mut c = Client::new(p.fd, ServerConfig::default(), "x".into());
        c.write_buffer = b"HTTP/1.1 204 No Content\r\n\r\n".to_vec();
        assert!(c.write_data().unwrap());
        assert_eq!(c.state, ClientState::Done);
        assert_eq!(c.written_bytes, c.write_buffer.len());
        assert_eq!(p.recevoir(), b"HTTP/1.1 204 No Content\r\n\r\n");
        assert!(c.write_data().unwrap()); // rien à écrire : reste terminé
    }

    #[test]
    fn grosse_reponse_ecrite_en_plusieurs_fois() {
        let p = Paire::new();
        let mut c = Client::new(p.fd, ServerConfig::default(), "x".into());
        c.write_buffer = vec![b'z'; 4 * 1024 * 1024];
        let mut recu = 0;
        let mut tours = 0;
        while !c.write_data().unwrap() {
            recu += p.recevoir().len();
            tours += 1;
            assert!(tours < 10_000);
        }
        loop {
            let n = p.recevoir().len();
            if n == 0 { break; }
            recu += n;
        }
        assert!(tours > 0, "le tampon du socket aurait dû saturer");
        assert_eq!(recu, 4 * 1024 * 1024);
    }
}
