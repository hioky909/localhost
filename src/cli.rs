//! Interface en ligne de commande : démarrage, vérification de configuration, statut en direct.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::config::{Config, HttpMethod, RouteConfig};
use crate::server::{self, Server, STATUS_PATH};
use crate::term::{self, bold, brand, brand_bold, cyan, dim, gray, green, red, yellow};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Options {
    command: String,
    config: String,
    addr: Option<String>,
    watch: bool,
    no_color: bool,
    stats_every: u64,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options { command: "run".into(), config: "server.conf".into(), addr: None, watch: false, no_color: false, stats_every: 10 };
    let mut args = std::env::args().skip(1).peekable();
    let mut command_set = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "run" | "check" | "status" | "version" | "help" if !command_set => {
                o.command = a.clone();
                command_set = true;
            }
            "-t" | "--test" => o.command = "check".into(),
            "-v" | "-V" | "--version" => o.command = "version".into(),
            "-h" | "--help" => o.command = "help".into(),
            "-c" | "--config" => o.config = args.next().ok_or("-c attend un fichier de configuration")?,
            "-a" | "--addr" => o.addr = Some(args.next().ok_or("--addr attend host:port")?),
            "-w" | "--watch" => o.watch = true,
            "--no-color" => o.no_color = true,
            "--stats" => {
                let v = args.next().ok_or("--stats attend un nombre de secondes")?;
                o.stats_every = v.parse().map_err(|_| format!("--stats : valeur invalide « {} »", v))?;
            }
            // Compatibilité : `localhost server.conf`
            other if !other.starts_with('-') && (other.ends_with(".conf") || std::path::Path::new(other).is_file()) => o.config = other.to_string(),
            other => return Err(format!("option inconnue « {} » (voir localhost --help)", other)),
        }
    }
    Ok(o)
}

pub fn main() -> i32 {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            term::init(false);
            eprintln!("{} {}", red("error:"), e);
            return 2;
        }
    };
    term::init(opts.no_color);
    match opts.command.as_str() {
        "help" => {
            print_help();
            0
        }
        "version" => {
            print_version();
            0
        }
        "check" => check(&opts.config),
        "status" => status(&opts),
        _ => run(&opts),
    }
}

// ───────────────────────────── aide & version ─────────────────────────────

fn logo_line() -> String {
    format!("{} {}  {}", brand_bold("◆ localhost"), dim(&format!("v{}", VERSION)), gray("serveur HTTP/1.1 · Rust · epoll"))
}

fn print_version() {
    println!("{}", logo_line());
    println!("{}", gray(&format!("  build {} · {}-{} · dépendance unique : libc", if cfg!(debug_assertions) { "debug" } else { "release" }, std::env::consts::ARCH, std::env::consts::OS)));
}

fn print_help() {
    println!("{}\n", logo_line());
    println!("{}", bold("USAGE"));
    println!("  localhost [commande] [options]\n");
    println!("{}", bold("COMMANDES"));
    let cmds = [
        ("run", "démarre le serveur au premier plan (par défaut)"),
        ("check", "vérifie la configuration puis quitte (comme nginx -t)"),
        ("status", "affiche l'état d'un serveur en cours d'exécution"),
        ("version", "affiche la version"),
    ];
    for (c, d) in cmds {
        println!("  {}  {}", cyan(&format!("{:<9}", c)), d);
    }
    println!("\n{}", bold("OPTIONS"));
    let opts = [
        ("-c, --config <fichier>", "fichier de configuration (défaut : server.conf)"),
        ("-t, --test", "alias de check"),
        ("-a, --addr <host:port>", "serveur à interroger pour status (défaut : 1re écoute de la config)"),
        ("-w, --watch", "status : rafraîchit le tableau de bord chaque seconde"),
        ("    --stats <s>", "run : intervalle de la ligne de statistiques (0 = désactivée)"),
        ("    --no-color", "désactive les couleurs (aussi via NO_COLOR)"),
        ("-v, --version", "affiche la version"),
    ];
    for (o, d) in opts {
        println!("  {}  {}", cyan(&format!("{:<24}", o)), d);
    }
    println!("\n{}", bold("EXEMPLES"));
    println!("  {}", gray("$ localhost check -c server.conf"));
    println!("  {}", gray("$ localhost run --stats 5"));
    println!("  {}", gray("$ localhost status --watch"));
}

// ───────────────────────────── configuration ─────────────────────────────

