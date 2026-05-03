pub mod request;
pub mod response;
pub mod router;
pub mod mime;

#[derive(Debug, Clone)]
pub struct CgiInfo {
    pub pid: i32,
    pub stdin_fd: i32,
    pub stdout_fd: i32,
}
