use std::fmt;

#[derive(Debug)]
#[allow(dead_code)]
pub enum ServerError {
    Io(std::io::Error),
    Config(String),
    Http(u16, String),
    Cgi(String),
    ParseError(String),
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServerError::Io(e) => write!(f, "IO error: {}", e),
            ServerError::Config(s) => write!(f, "Config error: {}", s),
            ServerError::Http(code, msg) => write!(f, "HTTP {} error: {}", code, msg),
            ServerError::Cgi(s) => write!(f, "CGI error: {}", s),
            ServerError::ParseError(s) => write!(f, "Parse error: {}", s),
        }
    }
}

impl From<std::io::Error> for ServerError {
    fn from(e: std::io::Error) -> Self {
        ServerError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, ServerError>;
