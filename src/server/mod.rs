pub mod reactor;
pub mod client;

use std::collections::HashMap;
use std::net::TcpListener;
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use libc::{EPOLLIN, EPOLLOUT};

use crate::config::Config;
use crate::log;
use crate::server::reactor::Reactor;
use crate::server::client::{Client, ClientState};
use crate::http::response::HttpResponse;
use crate::stats::Stats;
use crate::term;

/// Chemin du point de statut interne (réservé aux clients locaux).
pub const STATUS_PATH: &str = "/__localhost/status";

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

/// SIGINT / SIGTERM demandent un arrêt propre : la boucle termine son tour puis rend la main.
pub fn install_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGINT, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
    }
}

pub struct Listener {
    pub fd: RawFd,
    pub addr: String,
    pub names: Vec<String>,
}

struct CgiRun {
    started: Instant,
    script: String,
}

pub struct Server {
    config: Config,
    reactor: Reactor,
    listeners: HashMap<RawFd, Vec<crate::config::ServerConfig>>, // hôtes virtuels par socket
    pub bound: Vec<Listener>,
    clients: HashMap<RawFd, Client>,
    pipe_to_client: HashMap<RawFd, (RawFd, bool)>, // pipe_fd -> (client_fd, is_stdout)
    cgi_runs: HashMap<RawFd, CgiRun>,               // client_fd -> exécution CGI en cours
    pub stats: Stats,
    pub stats_every: Duration,
    pub version: &'static str,
}

impl Server {
    pub fn new(config: Config) -> std::io::Result<Self> {
        let reactor = Reactor::new()?;
        Ok(Self {
            config,
            reactor,
            listeners: HashMap::new(),
            bound: Vec::new(),
            clients: HashMap::new(),
            pipe_to_client: HashMap::new(),
            cgi_runs: HashMap::new(),
            stats: Stats::new(),
            stats_every: Duration::from_secs(10),
            version: env!("CARGO_PKG_VERSION"),
        })
    }

    /// Set a file descriptor to non-blocking mode
    fn set_nonblocking(fd: RawFd) -> std::io::Result<()> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
        if flags < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let res = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        if res < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Ouvre les sockets d'écoute. Un même host:port déclaré par plusieurs blocs `server`
    /// (hôtes virtuels) n'est ouvert qu'une fois : le premier bloc sert de défaut.
    pub fn setup_listeners(&mut self) -> std::io::Result<()> {
        let mut opened: HashMap<String, usize> = HashMap::new();
        for srv_config in &self.config.servers {
            for port in &srv_config.ports {
                let addr = format!("{}:{}", srv_config.host, port);
                if let Some(&i) = opened.get(&addr) {
                    let fd = self.bound[i].fd;
                    self.bound[i].names.extend(srv_config.server_names.iter().cloned());
                    let vhosts = self.listeners.get_mut(&fd).unwrap();
                    if !vhosts.iter().any(|v| std::ptr::eq(v, srv_config) || (v.server_names == srv_config.server_names && v.ports == srv_config.ports)) {
                        vhosts.push(srv_config.clone());
                    }
                    continue;
                }
                let listener = TcpListener::bind(&addr).map_err(|e| {
                    std::io::Error::new(e.kind(), format!("bind() to {} failed ({})", addr, e))
                })?;
                let fd = listener.as_raw_fd();

                Self::set_nonblocking(fd)?;

                // Add listener to reactor, interested in EPOLLIN (incoming connections)
                self.reactor.add(fd, EPOLLIN as u32, fd as u64)?;

                // The fd is owned by the server from now on (closed in shutdown()).
                std::mem::forget(listener);

                self.listeners.insert(fd, vec![srv_config.clone()]);
                opened.insert(addr.clone(), self.bound.len());
                self.bound.push(Listener { fd, addr, names: srv_config.server_names.clone() });
            }
        }
        Ok(())
    }

    pub fn run(&mut self) -> std::io::Result<()> {
        let mut events = vec![unsafe { std::mem::zeroed() }; 1024];
        let mut last_stats = Instant::now();

        while !SHUTDOWN.load(Ordering::SeqCst) {
            let n_events = self.reactor.wait(&mut events, 1000)?; // Block for up to 1s

            for i in 0..n_events {
                let event = &events[i];
                let fd = event.u64 as RawFd;

                if self.listeners.contains_key(&fd) {
                    // It's a listener: accept new connection
                    self.accept_connection(fd)?;
                } else if self.clients.contains_key(&fd) {
                    // It's a client: handle I/O
                    self.handle_client(fd, event.events)?;
                } else if let Some(&(client_fd, is_stdout)) = self.pipe_to_client.clone().get(&fd) {
                    // It's a CGI pipe event
                    self.handle_cgi_pipe(fd, client_fd, is_stdout, event.events)?;
                }
            }

            // After handling all events, check for timeouts
            self.check_timeouts()?;

            if last_stats.elapsed() >= self.stats_every {
                last_stats = Instant::now();
                self.print_stats_line();
            }
        }
        self.shutdown();
        Ok(())
    }

