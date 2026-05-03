use libc::{c_int, epoll_create1, epoll_ctl, epoll_event, epoll_wait, EPOLL_CTL_ADD, EPOLL_CTL_DEL, EPOLL_CTL_MOD};
use std::io::{Error, ErrorKind, Result};
use std::os::unix::io::RawFd;

pub struct Reactor {
    epoll_fd: RawFd,
}

impl Reactor {
    pub fn new() -> Result<Self> {
        let epoll_fd = unsafe { epoll_create1(0) };
        if epoll_fd < 0 {
            return Err(Error::last_os_error());
        }
        Ok(Self { epoll_fd })
    }

    pub fn add(&self, fd: RawFd, interests: u32, user_data: u64) -> Result<()> {
        let mut event = epoll_event {
            events: interests,
            u64: user_data,
        };
        let res = unsafe {
            epoll_ctl(self.epoll_fd, EPOLL_CTL_ADD, fd, &mut event)
        };
        if res < 0 {
            Err(Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn modify(&self, fd: RawFd, interests: u32, user_data: u64) -> Result<()> {
        let mut event = epoll_event {
            events: interests,
            u64: user_data,
        };
        let res = unsafe {
            epoll_ctl(self.epoll_fd, EPOLL_CTL_MOD, fd, &mut event)
        };
        if res < 0 {
            Err(Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn remove(&self, fd: RawFd) -> Result<()> {
        // According to epoll docs, the event pointer can be NULL for EPOLL_CTL_DEL
        // since Linux 2.6.9, but libc bindings expect a pointer. We can pass a dummy event.
        let mut dummy_event: epoll_event = unsafe { std::mem::zeroed() };
        let res = unsafe {
            epoll_ctl(self.epoll_fd, EPOLL_CTL_DEL, fd, &mut dummy_event)
        };
        if res < 0 {
            let err = Error::last_os_error();
            // Ignore ENOENT (No such file or directory) if already removed.
            if err.raw_os_error() == Some(libc::ENOENT) {
                return Ok(());
            }
            Err(err)
        } else {
            Ok(())
        }
    }

    /// Wait for events, blocking until at least one event occurs or the timeout expires.
    /// To block indefinitely, set timeout_ms to -1.
    pub fn wait(&self, events: &mut [epoll_event], timeout_ms: c_int) -> Result<usize> {
        let res = unsafe {
            epoll_wait(
                self.epoll_fd,
                events.as_mut_ptr(),
                events.len() as c_int,
                timeout_ms,
            )
        };
        if res < 0 {
            let err = Error::last_os_error();
            if err.kind() == ErrorKind::Interrupted {
                // Not a fatal error, just interrupted by signal
                return Ok(0);
            }
            return Err(err);
        }
        Ok(res as usize)
    }
}

impl Drop for Reactor {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.epoll_fd);
        }
    }
}
