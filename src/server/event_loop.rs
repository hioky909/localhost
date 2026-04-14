use std::collections::HashMap;
use crate::network::connection::{Connection, ConnectionState};
use crate::router::router::Router;
use crate::config::model::ServerConfig;
use crate::http::request::{parse_request, ParseResult};
use crate::http::response::HttpResponse;
use crate::http::methods::{handle_get, handle_head, handle_post, handle_delete};
use crate::cgi::executor::execute_cgi;
use crate::server::listener::accept_connection;
use crate::utils::error::{ServerError, Result};
use crate::utils::logger::{log_info, log_warn, log_debug};
use std::path::Path;

const MAX_EVENTS: usize = 64;
const TIMEOUT_SECS: u64 = 60;
const MAX_HEADER_SIZE: usize = 8192;

pub struct EventLoop {
    pub epoll_fd: i32,
    pub listener_fd: i32,
    pub connections: HashMap<i32, Connection>,
    pub router: Router,
    pub server_config: Vec<ServerConfig>,
}

impl EventLoop {
    pub fn new(listener_fd: i32, router: Router, configs: Vec<ServerConfig>) -> Result<Self> {
        let epoll_fd = unsafe { libc::epoll_create1(0) };
        if epoll_fd < 0 {
            return Err(ServerError::Io(std::io::Error::last_os_error()));
        }

        // Register listener
        let mut event = libc::epoll_event {
            events: (libc::EPOLLIN | libc::EPOLLET) as u32,
            u64: listener_fd as u64,
        };
        unsafe {
            if libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, listener_fd, &mut event) < 0 {
                libc::close(epoll_fd);
                return Err(ServerError::Io(std::io::Error::last_os_error()));
            }
        }

        Ok(EventLoop {
            epoll_fd,
            listener_fd,
            connections: HashMap::new(),
            router,
            server_config: configs,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let mut events = vec![libc::epoll_event { events: 0, u64: 0 }; MAX_EVENTS];
        log_info("Event loop started");

        loop {
            // Check shutdown signal
            if crate::SHUTDOWN.load(std::sync::atomic::Ordering::Relaxed) {
                log_info("Shutdown signal received");
                break;
            }

            // SINGLE epoll_wait call per iteration
            let n = unsafe {
                libc::epoll_wait(
                    self.epoll_fd,
                    events.as_mut_ptr(),
                    MAX_EVENTS as i32,
                    5000, // 5s timeout
                )
            };

            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(ServerError::Io(err));
            }

            for i in 0..n as usize {
                let fd = events[i].u64 as i32;
                let ev = events[i].events;

                if fd == self.listener_fd {
                    if let Err(e) = self.handle_new_connection() {
                        log_warn(&format!("Error accepting connection: {}", e));
                    }
                } else {
                    let rdhup = ev & (libc::EPOLLRDHUP as u32) != 0;
                    let hup = ev & (libc::EPOLLHUP as u32) != 0;
                    let err_ev = ev & (libc::EPOLLERR as u32) != 0;

                    if rdhup || hup || err_ev {
                        self.close_connection(fd);
                        continue;
                    }

                    if ev & (libc::EPOLLIN as u32) != 0 {
                        if let Err(e) = self.handle_read(fd) {
                            log_debug(&format!("Read error on fd {}: {}", fd, e));
                            self.close_connection(fd);
                        }
                    }

                    if ev & (libc::EPOLLOUT as u32) != 0 {
                        if let Err(e) = self.handle_write(fd) {
                            log_debug(&format!("Write error on fd {}: {}", fd, e));
                            self.close_connection(fd);
                        }
                    }
                }
            }

            self.cleanup_timeouts();
        }

        Ok(())
    }