    /// Ligne de synthèse périodique, seulement s'il y a eu du trafic depuis la précédente.
    fn print_stats_line(&mut self) {
        let (n, rps) = self.stats.tick();
        if n == 0 {
            return;
        }
        let s = &self.stats;
        let ok = s.success_rate();
        let rate = if ok >= 99.0 { term::green(&format!("{:.1} %", ok)) } else if ok >= 95.0 { term::yellow(&format!("{:.1} %", ok)) } else { term::red(&format!("{:.1} %", ok)) };
        println!(
            "{}  {} {}  {}  {} {}  {} {}  {} {}  {}",
            term::gray(&term::clock()),
            term::magenta(&format!("{:<6}", "stats")),
            term::bold(&format!("{:.0} req/s", rps)),
            term::dim(&format!("{} req", n)),
            term::dim("ok"),
            rate,
            term::dim("p95"),
            term::micros(s.percentile(95.0)),
            term::dim("actives"),
            s.active,
            term::dim(&format!("total {} · ↑ {}", s.requests, term::bytes(s.bytes_out)))
        );
    }

    fn shutdown(&mut self) {
        println!();
        log::warn("signal", "arrêt demandé — fermeture propre");
        let active = self.clients.len();
        for (fd, _) in self.clients.drain() {
            let _ = self.reactor.remove(fd);
            unsafe { libc::close(fd); }
        }
        for l in &self.bound {
            let _ = self.reactor.remove(l.fd);
            unsafe { libc::close(l.fd); }
        }
        log::info("close", &format!("{} écoute(s) fermée(s), {} connexion(s) interrompue(s)", self.bound.len(), active));
        crate::cli::print_summary(self);
    }

    fn check_timeouts(&mut self) -> std::io::Result<()> {
        let now = std::time::Instant::now();
        let mut timed_out = Vec::new();

        for (&fd, client) in &self.clients {
            if let Some(timeout_secs) = client.server_config.timeout {
                if now.duration_since(client.last_activity).as_secs() >= timeout_secs {
                    timed_out.push((fd, timeout_secs));
                }
            }
        }

        for (fd, secs) in timed_out {
            self.reactor.remove(fd)?;
            if let Some(c) = self.clients.remove(&fd) {
                // Un CGI encore en cours pour ce client : on le termine et on libère ses pipes.
                if let Some(pid) = c.cgi_pid {
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                        libc::waitpid(pid, std::ptr::null_mut(), 0);
                    }
                    for p in [c.cgi_stdin_fd, c.cgi_stdout_fd].into_iter().flatten() {
                        let _ = self.reactor.remove(p);
                        if self.pipe_to_client.remove(&p).is_some() {
                            unsafe { libc::close(p); }
                        }
                    }
                    self.cgi_runs.remove(&fd);
                    self.stats.cgi_failures += 1;
                    log::warn("cgi", &format!("pid {} arrêté (client expiré)", pid));
                }
                let (m, p) = c.method_and_path();
                let what = if c.read_buffer.is_empty() { "connexion inactive".to_string() } else { format!("requête incomplète {} {}", m, p) };
                log::warn("idle", &format!("{} — {} depuis {} s, connexion fermée", term::gray(&c.peer), what, secs));
            }
            self.stats.timeouts += 1;
            self.stats.connection_closed();
            unsafe { libc::close(fd); }
        }

