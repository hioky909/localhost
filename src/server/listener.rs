use std::net::SocketAddr;
use crate::utils::error::{ServerError, Result};

pub fn create_listener(addr: &str) -> Result<i32> {
    let sock_addr: SocketAddr = addr.parse().map_err(|e| {
        ServerError::Config(format!("Invalid address '{}': {}", addr, e))
    })?;

    unsafe {
        let fd = libc::socket(
            libc::AF_INET,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK,
            0,
        );
        if fd < 0 {
            return Err(ServerError::Io(std::io::Error::last_os_error()));
        }

        // SO_REUSEADDR
        let opt: libc::c_int = 1;
        if libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &opt as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        ) < 0 {
            libc::close(fd);
            return Err(ServerError::Io(std::io::Error::last_os_error()));
        }

        // Build sockaddr_in
        let ip_bytes = match sock_addr.ip() {
            std::net::IpAddr::V4(ipv4) => ipv4.octets(),
            _ => return Err(ServerError::Config("Only IPv4 supported".to_string())),
        };
        let port = sock_addr.port();

        let mut addr_in: libc::sockaddr_in = std::mem::zeroed();
        addr_in.sin_family = libc::AF_INET as libc::sa_family_t;
        addr_in.sin_port = port.to_be();
        addr_in.sin_addr.s_addr = u32::from_be_bytes(ip_bytes).to_be();

        if libc::bind(
            fd,
            &addr_in as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ) < 0 {
            libc::close(fd);
            return Err(ServerError::Io(std::io::Error::last_os_error()));
        }

        if libc::listen(fd, 128) < 0 {
            libc::close(fd);
            return Err(ServerError::Io(std::io::Error::last_os_error()));
        }

        Ok(fd)
    }
}

pub fn accept_connection(listener_fd: i32) -> Result<Option<i32>> {
    unsafe {
        let mut addr: libc::sockaddr_in = std::mem::zeroed();
        let mut addrlen = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;

        let fd = libc::accept4(
            listener_fd,
            &mut addr as *mut _ as *mut libc::sockaddr,
            &mut addrlen,
            libc::SOCK_NONBLOCK,
        );

        if fd < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(ServerError::Io(err));
        }

        Ok(Some(fd))
    }
}