fn load(path: &str) -> Result<Config, String> {
    Config::load_file(path).map_err(|e| match e {
        crate::config::parser::ConfigError::Io(io) => format!("impossible de lire {} : {}", path, io),
        crate::config::parser::ConfigError::ParseError(m) => format!("{} : {}", path, m),
    })
}

/// Erreurs bloquantes et avertissements, sur la configuration déjà analysée.
fn validate(cfg: &Config) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if cfg.servers.is_empty() {
        errors.push("aucun bloc server".into());
    }
    let mut seen: Vec<(String, Vec<String>)> = Vec::new();
    for (i, s) in cfg.servers.iter().enumerate() {
        let name = s.server_names.first().cloned().unwrap_or_else(|| format!("server #{}", i + 1));
        if s.ports.is_empty() {
            errors.push(format!("{} : aucune directive listen", name));
        }
        let mut ports = s.ports.clone();
        ports.sort();
        for w in ports.windows(2) {
            if w[0] == w[1] {
                errors.push(format!("{} : port {} déclaré deux fois dans le même bloc", name, w[0]));
            }
        }
        ports.dedup();
        for p in &ports {
            let addr = format!("{}:{}", s.host, p);
            if let Some((_, names)) = seen.iter().find(|(a, _)| *a == addr) {
                if names.iter().any(|n| s.server_names.contains(n)) || s.server_names.is_empty() {
                    errors.push(format!("{} : {} et server_name déjà utilisés par un autre bloc", name, addr));
                }
            } else {
                seen.push((addr, s.server_names.clone()));
            }
        }
        for (code, page) in &s.error_pages {
            if !std::path::Path::new(page).is_file() {
                warnings.push(format!("{} : page d'erreur {} introuvable ({})", name, code, page));
            }
        }
        for r in &s.routes {
            if let Some(root) = &r.root_directory {
                if r.redirection.is_none() && !std::path::Path::new(root).is_dir() {
                    warnings.push(format!("{} : location {} → racine {} inexistante", name, r.path, root));
                }
            }
            if let Some(store) = &r.upload_store {
                if !std::path::Path::new(store).is_dir() {
                    warnings.push(format!("{} : location {} → {} sera créé au premier upload", name, r.path, store));
                }
            }
        }
    }
    (errors, warnings)
}

