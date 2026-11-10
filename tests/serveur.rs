//! Tests de bout en bout : chaque test démarre le vrai binaire `localhost` sur un port libre,
//! avec sa propre configuration et son propre dossier, puis lui parle en HTTP brut.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct Serveur {
    proc: Child,
    port: u16,
    dir: PathBuf,
}

fn port_libre() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

impl Serveur {
    /// `conf` reçoit le port et le dossier de travail, et renvoie le contenu de server.conf.
    fn demarrer(conf: impl Fn(u16, &str) -> String) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("lh-e2e-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("www/fichiers")).unwrap();
        std::fs::write(dir.join("www/index.html"), "<h1>accueil</h1>").unwrap();
        std::fs::write(dir.join("www/fichiers/note.txt"), "note").unwrap();
        std::fs::write(dir.join("www/404.html"), "<p>page perso</p>").unwrap();
        let cgi = dir.join("www/echo.sh");
        std::fs::write(&cgi, "#!/bin/sh\nprintf 'Content-Type: text/plain\\r\\n\\r\\n'\nprintf '%s|%s|' \"$REQUEST_METHOD\" \"$QUERY_STRING\"\ncat\n").unwrap();
        let fail = dir.join("www/fail.sh");
        std::fs::write(&fail, "#!/bin/sh\nexit 3\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        for f in [&cgi, &fail] {
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let port = port_libre();
        let d = dir.to_string_lossy().to_string();
        std::fs::write(dir.join("server.conf"), conf(port, &d)).unwrap();
        let proc = Command::new(env!("CARGO_BIN_EXE_localhost"))
            .args(["run", "-c"])
            .arg(dir.join("server.conf"))
            .arg("--no-color")
            .current_dir(&dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let debut = Instant::now();
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(debut.elapsed() < Duration::from_secs(5), "le serveur n'a pas démarré");
            std::thread::sleep(Duration::from_millis(20));
        }
        Serveur { proc, port, dir }
    }

    fn standard() -> Self {
        Self::demarrer(|port, d| format!(
            "server {{ listen {port}; host 127.0.0.1; server_name site.local; client_max_body_size 2048;
               error_page 404 {d}/www/404.html;
               location / {{ allow_methods GET; root {d}/www; index index.html; }}
               location /fichiers {{ allow_methods GET DELETE; root {d}/www/fichiers; autoindex on; }}
               location /upload {{ allow_methods GET POST; root {d}/www; upload_store {d}/depot; }}
               location /cgi {{ allow_methods GET POST; root {d}/www; cgi_extension .sh; }}
               location /ancien {{ return /; }}
             }}"
        ))
    }

    /// Envoie des octets bruts et lit la réponse jusqu'à la fermeture de la connexion.
    fn brut(&self, data: &[u8]) -> Vec<u8> {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(data).unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        out
    }

    fn requete(&self, method: &str, path: &str, headers: &str, body: &[u8]) -> Reponse {
        let mut raw = format!("{method} {path} HTTP/1.1\r\nHost: site.local\r\nConnection: close\r\n{headers}");
        if !body.is_empty() || method == "POST" {
            raw.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        raw.push_str("\r\n");
        let mut data = raw.into_bytes();
        data.extend_from_slice(body);
        Reponse::analyser(&self.brut(&data))
    }

    fn get(&self, path: &str) -> Reponse {
        self.requete("GET", path, "", b"")
    }
}