        Ok(())
    }

    fn accept_connection(&mut self, listener_fd: RawFd) -> std::io::Result<()> {
        let srv_config = self.listeners.get(&listener_fd).unwrap()[0].clone();

        let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        let mut len: libc::socklen_t = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;

        let client_fd = unsafe {
            libc::accept(listener_fd, &mut addr as *mut _ as *mut libc::sockaddr, &mut len)
        };

        if client_fd < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(());
            }
            return Err(err);
        }

        let ip = u32::from_be(addr.sin_addr.s_addr);
        let peer = format!(
            "{}.{}.{}.{}:{}",
            ip >> 24,
            (ip >> 16) & 0xff,
            (ip >> 8) & 0xff,
            ip & 0xff,
            u16::from_be(addr.sin_port)
        );

        Self::set_nonblocking(client_fd)?;
        self.reactor.add(client_fd, EPOLLIN as u32, client_fd as u64)?;

        let mut client = Client::new(client_fd, srv_config, peer);
        client.listener_fd = listener_fd;
        self.clients.insert(client_fd, client);
        self.stats.connection_opened();

        Ok(())
    }

    /// Réponse JSON du point de statut, servie hors routeur et réservée à 127.0.0.0/8.
    fn status_response(&self) -> Vec<u8> {
        let addrs: Vec<String> = self.bound.iter().map(|l| l.addr.clone()).collect();
        let body = self.stats.to_json(self.version, std::process::id(), &addrs);
        let mut res = HttpResponse::new(crate::http::response::HttpStatus::Ok);
        res.set_header("Content-Type", "application/json");
        res.set_header("Cache-Control", "no-store");
        res.set_body(body.into_bytes());
        res.to_bytes()
    }

    fn handle_client(&mut self, fd: RawFd, events: u32) -> std::io::Result<()> {
        let mut done = false;
        let status_body = self.status_response();

        if let Some(client) = self.clients.get_mut(&fd) {
            // Handle reads
            if (events & EPOLLIN as u32) != 0 && client.state == ClientState::ReadingRequest {
                match client.read_data() {
                    Ok(finished) => {
                        let is_status = client.state == ClientState::Processing
                            && client.request.as_ref().map(|r| r.path == STATUS_PATH).unwrap_or(false)
                            && client.peer.starts_with("127.");
                        if finished && is_status {
                            client.internal = true;
                            client.write_buffer = status_body;
                            client.state = ClientState::WritingResponse;
                            self.reactor.modify(fd, EPOLLOUT as u32, fd as u64)?;
                        } else if finished && client.state == ClientState::Processing {
                            // Hôte virtuel : choisit le bloc `server` dont server_name correspond à Host.
                            if let (Some(vhosts), Some(req)) = (self.listeners.get(&client.listener_fd), &client.request) {
                                if vhosts.len() > 1 {
                                    let host = req.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("host")).map(|(_, v)| v.split(':').next().unwrap_or("").to_lowercase());
                                    if let Some(h) = host {
                                        if let Some(v) = vhosts.iter().find(|v| v.server_names.iter().any(|n| n.eq_ignore_ascii_case(&h))) {
                                            client.server_config = v.clone();
                                        }
                                    }
                                }
                            }
                            match client.process_request() {
                                Err(info) => {
                                    let (_, script) = client.method_and_path();
                                    log::info("cgi", &format!("▸ {}  {}", script, term::gray(&format!("pid {}", info.pid))));
                                    self.stats.cgi_runs += 1;
                                    self.cgi_runs.insert(fd, CgiRun { started: Instant::now(), script });
                                    // Register pipes with reactor
                                    if client.state == ClientState::CgiWritingStdin {
                                        self.reactor.add(info.stdin_fd, EPOLLOUT as u32, info.stdin_fd as u64)?;
                                        self.pipe_to_client.insert(info.stdin_fd, (fd, false));
                                    } else {
                                        self.reactor.add(info.stdout_fd, EPOLLIN as u32, info.stdout_fd as u64)?;
                                        self.pipe_to_client.insert(info.stdout_fd, (fd, true));
                                        // Close stdin immediately if no body
                                        unsafe { libc::close(info.stdin_fd); }
                                    }
                                }
                                _ => {
                                    // Switch interest to writing response
                                    self.reactor.modify(fd, EPOLLOUT as u32, fd as u64)?;
                                }
                            }
                        } else if finished && client.state == ClientState::WritingResponse {
                            // State already set (e.g., body size limit reached), switch to writing
                            self.reactor.modify(fd, EPOLLOUT as u32, fd as u64)?;
                        }
                    }
                    Err(e) => {
                        log::error("read", &format!("{} — {}", client.peer, e));
                        done = true;
                    }
                }
            }

            // Handle writes (normal response)
            if (events & EPOLLOUT as u32) != 0 && client.state == ClientState::WritingResponse {
                match client.write_data() {
                    Ok(finished) => {
                        if finished {
                            done = true;
                        }
                    }
                    Err(e) => {
                        log::error("write", &format!("{} — {}", client.peer, e));
                        done = true;
                    }
                }
            }
        }

        if done {
            self.reactor.remove(fd)?;
            if let Some(client) = self.clients.remove(&fd) {
                self.finish(&client);
                // If there's a pending CGI, clean up pipes
                if let Some(stdin) = client.cgi_stdin_fd {
                    let _ = self.reactor.remove(stdin);
                    self.pipe_to_client.remove(&stdin);
                    unsafe { libc::close(stdin); }
                }
                if let Some(stdout) = client.cgi_stdout_fd {
                    let _ = self.reactor.remove(stdout);
                    self.pipe_to_client.remove(&stdout);
                    unsafe { libc::close(stdout); }
                }
            }
            self.cgi_runs.remove(&fd);
            self.stats.connection_closed();
            unsafe { libc::close(fd); }
        }

        Ok(())
    }

    /// Journalise et comptabilise une requête terminée.
    fn finish(&mut self, client: &Client) {
        if client.internal || client.write_buffer.is_empty() {
            return;
        }
        let status = client.status_code();
        let (method, path) = client.method_and_path();
        let micros = client.started_at.map(|t| t.elapsed().as_micros() as u64).unwrap_or(0);
        let bytes_out = client.written_bytes as u64;
        self.stats.record(&method, status, micros, client.read_buffer.len() as u64, bytes_out);
        if method == "POST" && status == 201 {
            self.stats.uploads += 1;
        }
        log::access(&method, status, &path, bytes_out, micros, &client.peer, &client.host());
    }

    fn handle_cgi_pipe(&mut self, pipe_fd: RawFd, client_fd: RawFd, is_stdout: bool, events: u32) -> std::io::Result<()> {
        if let Some(client) = self.clients.get_mut(&client_fd) {
            if is_stdout {
                // Un script terminé ferme son stdout : epoll signale EPOLLHUP, parfois sans EPOLLIN.
                if (events & (EPOLLIN | libc::EPOLLHUP | libc::EPOLLERR) as u32) != 0 {
                    match client.read_cgi_stdout() {
                        Ok(finished) => {
                            if finished {
                                // CGI done! Remove pipe from reactor and switch client to writing response
                                self.reactor.remove(pipe_fd)?;
                                self.pipe_to_client.remove(&pipe_fd);
                                self.reactor.modify(client_fd, EPOLLOUT as u32, client_fd as u64)?;

                                // Clean up CGI process and report its exit status
                                if let Some(pid) = client.cgi_pid {
                                    let mut status: libc::c_int = 0;
                                    unsafe { libc::waitpid(pid, &mut status, 0); }
                                    let code = if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { -1 };
                                    let took = self.cgi_runs.get(&client_fd).map(|r| r.started.elapsed().as_micros() as u64).unwrap_or(0);
                                    let script = self.cgi_runs.get(&client_fd).map(|r| r.script.clone()).unwrap_or_default();
                                    let empty = client.response.as_ref().map(|r| r.body.is_empty()).unwrap_or(true);
                                    if code != 0 && empty {
                                        // Le script a échoué sans rien produire : 502, comme une passerelle.
                                        let mut res = HttpResponse::new(crate::http::response::HttpStatus::BadGateway);
                                        res.set_header("Content-Type", "text/plain; charset=utf-8");
                                        res.set_body(b"502 Bad Gateway: le script CGI s'est termine en erreur\n".to_vec());
                                        client.write_buffer = res.to_bytes();
                                    }
                                    if code == 0 {
                                        log::ok("cgi", &format!("✓ {}  {}", script, term::gray(&format!("pid {} · exit 0 · {}", pid, term::micros(took)))));
                                    } else {
                                        self.stats.cgi_failures += 1;
                                        log::warn("cgi", &format!("✗ {}  {}", script, term::gray(&format!("pid {} · exit {} · {}", pid, code, term::micros(took)))));
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            log::error("cgi", &format!("stdout: {}", e));
                            self.stats.cgi_failures += 1;
                            self.reactor.remove(pipe_fd)?;
                            self.pipe_to_client.remove(&pipe_fd);
                            // Set error response
                            let res = HttpResponse::new(crate::http::response::HttpStatus::InternalServerError);
                            client.write_buffer = res.to_bytes();
                            client.state = ClientState::WritingResponse;
                            self.reactor.modify(client_fd, EPOLLOUT as u32, client_fd as u64)?;
                        }
                    }
                }
            } else {
                // Writing to stdin
                if (events & EPOLLOUT as u32) != 0 {
                    match client.write_cgi_stdin() {
                        Ok(finished) => {
                            if finished {
                                // Stdin writing done. Close it and start reading stdout.
                                self.reactor.remove(pipe_fd)?;
                                self.pipe_to_client.remove(&pipe_fd);
                                unsafe { libc::close(pipe_fd); }

                                if let Some(stdout) = client.cgi_stdout_fd {
                                    self.reactor.add(stdout, EPOLLIN as u32, stdout as u64)?;
                                    self.pipe_to_client.insert(stdout, (client_fd, true));
                                }
                            }
                        }
                        Err(e) => {
                            log::error("cgi", &format!("stdin: {}", e));
                            self.reactor.remove(pipe_fd)?;
                            self.pipe_to_client.remove(&pipe_fd);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Utilisé par `localhost status --watch` pour sortir proprement sur Ctrl+C.
pub fn shutdown_requested() -> bool {
    SHUTDOWN.load(Ordering::SeqCst)
}