fn methods(r: &RouteConfig) -> String {
    r.accepted_methods
        .iter()
        .map(|m| match m {
            HttpMethod::Get => term::method_word("GET"),
            HttpMethod::Post => term::method_word("POST"),
            HttpMethod::Delete => term::method_word("DELETE"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn route_kind(r: &RouteConfig) -> String {
    if let Some(to) = &r.redirection {
        return format!("{} {}", term::paint("1;34", "301 →"), to);
    }
    let mut parts = Vec::new();
    if let Some(ext) = &r.cgi_extension {
        parts.push(format!("{} {}", yellow("cgi"), ext));
    }
    if let Some(store) = &r.upload_store {
        parts.push(format!("{} → {}", yellow("upload"), store));
    }
    if let Some(root) = &r.root_directory {
        parts.push(format!("{} {}", dim("root"), root));
    }
    if let Some(index) = &r.index_file {
        parts.push(format!("{} {}", dim("index"), index));
    }
    if r.directory_listing {
        parts.push(cyan("autoindex"));
    }
    parts.join(gray("  ·  ").as_str())
}

fn print_config(cfg: &Config) {
    for s in &cfg.servers {
        let names = if s.server_names.is_empty() { "_".to_string() } else { s.server_names.join(", ") };
        let ports: Vec<String> = s.ports.iter().map(|p| format!("{}:{}", s.host, p)).collect();
        println!("  {}  {}  {}", bold(&names), gray(&ports.join("  ")), dim(&format!("body ≤ {} · timeout {} s", s.client_max_body_size.map(|b| term::bytes(b as u64)).unwrap_or("∞".into()), s.timeout.unwrap_or(0))));
        for r in &s.routes {
            println!("    {}  {}  {}", cyan(&term::pad(&r.path, 16)), term::pad(&methods(r), 20), route_kind(r));
        }
        if !s.error_pages.is_empty() {
            let mut codes: Vec<_> = s.error_pages.keys().collect();
            codes.sort();
            println!("    {}  {}", gray(&term::pad("error_page", 16)), gray(&codes.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")));
        }
    }
}

fn check(path: &str) -> i32 {
    let started = Instant::now();
    println!("{}\n", logo_line());
    let cfg = match load(path) {
        Ok(c) => c,
        Err(e) => {
            println!("  {} {}", red("✗"), e);
            println!("\n  {}", red(&format!("échec du test de configuration ({})", path)));
            return 1;
        }
    };
    println!("  {} syntaxe de {} correcte", green("✓"), bold(path));
    let (errors, warnings) = validate(&cfg);
    let routes: usize = cfg.servers.iter().map(|s| s.routes.len()).sum();
    let listens: usize = cfg.servers.iter().map(|s| s.ports.len()).sum();
    println!("  {} {} bloc(s) server · {} écoute(s) · {} location(s)\n", green("✓"), cfg.servers.len(), listens, routes);
    print_config(&cfg);
    println!();
    for w in &warnings {
        println!("  {} {}", yellow("!"), w);
    }
    for e in &errors {
        println!("  {} {}", red("✗"), e);
    }
    if errors.is_empty() {
        println!("{}  {}", if warnings.is_empty() { String::new() } else { "\n".into() }, green(&format!("test réussi en {}", term::micros(started.elapsed().as_micros() as u64))));
        0
    } else {
        println!("\n  {}", red(&format!("échec : {} erreur(s)", errors.len())));
        1
    }
}

// ───────────────────────────── run ─────────────────────────────

fn run(opts: &Options) -> i32 {
    let started = Instant::now();
    let cfg = match load(&opts.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {}", red("✗"), e);
            return 1;
        }
    };
    let (errors, warnings) = validate(&cfg);
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("{} {}", red("✗"), e);
        }
        eprintln!("{}", red("configuration invalide — lancez `localhost check` pour le détail"));
        return 1;
    }

    let mut srv = match Server::new(cfg.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} epoll : {}", red("✗"), e);
            return 1;
        }
    };
    if let Err(e) = srv.setup_listeners() {
        eprintln!("{} {}", red("✗"), e);
        return 1;
    }
    srv.stats_every = Duration::from_secs(if opts.stats_every == 0 { u64::MAX / 4 } else { opts.stats_every });
    server::install_signal_handlers();

    // Bannière de démarrage
    let w = 58;
    println!();
    println!("  {}", brand(&format!("╭{}╮", "─".repeat(w))));
    let title = format!("{}  {}", brand_bold("◆ localhost"), dim(&format!("v{}", VERSION)));
    let tag = gray("HTTP/1.1 · epoll · Rust");
    println!("  {} {}{}{} {}", brand("│"), title, " ".repeat(w - 2 - term::visible_len(&title) - term::visible_len(&tag)), tag, brand("│"));
    println!("  {}", brand(&format!("╰{}╯", "─".repeat(w))));
    println!();
    let kv = |k: &str, v: String| println!("  {}  {}", gray(&format!("{:<9}", k)), v);
    kv("config", format!("{}  {}", bold(&opts.config), green("✓ valide")));
    kv("pid", std::process::id().to_string());
    kv("modèle", "1 boucle d'événements epoll · E/S non bloquantes · CGI par pipes".into());
    let routes: usize = cfg.servers.iter().map(|s| s.routes.len()).sum();
    kv("routes", format!("{} location(s) sur {} bloc(s) server", routes, cfg.servers.len()));
    println!();
    for l in &srv.bound {
        let names = if l.names.is_empty() { "_".into() } else { l.names.join(", ") };
        println!("  {}  {}  {}", green("● LISTEN"), bold(&format!("http://{}", l.addr)), gray(&names));
    }
    println!();
    for wmsg in &warnings {
        println!("  {} {}", yellow("!"), wmsg);
    }
    println!(
        "  {} {}  {}\n",
        green("prêt"),
        dim(&format!("en {}", term::micros(started.elapsed().as_micros() as u64))),
        gray("Ctrl+C pour arrêter · statut : localhost status --watch"),
    );
    println!("{}", gray(&format!("{:<12}  {:<6} {:<4} {:<34} {:>9} {:>10}  {:<21}  {}", "heure", "méth.", "code", "chemin", "taille", "durée", "client", "hôte")));

    match srv.run() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{} boucle d'événements : {}", red("✗"), e);
            1
        }
    }
}