    fn handle_new_connection(&mut self) -> Result<()> {
        loop {
            match accept_connection(self.listener_fd)? {
                None => break,
                Some(fd) => {
                    log_debug(&format!("New connection fd={}", fd));
                    let conn = Connection::new(fd);
                    self.connections.insert(fd, conn);

                    let mut event = libc::epoll_event {
                        events: (libc::EPOLLIN | libc::EPOLLET | libc::EPOLLRDHUP) as u32,
                        u64: fd as u64,
                    };
                    unsafe {
                        if libc::epoll_ctl(self.epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut event) < 0 {
                            let e = std::io::Error::last_os_error();
                            log_warn(&format!("epoll_ctl ADD failed: {}", e));
                            libc::close(fd);
                            self.connections.remove(&fd);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn handle_read(&mut self, fd: i32) -> Result<()> {
        let conn = match self.connections.get_mut(&fd) {
            Some(c) => c,
            None => return Ok(()),
        };

        conn.last_active = std::time::Instant::now();

        loop {
            let mut buf = [0u8; 8192];
            let n = unsafe {
                libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
            };

            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::WouldBlock {
                    break;
                }
                return Err(ServerError::Io(err));
            } else if n == 0 {
                // EOF
                return Err(ServerError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "connection closed",
                )));
            }

            let n = n as usize;
            conn.read_buf.extend_from_slice(&buf[..n]);

            // Check max body size
            let max_size = self.server_config.first()
                .map(|s| s.client_max_body_size)
                .unwrap_or(10485760);
            if conn.read_buf.len() > max_size + MAX_HEADER_SIZE {
                let resp = HttpResponse::error(413, "Payload Too Large");
                conn.write_buf = resp.to_bytes();
                conn.state = ConnectionState::Writing;
                conn.bytes_written = 0;
                self.register_write(fd);
                return Ok(());
            }

            // Try to parse
            let read_buf = conn.read_buf.clone();
            match parse_request(&read_buf) {
                ParseResult::Complete(_) => {
                    conn.state = ConnectionState::Processing;
                    break;
                }
                ParseResult::Incomplete => {
                    // continue reading
                }
                ParseResult::Error(code) => {
                    let resp = HttpResponse::error(code, "Bad Request");
                    conn.write_buf = resp.to_bytes();
                    conn.state = ConnectionState::Writing;
                    conn.bytes_written = 0;
                    self.register_write(fd);
                    return Ok(());
                }
            }
        }

        // If processing state, dispatch request
        let state = match self.connections.get(&fd) {
            Some(c) => matches!(c.state, ConnectionState::Processing),
            None => false,
        };
        if state {
            if let Err(e) = self.process_request(fd) {
                log_warn(&format!("process_request error: {}", e));
                if let Some(conn) = self.connections.get_mut(&fd) {
                    let resp = HttpResponse::error(500, "Internal Server Error");
                    conn.write_buf = resp.to_bytes();
                    conn.state = ConnectionState::Writing;
                    conn.bytes_written = 0;
                    self.register_write(fd);
                }
            }
        }

        Ok(())
    }

    fn handle_write(&mut self, fd: i32) -> Result<()> {
        let conn = match self.connections.get_mut(&fd) {
            Some(c) => c,
            None => return Ok(()),
        };

        conn.last_active = std::time::Instant::now();

        loop {
            if conn.bytes_written >= conn.write_buf.len() {
                break;
            }

            let remaining = &conn.write_buf[conn.bytes_written..];
            let n = unsafe {
                libc::write(fd, remaining.as_ptr() as *const libc::c_void, remaining.len())
            };

            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::WouldBlock {
                    break;
                }
                return Err(ServerError::Io(err));
            } else if n == 0 {
                break;
            }

            conn.bytes_written += n as usize;
        }

        // Check if done writing
        if conn.bytes_written >= conn.write_buf.len() {
            let keep_alive = conn.keep_alive;
            if keep_alive {
                conn.reset_for_keep_alive();
                // Re-register for read
                let mut event = libc::epoll_event {
                    events: (libc::EPOLLIN | libc::EPOLLET | libc::EPOLLRDHUP) as u32,
                    u64: fd as u64,
                };
                unsafe {
                    if libc::epoll_ctl(self.epoll_fd, libc::EPOLL_CTL_MOD, fd, &mut event) < 0 {
                        let e = std::io::Error::last_os_error();
                        log_warn(&format!("epoll_ctl MOD (read) failed for fd {}: {}", fd, e));
                    }
                }
            } else {
                self.close_connection(fd);
            }
        }

        Ok(())
    }

    fn close_connection(&mut self, fd: i32) {
        if self.connections.remove(&fd).is_some() {
            unsafe {
                libc::epoll_ctl(self.epoll_fd, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut());
                libc::close(fd);
            }
            log_debug(&format!("Closed connection fd={}", fd));
        }
    }

    fn process_request(&mut self, fd: i32) -> Result<()> {
        let conn = match self.connections.get_mut(&fd) {
            Some(c) => c,
            None => return Ok(()),
        };

        let read_buf = conn.read_buf.clone();
        let req = match parse_request(&read_buf) {
            ParseResult::Complete(r) => r,
            _ => {
                let resp = HttpResponse::error(400, "Bad Request");
                conn.write_buf = resp.to_bytes();
                conn.state = ConnectionState::Writing;
                conn.bytes_written = 0;
                self.register_write(fd);
                return Ok(());
            }
        };

        // Determine keep-alive
        let conn_header = req.headers.get("connection").map(|s| s.to_lowercase());
        let keep_alive = match conn_header.as_deref() {
            Some("keep-alive") => true,
            Some("close") => false,
            _ => req.version == "HTTP/1.1",
        };

        log_info(&format!("{} {} {}", req.method, req.path, req.version));

        // Route request
        let server_addr = self.server_config.first()
            .map(|s| s.listen_addr.clone())
            .unwrap_or_default();

        let resp = match self.router.route(&req, &server_addr) {
            None => HttpResponse::error(404, "No matching route"),
            Some(route) => {
                // Check method allowed
                let method_upper = req.method.to_uppercase();
                if !route.location.methods.contains(&method_upper) {
                    let allowed = route.location.methods.join(", ");
                    HttpResponse::error(405, "Method Not Allowed")
                        .with_header("Allow", &allowed)
                } else {
                    // Check CGI
                    let path_ext = Path::new(&req.path)
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| format!(".{}", e))
                        .unwrap_or_default();

                    if !route.location.cgi.is_empty() && route.location.cgi.contains_key(&path_ext) {
                        let interpreter = route.location.cgi[&path_ext].clone();
                        let root = &route.effective_root;
                        let rel = req.path.trim_start_matches('/');
                        let script_path = format!("{}/{}", root, rel);
                        let server_name = route.server.server_name.clone();
                        let port = server_addr.split(':').nth(1).unwrap_or("8080").to_string();
                        execute_cgi(&req, &script_path, &interpreter, &server_name, &port)
                    } else {
                        match method_upper.as_str() {
                            "GET" => handle_get(&req, route.server, route.location, &route.effective_root),
                            "HEAD" => handle_head(&req, route.server, route.location, &route.effective_root),
                            "POST" => handle_post(&req, route.server, route.location, &route.effective_root),
                            "DELETE" => handle_delete(&req, route.server, route.location, &route.effective_root),
                            _ => HttpResponse::error(501, "Not Implemented"),
                        }
                    }
                }
            }
        };

        // Set keep-alive header
        let resp = if keep_alive {
            resp.with_header("Connection", "keep-alive")
        } else {
            resp.with_header("Connection", "close")
        };

        let conn = match self.connections.get_mut(&fd) {
            Some(c) => c,
            None => return Ok(()),
        };
        conn.keep_alive = keep_alive;
        conn.write_buf = resp.to_bytes();
        conn.state = ConnectionState::Writing;
        conn.bytes_written = 0;

        self.register_write(fd);

        Ok(())
    }

    fn register_write(&self, fd: i32) {
        let mut event = libc::epoll_event {
            events: (libc::EPOLLOUT | libc::EPOLLET | libc::EPOLLRDHUP) as u32,
            u64: fd as u64,
        };
        unsafe {
            if libc::epoll_ctl(self.epoll_fd, libc::EPOLL_CTL_MOD, fd, &mut event) < 0 {
                let e = std::io::Error::last_os_error();
                crate::utils::logger::log_warn(&format!("epoll_ctl MOD (write) failed for fd {}: {}", fd, e));
            }
        }
    }

    fn cleanup_timeouts(&mut self) {
        let now = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(TIMEOUT_SECS);
        let to_close: Vec<i32> = self.connections.iter()
            .filter(|(_, c)| now.duration_since(c.last_active) > timeout)
            .map(|(fd, _)| *fd)
            .collect();
        for fd in to_close {
            log_debug(&format!("Timeout: closing fd={}", fd));
            self.close_connection(fd);
        }
    }
}

impl Drop for EventLoop {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.epoll_fd);
        }
    }
}