impl Drop for Serveur {
    fn drop(&mut self) {
        let _ = self.proc.kill();
        let _ = self.proc.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Reponse {
    statut: u16,
    en_tetes: Vec<(String, String)>,
    corps: Vec<u8>,
}

impl Reponse {
    fn analyser(raw: &[u8]) -> Self {
        let fin = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("réponse HTTP complète");
        let tete = String::from_utf8_lossy(&raw[..fin]).to_string();
        let mut lignes = tete.lines();
        let statut = lignes.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
        let en_tetes = lignes
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
            .collect();
        Reponse { statut, en_tetes, corps: raw[fin + 4..].to_vec() }
    }
    fn en_tete(&self, nom: &str) -> Option<&str> {
        self.en_tetes.iter().find(|(k, _)| k == nom).map(|(_, v)| v.as_str())
    }
    fn texte(&self) -> String {
        String::from_utf8_lossy(&self.corps).to_string()
    }
}

#[test]
fn sert_la_page_d_accueil() {
    let s = Serveur::standard();
    let r = s.get("/");
    assert_eq!(r.statut, 200);
    assert_eq!(r.texte(), "<h1>accueil</h1>");
    assert_eq!(r.en_tete("content-type"), Some("text/html; charset=utf-8"));
    assert_eq!(r.en_tete("content-length"), Some("16"));
}

#[test]
fn page_404_personnalisee() {
    let s = Serveur::standard();
    let r = s.get("/introuvable.html");
    assert_eq!(r.statut, 404);
    assert_eq!(r.texte(), "<p>page perso</p>");
}

#[test]
fn methode_interdite_405_et_inconnue_501() {
    let s = Serveur::standard();
    assert_eq!(s.requete("DELETE", "/index.html", "", b"").statut, 405);
    assert_eq!(s.requete("PUT", "/", "", b"").statut, 501);
}

#[test]
fn redirection_301() {
    let s = Serveur::standard();
    let r = s.get("/ancien");
    assert_eq!(r.statut, 301);
    assert_eq!(r.en_tete("location"), Some("/"));
}

#[test]
fn listing_de_dossier() {
    let s = Serveur::standard();
    let r = s.get("/fichiers/");
    assert_eq!(r.statut, 200);
    assert!(r.texte().contains("note.txt"));
}

#[test]
fn remontee_de_dossier_refusee() {
    let s = Serveur::standard();
    std::fs::write(s.dir.join("secret.txt"), "SECRET").unwrap();
    let r = Reponse::analyser(&s.brut(b"GET /fichiers/../../secret.txt HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"));
    assert_eq!(r.statut, 400);
    assert!(!r.texte().contains("SECRET"));
    let r = Reponse::analyser(&s.brut(b"DELETE /fichiers/../../secret.txt HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"));
    assert_eq!(r.statut, 400);
    assert!(s.dir.join("secret.txt").exists());
}

#[test]
fn suppression_d_un_fichier() {
    let s = Serveur::standard();
    assert_eq!(s.requete("DELETE", "/fichiers/note.txt", "", b"").statut, 204);
    assert!(!s.dir.join("www/fichiers/note.txt").exists());
    assert_eq!(s.get("/fichiers/note.txt").statut, 404);
}

#[test]
fn upload_multipart_binaire_intact() {
    let s = Serveur::standard();
    let contenu: Vec<u8> = (0u8..=255).cycle().take(1500).collect();
    let mut corps = b"--frontiere\r\nContent-Disposition: form-data; name=\"f\"; filename=\"image.bin\"\r\n\r\n".to_vec();
    corps.extend_from_slice(&contenu);
    corps.extend_from_slice(b"\r\n--frontiere--\r\n");
    let r = s.requete("POST", "/upload", "Content-Type: multipart/form-data; boundary=frontiere\r\n", &corps);
    assert_eq!(r.statut, 201);
    assert_eq!(std::fs::read(s.dir.join("depot/image.bin")).unwrap(), contenu);
}

#[test]
fn corps_trop_gros_413() {
    let s = Serveur::standard();
    let r = s.requete("POST", "/upload", "", &vec![b'x'; 10_000]);
    assert_eq!(r.statut, 413);
}

#[test]
fn cgi_get_avec_query_string() {
    let s = Serveur::standard();
    let r = s.get("/cgi/echo.sh?nom=johan");
    assert_eq!(r.statut, 200);
    assert_eq!(r.en_tete("content-type"), Some("text/plain"));
    assert_eq!(r.texte(), "GET|nom=johan|");
}

#[test]
fn cgi_post_recoit_le_corps_sur_stdin() {
    let s = Serveur::standard();
    let r = s.requete("POST", "/cgi/echo.sh", "", b"donnees=42");
    assert_eq!(r.statut, 200);
    assert_eq!(r.texte(), "POST||donnees=42");
}

#[test]
fn cgi_en_echec_502() {
    let s = Serveur::standard();
    assert_eq!(s.get("/cgi/fail.sh").statut, 502);
}

#[test]
fn requete_chunked() {
    let s = Serveur::standard();
    let r = Reponse::analyser(&s.brut(
        b"POST /cgi/echo.sh HTTP/1.1\r\nHost: x\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n",
    ));
    assert_eq!(r.statut, 200);
    assert!(r.texte().ends_with("abcde"));
}

#[test]
fn requete_recue_en_plusieurs_paquets() {
    let s = Serveur::standard();
    let mut c = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    for morceau in ["GET / HT", "TP/1.1\r\nHo", "st: x\r\nConnection: close\r", "\n\r\n"] {
        c.write_all(morceau.as_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(30));
    }
    let mut out = Vec::new();
    let _ = c.read_to_end(&mut out);
    assert_eq!(Reponse::analyser(&out).statut, 200);
}

#[test]
fn expect_100_continue() {
    let s = Serveur::standard();
    let mut c = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    c.write_all(b"POST /cgi/echo.sh HTTP/1.1\r\nHost: x\r\nConnection: close\r\nExpect: 100-continue\r\nContent-Length: 2\r\n\r\n").unwrap();
    let mut buf = [0u8; 25];
    c.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"HTTP/1.1 100 Continue\r\n\r\n");
    c.write_all(b"ok").unwrap();
    let mut out = Vec::new();
    let _ = c.read_to_end(&mut out);
    assert!(String::from_utf8_lossy(&out).ends_with("ok"));
}

#[test]
fn requete_malformee_ne_fait_pas_tomber_le_serveur() {
    let s = Serveur::standard();
    let debut = Instant::now();
    assert_eq!(Reponse::analyser(&s.brut(b"\x00\xff\xfe nimporte quoi\r\n\r\n")).statut, 400);
    assert_eq!(Reponse::analyser(&s.brut(b"GET\r\n\r\n")).statut, 400);
    // Réponse immédiate : la connexion n'est pas gardée ouverte jusqu'au timeout.
    assert!(debut.elapsed() < Duration::from_secs(1));
    assert_eq!(s.get("/").statut, 200);
}

#[test]
fn connexions_simultanees() {
    let s = Serveur::standard();
    let port = s.port;
    let fils: Vec<_> = (0..20)
        .map(|_| {
            std::thread::spawn(move || {
                let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
                c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                c.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
                let mut out = Vec::new();
                let _ = c.read_to_end(&mut out);
                Reponse::analyser(&out).statut
            })
        })
        .collect();
    for f in fils {
        assert_eq!(f.join().unwrap(), 200);
    }
}

#[test]
fn client_lent_n_empeche_pas_les_autres() {
    let s = Serveur::standard();
    let mut lent = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
    lent.write_all(b"GET / HTTP/1.1\r\n").unwrap(); // en-têtes jamais terminés
    let debut = Instant::now();
    assert_eq!(s.get("/").statut, 200);
    assert!(debut.elapsed() < Duration::from_secs(1));
}

#[test]
fn hotes_virtuels_sur_le_meme_port() {
    let s = Serveur::demarrer(|port, d| {
        std::fs::create_dir_all(format!("{d}/b")).unwrap();
        std::fs::write(format!("{d}/b/index.html"), "site B").unwrap();
        format!(
            "server {{ listen {port}; server_name a.local; location / {{ root {d}/www; index index.html; }} }}
             server {{ listen {port}; server_name b.local; location / {{ root {d}/b; index index.html; }} }}"
        )
    });
    let pour = |hote: &str| Reponse::analyser(&s.brut(format!("GET / HTTP/1.1\r\nHost: {hote}\r\nConnection: close\r\n\r\n").as_bytes())).texte();
    assert_eq!(pour("a.local"), "<h1>accueil</h1>");
    assert_eq!(pour("b.local"), "site B");
    assert_eq!(pour("inconnu.local"), "<h1>accueil</h1>"); // premier bloc par défaut
}

#[test]
fn plusieurs_ports_dans_un_bloc() {
    let second = port_libre();
    let s = Serveur::demarrer(move |port, d| format!("server {{ listen {port}; listen {second}; location / {{ root {d}/www; index index.html; }} }}"));
    let r = Reponse::analyser(&{
        let mut c = TcpStream::connect(("127.0.0.1", second)).unwrap();
        c.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
        let mut out = Vec::new();
        let _ = c.read_to_end(&mut out);
        out
    });
    assert_eq!(r.statut, 200);
    assert_eq!(s.get("/").statut, 200);
}

#[test]
fn point_de_statut_compte_les_requetes() {
    let s = Serveur::standard();
    s.get("/");
    s.get("/absent");
    let r = s.get("/__localhost/status");
    assert_eq!(r.statut, 200);
    assert_eq!(r.en_tete("content-type"), Some("application/json"));
    let j = r.texte();
    assert!(j.contains("\"server\":\"localhost\""), "{}", j);
    assert!(j.contains("\"requests\":2"), "{}", j);
    assert!(j.contains("\"s4xx\":1"), "{}", j);
}

#[test]
fn commande_check_valide_et_refuse() {
    let dir = std::env::temp_dir().join(format!("lh-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ok = dir.join("ok.conf");
    let ko = dir.join("ko.conf");
    std::fs::write(&ok, "server { listen 8099; location / { root .; } }").unwrap();
    std::fs::write(&ko, "server { listen 8099; listen 8099; }").unwrap();
    let code = |f: &PathBuf| Command::new(env!("CARGO_BIN_EXE_localhost")).args(["check", "--no-color", "-c"]).arg(f).output().unwrap().status.code();
    assert_eq!(code(&ok), Some(0));
    assert_eq!(code(&ko), Some(1));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn commande_version_et_option_inconnue() {
    let out = Command::new(env!("CARGO_BIN_EXE_localhost")).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains(env!("CARGO_PKG_VERSION")));
    let out = Command::new(env!("CARGO_BIN_EXE_localhost")).arg("--inexistante").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn arret_propre_sur_sigterm() {
    let mut s = Serveur::standard();
    unsafe { libc::kill(s.proc.id() as i32, libc::SIGTERM) };
    let debut = Instant::now();
    let statut = loop {
        if let Some(st) = s.proc.try_wait().unwrap() {
            break st;
        }
        assert!(debut.elapsed() < Duration::from_secs(3), "le serveur ne s'est pas arrêté");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(statut.success());
}