/// Bilan affiché à l'arrêt (appelé par Server::shutdown).
pub fn print_summary(srv: &Server) {
    let s = &srv.stats;
    println!();
    println!("  {}", bold("Bilan de la session"));
    let kv = |k: &str, v: String| println!("  {}  {}", gray(&format!("{:<13}", k)), v);
    kv("durée", term::uptime(s.uptime_secs()));
    kv("requêtes", format!("{}  {}", bold(&s.requests.to_string()), gray(&format!("({} connexions, pic {} simultanées)", s.accepted, s.peak))));
    kv("statuts", format!("{} 2xx  {} 3xx  {} 4xx  {} 5xx", green(&s.by_class[2].to_string()), term::paint("34", &s.by_class[3].to_string()), yellow(&s.by_class[4].to_string()), red(&s.by_class[5].to_string())));
    kv("latence", format!("p50 {}  ·  p95 {}  ·  p99 {}", term::micros(s.percentile(50.0)), term::micros(s.percentile(95.0)), term::micros(s.percentile(99.0))));
    kv("transfert", format!("↓ {}  ↑ {}", term::bytes(s.bytes_in), term::bytes(s.bytes_out)));
    kv("cgi / upload", format!("{} exécution(s) ({} échec(s)) · {} upload(s) · {} timeout(s)", s.cgi_runs, s.cgi_failures, s.uploads, s.timeouts));
    println!("\n  {}\n", dim("au revoir 👋"));
}

// ───────────────────────────── status ─────────────────────────────

fn fetch_status(addr: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect_timeout(&addr.parse().map_err(|_| format!("adresse invalide : {}", addr))?, Duration::from_secs(2))
        .map_err(|e| format!("aucun serveur sur {} ({})", addr, e))?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    write!(stream, "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", STATUS_PATH, addr).map_err(|e| e.to_string())?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).map_err(|e| e.to_string())?;
    let body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    if !body.starts_with('{') {
        return Err(format!("{} répond, mais ce n'est pas un serveur localhost", addr));
    }
    Ok(body)
}

