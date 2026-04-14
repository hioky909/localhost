use crate::http::request::HttpRequest;

pub enum ConnectionState {
    Reading,
    Processing,
    Writing,
    #[allow(dead_code)]
    Closed,
}

pub struct Connection {
    pub fd: i32,
    pub state: ConnectionState,
    pub read_buf: Vec<u8>,
    pub write_buf: Vec<u8>,
    pub request: Option<HttpRequest>,
    pub keep_alive: bool,
    pub last_active: std::time::Instant,
    pub bytes_written: usize,
}

impl Connection {
    pub fn new(fd: i32) -> Self {
        Connection {
            fd,
            state: ConnectionState::Reading,
            read_buf: Vec::with_capacity(4096),
            write_buf: Vec::new(),
            request: None,
            keep_alive: false,
            last_active: std::time::Instant::now(),
            bytes_written: 0,
        }
    }

    pub fn reset_for_keep_alive(&mut self) {
        self.state = ConnectionState::Reading;
        self.read_buf.clear();
        self.write_buf.clear();
        self.request = None;
        self.bytes_written = 0;
        self.last_active = std::time::Instant::now();
    }
}
