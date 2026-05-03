pub mod reactor;
pub mod client;

use std::collections::HashMap;
use std::net::TcpListener;
use std::os::unix::io::{AsRawFd, RawFd};
use libc::{EPOLLIN, EPOLLOUT};

use crate::config::Config;
use crate::server::reactor::Reactor;
use crate::server::client::{Client, ClientState};
use crate::http::response::HttpResponse;

pub struct Server {
    config: Config,
    reactor: Reactor,
    listeners: HashMap<RawFd, crate::config::ServerConfig>,
    clients: HashMap<RawFd, Client>,
    pipe_to_client: HashMap<RawFd, (RawFd, bool)>, // pipe_fd -> (client_fd, is_stdout)
}

impl Server {
    pub fn new(config: Config) -> std::io::Result<Self> {
        let reactor = Reactor::new()?;
        Ok(Self {
            config,
            reactor,
            listeners: HashMap::new(),
            clients: HashMap::new(),
            pipe_to_client: HashMap::new(),
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

    pub fn setup_listeners(&mut self) -> std::io::Result<()> {
        for srv_config in &self.config.servers {
            for port in &srv_config.ports {
                let addr = format!("{}:{}", srv_config.host, port);
                let listener = TcpListener::bind(&addr)?;
                let fd = listener.as_raw_fd();
                
                Self::set_nonblocking(fd)?;
                
                // Add listener to reactor, interested in EPOLLIN (incoming connections)
                self.reactor.add(fd, EPOLLIN as u32, fd as u64)?;
                
                // Keep the listener alive by preventing it from being dropped
                // In a real app we might store the TcpListener itself, but storing the mapping is fine
                std::mem::forget(listener);
                
                self.listeners.insert(fd, srv_config.clone());
                println!("Listening on {}", addr);
            }
        }
        Ok(())
    }

    pub fn run(&mut self) -> std::io::Result<()> {
        let mut events = vec![unsafe { std::mem::zeroed() }; 1024];

        println!("Server started. Press Ctrl+C to stop.");
        loop {
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
        }
    }

    fn check_timeouts(&mut self) -> std::io::Result<()> {
        let now = std::time::Instant::now();
        let mut timed_out = Vec::new();

        for (&fd, client) in &self.clients {
            if let Some(timeout_secs) = client.server_config.timeout {
                if now.duration_since(client.last_activity).as_secs() >= timeout_secs {
                    timed_out.push(fd);
                }
            }
        }

        for fd in timed_out {
            println!("Client {} timed out, closing connection", fd);
            self.reactor.remove(fd)?;
            self.clients.remove(&fd);
            unsafe { libc::close(fd); }
        }

        Ok(())
    }

    fn accept_connection(&mut self, listener_fd: RawFd) -> std::io::Result<()> {
        let srv_config = self.listeners.get(&listener_fd).unwrap().clone();
        
        // Use libc::accept to accept the connection manually
        let mut addr: libc::sockaddr = unsafe { std::mem::zeroed() };
        let mut len: libc::socklen_t = std::mem::size_of::<libc::sockaddr>() as libc::socklen_t;
        
        let client_fd = unsafe {
            libc::accept(listener_fd, &mut addr as *mut _, &mut len)
        };

        if client_fd < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(());
            }
            return Err(err);
        }

        Self::set_nonblocking(client_fd)?;
        self.reactor.add(client_fd, EPOLLIN as u32, client_fd as u64)?;
        
        let client = Client::new(client_fd, srv_config);
        self.clients.insert(client_fd, client);
        
        Ok(())
    }

    fn handle_client(&mut self, fd: RawFd, events: u32) -> std::io::Result<()> {
        let mut done = false;
        
        if let Some(client) = self.clients.get_mut(&fd) {
            // Handle reads
            if (events & EPOLLIN as u32) != 0 && client.state == ClientState::ReadingRequest {
                match client.read_data() {
                    Ok(finished) => {
                        if finished && client.state == ClientState::Processing {
                            match client.process_request() {
                                Err(info) => {
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
                        eprintln!("Error reading from client: {}", e);
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
                        eprintln!("Error writing to client: {}", e);
                        done = true;
                    }
                }
            }
        }

        if done {
            self.reactor.remove(fd)?;
            if let Some(client) = self.clients.remove(&fd) {
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
            unsafe { libc::close(fd); }
        }

        Ok(())
    }

    fn handle_cgi_pipe(&mut self, pipe_fd: RawFd, client_fd: RawFd, is_stdout: bool, events: u32) -> std::io::Result<()> {
        if let Some(client) = self.clients.get_mut(&client_fd) {
            if is_stdout {
                if (events & EPOLLIN as u32) != 0 {
                    match client.read_cgi_stdout() {
                        Ok(finished) => {
                            if finished {
                                // CGI done! Remove pipe from reactor and switch client to writing response
                                self.reactor.remove(pipe_fd)?;
                                self.pipe_to_client.remove(&pipe_fd);
                                self.reactor.modify(client_fd, EPOLLOUT as u32, client_fd as u64)?;
                                
                                // Clean up CGI process
                                if let Some(pid) = client.cgi_pid {
                                    unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0); }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("CGI Stdout error: {}", e);
                            self.reactor.remove(pipe_fd)?;
                            self.pipe_to_client.remove(&pipe_fd);
                            // Set error response
                            let mut res = HttpResponse::new(crate::http::response::HttpStatus::InternalServerError);
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
                            eprintln!("CGI Stdin error: {}", e);
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