fn num(json: &str, key: &str) -> u64 {
    let pat = format!("\"{}\":", key);
    json.find(&pat)
        .map(|i| &json[i + pat.len()..])
        .and_then(|rest| rest.split([',', '}']).next())
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

fn text(json: &str, key: &str) -> String {
    let pat = format!("\"{}\":\"", key);
    json.find(&pat).map(|i| json[i + pat.len()..].split('"').next().unwrap_or("").to_string()).unwrap_or_default()
}

fn sparkline(values: &[f64]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = values.iter().cloned().fold(0.0_f64, f64::max);
    values
        .iter()
        .map(|v| if max <= 0.0 { BARS[0] } else { BARS[((v / max) * 7.0).round() as usize] })
        .collect()
}

fn render_status(json: &str, addr: &str, history: &[f64], live: bool) {
    let req = num(json, "requests");
    let classes = [num(json, "s2xx"), num(json, "s3xx"), num(json, "s4xx"), num(json, "s5xx")];
    let max_class = *classes.iter().max().unwrap_or(&0);
    let rps = history.last().copied().unwrap_or(0.0);
    println!();
    println!(
        "  {}  {}  {}  {}",
        brand_bold("◆ localhost"),
        dim(&format!("v{}", text(json, "version"))),
        green("● en ligne"),
        gray(&format!("pid {} · {} · depuis {}", num(json, "pid"), addr, term::uptime(num(json, "uptime_s"))))
    );
    println!("  {}", gray(&"─".repeat(72)));
    println!(
        "  {}  {}  {}  {}",
        term::pad(&format!("{} {}", bold(&format!("{:.0}", rps)), gray("req/s")), 16),
        term::pad(&format!("{} {}", bold(&req.to_string()), gray("requêtes")), 20),
        term::pad(&format!("{} {}", bold(&num(json, "active").to_string()), gray(&format!("connexion(s) · pic {}", num(json, "peak")))), 24),
        format!("{} {}", bold(&term::micros(num(json, "p95_us"))), gray("p95"))
    );
    println!();
    if live {
        println!("  {}  {}", gray(&format!("{:<10}", "débit")), cyan(&sparkline(history)));
        println!();
    }
    let labels = [("2xx", "32"), ("3xx", "34"), ("4xx", "33"), ("5xx", "31")];
    for (i, (l, c)) in labels.iter().enumerate() {
        let pct = if req == 0 { 0.0 } else { classes[i] as f64 * 100.0 / req as f64 };
        println!("  {}  {}  {}  {}", term::paint(&format!("1;{}", c), l), term::paint(c, &term::bar(classes[i], max_class, 34)), term::pad_left(&classes[i].to_string(), 7), gray(&format!("{:5.1} %", pct)));
    }
    println!();
    println!(
        "  {}  p50 {}   p95 {}   p99 {}",
        gray(&format!("{:<10}", "latence")),
        term::micros(num(json, "p50_us")),
        term::micros(num(json, "p95_us")),
        term::micros(num(json, "p99_us"))
    );
    println!(
        "  {}  {} GET   {} POST   {} DELETE   {} autres",
        gray(&format!("{:<10}", "méthodes")),
        cyan(&num(json, "get").to_string()),
        yellow(&num(json, "post").to_string()),
        red(&num(json, "delete").to_string()),
        num(json, "other")
    );
    println!("  {}  ↓ {}   ↑ {}", gray(&format!("{:<10}", "transfert")), term::bytes(num(json, "bytes_in")), term::bytes(num(json, "bytes_out")));
    println!(
        "  {}  {} exécutions ({} échecs) · {} uploads · {} timeouts",
        gray(&format!("{:<10}", "cgi")),
        num(json, "cgi_runs"),
        num(json, "cgi_failures"),
        num(json, "uploads"),
        num(json, "timeouts")
    );
    println!("  {}  {}", gray(&format!("{:<10}", "écoutes")), text(json, "listeners").replace(',', "  "));
    if live {
        println!("\n  {}", gray("rafraîchi chaque seconde · Ctrl+C pour quitter"));
    }
}

fn status(opts: &Options) -> i32 {
    let addr = match &opts.addr {
        Some(a) => a.clone(),
        None => match load(&opts.config) {
            Ok(c) => c.servers.first().and_then(|s| s.ports.first().map(|p| format!("{}:{}", s.host, p))).unwrap_or_else(|| "127.0.0.1:8080".into()),
            Err(_) => "127.0.0.1:8080".into(),
        },
    };
    if !opts.watch {
        return match fetch_status(&addr) {
            Ok(j) => {
                render_status(&j, &addr, &[], false);
                println!();
                0
            }
            Err(e) => {
                eprintln!("  {} {}", red("●"), e);
                1
            }
        };
    }
    server::install_signal_handlers();
    let mut history: Vec<f64> = Vec::new();
    let mut last: Option<(u64, Instant)> = None;
    loop {
        match fetch_status(&addr) {
            Ok(j) => {
                let now = Instant::now();
                let req = num(&j, "requests");
                if let Some((prev, at)) = last {
                    history.push(req.saturating_sub(prev) as f64 / now.duration_since(at).as_secs_f64().max(0.001));
                    if history.len() > 60 {
                        history.remove(0);
                    }
                }
                last = Some((req, now));
                print!("\x1b[2J\x1b[H");
                render_status(&j, &addr, &history, true);
                let _ = std::io::stdout().flush();
            }
            Err(e) => {
                print!("\x1b[2J\x1b[H");
                println!("\n  {} {}\n  {}", red("● hors ligne"), e, gray("nouvelle tentative dans 1 s…"));
            }
        }
        std::thread::sleep(Duration::from_secs(1));
        if server::shutdown_requested() {
            println!();
            return 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuse_un_port_en_double_dans_un_bloc() {
        let cfg = Config::load_string("server { listen 8093; listen 8093; host 127.0.0.1; server_name a; location / { root www; } }").unwrap();
        let (errors, _) = validate(&cfg);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn accepte_des_hotes_virtuels_sur_le_meme_port() {
        let cfg = Config::load_string(
            "server { listen 8094; host 127.0.0.1; server_name a.local; location / { root www; } }
             server { listen 8094; host 127.0.0.1; server_name b.local; location / { root www; } }",
        )
        .unwrap();
        assert!(validate(&cfg).0.is_empty());
    }

    #[test]
    fn lit_le_json_de_statut() {
        let j = r#"{"version":"1.0.0","requests":42,"listeners":"127.0.0.1:8080,127.0.0.1:8081"}"#;
        assert_eq!(num(j, "requests"), 42);
        assert_eq!(text(j, "listeners"), "127.0.0.1:8080,127.0.0.1:8081");
    }

    fn cfg(src: &str) -> Config {
        Config::load_string(src).unwrap()
    }

    #[test]
    fn aucun_bloc_server_est_une_erreur() {
        let (errors, _) = validate(&cfg(""));
        assert_eq!(errors, vec!["aucun bloc server".to_string()]);
    }

    #[test]
    fn bloc_sans_listen_est_une_erreur() {
        let (errors, _) = validate(&cfg("server { server_name api; }"));
        assert_eq!(errors, vec!["api : aucune directive listen".to_string()]);
    }

    #[test]
    fn bloc_anonyme_designe_par_son_numero() {
        let (errors, _) = validate(&cfg("server { listen 1; } server { }"));
        assert!(errors.iter().any(|e| e.starts_with("server #2")));
    }

    #[test]
    fn meme_adresse_et_meme_nom_refuses() {
        let (errors, _) = validate(&cfg("server { listen 8095; server_name a; } server { listen 8095; server_name a; }"));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("127.0.0.1:8095"));
    }

    #[test]
    fn second_bloc_sans_nom_sur_une_adresse_deja_prise() {
        let (errors, _) = validate(&cfg("server { listen 8096; server_name a; } server { listen 8096; }"));
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn meme_port_sur_deux_hotes_differents_accepte() {
        let (errors, _) = validate(&cfg("server { listen 8097; host 127.0.0.1; } server { listen 8097; host 127.0.0.2; }"));
        assert!(errors.is_empty());
    }

    #[test]
    fn avertissements_pour_chemins_absents() {
        let (errors, warnings) = validate(&cfg(
            "server { listen 1; server_name w; error_page 404 /nulle/part.html;
              location / { root /dossier/inexistant; }
              location /up { upload_store /depot/inexistant; } }",
        ));
        assert!(errors.is_empty());
        assert_eq!(warnings.len(), 3);
        assert!(warnings.iter().any(|w| w.contains("page d'erreur 404 introuvable")));
        assert!(warnings.iter().any(|w| w.contains("racine /dossier/inexistant inexistante")));
        assert!(warnings.iter().any(|w| w.contains("sera créé au premier upload")));
    }

    #[test]
    fn redirection_sans_avertissement_de_racine() {
        let (_, warnings) = validate(&cfg("server { listen 1; location / { root /inexistant; return /ailleurs; } }"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn configuration_du_depot_sans_erreur() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/server.conf");
        let (errors, _) = validate(&Config::load_file(path).unwrap());
        assert!(errors.is_empty(), "{:?}", errors);
    }

    #[test]
    fn description_des_routes() {
        let c = cfg("server { listen 1; location /a { allow_methods GET DELETE; root www; index i.html; autoindex on; }
                     location /b { return /c; } location /d { cgi_extension .py; upload_store up; } }");
        let r = &c.servers[0].routes;
        assert_eq!(methods(&r[0]), "GET DELETE");
        assert_eq!(route_kind(&r[0]), "root www  ·  index i.html  ·  autoindex");
        assert_eq!(route_kind(&r[1]), "301 → /c");
        assert_eq!(route_kind(&r[2]), "cgi .py  ·  upload → up");
    }

    #[test]
    fn sparkline_proportionnelle() {
        assert_eq!(sparkline(&[0.0, 1.0, 2.0, 7.0]), "▁▂▃█");
        assert_eq!(sparkline(&[5.0, 5.0]), "██");
    }

    #[test]
    fn sparkline_vide_ou_nulle() {
        assert_eq!(sparkline(&[]), "");
        assert_eq!(sparkline(&[0.0, 0.0, 0.0]), "▁▁▁");
    }

    #[test]
    fn lecture_json_robuste() {
        let j = r#"{"a":1,"b": 22 ,"c":"x","d":-3,"z":9}"#;
        assert_eq!(num(j, "b"), 22);
        assert_eq!(num(j, "z"), 9);
        assert_eq!(num(j, "absent"), 0);
        assert_eq!(num(j, "c"), 0); // valeur texte : pas un nombre
        assert_eq!(num(j, "d"), 0); // compteur négatif impossible : ignoré
        assert_eq!(text(j, "c"), "x");
        assert_eq!(text(j, "absent"), "");
    }

    #[test]
    fn lit_le_json_produit_par_les_statistiques() {
        let mut s = crate::stats::Stats::new();
        s.record("GET", 200, 120, 0, 512);
        s.record("GET", 500, 80, 0, 0);
        let j = s.to_json(VERSION, 7, &["127.0.0.1:8080".into()]);
        assert_eq!(num(&j, "requests"), 2);
        assert_eq!(num(&j, "s5xx"), 1);
        assert_eq!(num(&j, "bytes_out"), 512);
        assert_eq!(text(&j, "version"), VERSION);
        assert_eq!(text(&j, "listeners"), "127.0.0.1:8080");
    }
}
