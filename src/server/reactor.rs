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

#[cfg(test)]
mod tests {
    use super::*;

    fn tube() -> (RawFd, RawFd) {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    }

    fn fermer(fds: &[RawFd]) {
        for &fd in fds {
            unsafe { libc::close(fd) };
        }
    }

    fn evenements(r: &Reactor, timeout: c_int) -> Vec<(u32, u64)> {
        let mut ev = vec![unsafe { std::mem::zeroed::<epoll_event>() }; 8];
        let n = r.wait(&mut ev, timeout).unwrap();
        ev[..n].iter().map(|e| (e.events, e.u64)).collect()
    }

    #[test]
    fn aucun_evenement_avant_le_timeout() {
        let r = Reactor::new().unwrap();
        let (lec, ecr) = tube();
        r.add(lec, libc::EPOLLIN as u32, 7).unwrap();
        assert!(evenements(&r, 10).is_empty());
        fermer(&[lec, ecr]);
    }

    #[test]
    fn signale_les_donnees_disponibles_avec_la_cle_utilisateur() {
        let r = Reactor::new().unwrap();
        let (lec, ecr) = tube();
        r.add(lec, libc::EPOLLIN as u32, 42).unwrap();
        assert_eq!(unsafe { libc::write(ecr, b"x".as_ptr() as *const _, 1) }, 1);
        let ev = evenements(&r, 100);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].1, 42);
        assert!(ev[0].0 & libc::EPOLLIN as u32 != 0);
        fermer(&[lec, ecr]);
    }

    #[test]
    fn modify_change_les_interets() {
        let r = Reactor::new().unwrap();
        let (lec, ecr) = tube();
        r.add(ecr, libc::EPOLLIN as u32, 1).unwrap();
        assert!(evenements(&r, 10).is_empty()); // un tube n'est jamais lisible côté écriture
        r.modify(ecr, libc::EPOLLOUT as u32, 2).unwrap();
        let ev = evenements(&r, 100);
        assert_eq!(ev, vec![(libc::EPOLLOUT as u32, 2)]);
        fermer(&[lec, ecr]);
    }

    #[test]
    fn remove_arrete_la_surveillance_et_tolere_un_double_retrait() {
        let r = Reactor::new().unwrap();
        let (lec, ecr) = tube();
        r.add(lec, libc::EPOLLIN as u32, 1).unwrap();
        r.remove(lec).unwrap();
        unsafe { libc::write(ecr, b"x".as_ptr() as *const _, 1) };
        assert!(evenements(&r, 10).is_empty());
        r.remove(lec).unwrap(); // ENOENT ignoré
        fermer(&[lec, ecr]);
    }

    #[test]
    fn fermeture_de_l_autre_extremite_signalee() {
        let r = Reactor::new().unwrap();
        let (lec, ecr) = tube();
        r.add(lec, libc::EPOLLIN as u32, 3).unwrap();
        fermer(&[ecr]);
        let ev = evenements(&r, 100);
        assert_eq!(ev.len(), 1);
        assert!(ev[0].0 & libc::EPOLLHUP as u32 != 0);
        fermer(&[lec]);
    }

    #[test]
    fn erreurs_du_systeme_remontees() {
        let r = Reactor::new().unwrap();
        let (lec, ecr) = tube();
        r.add(lec, libc::EPOLLIN as u32, 1).unwrap();
        assert!(r.add(lec, libc::EPOLLIN as u32, 1).is_err()); // EEXIST
        assert!(r.modify(ecr, libc::EPOLLOUT as u32, 1).is_err()); // ENOENT : jamais ajouté
        assert!(r.add(-1, libc::EPOLLIN as u32, 1).is_err()); // EBADF
        fermer(&[lec, ecr]);
    }

    #[test]
    fn plusieurs_descripteurs_surveilles() {
        let r = Reactor::new().unwrap();
        let (l1, e1) = tube();
        let (l2, e2) = tube();
        r.add(l1, libc::EPOLLIN as u32, 1).unwrap();
        r.add(l2, libc::EPOLLIN as u32, 2).unwrap();
        unsafe {
            libc::write(e1, b"a".as_ptr() as *const _, 1);
            libc::write(e2, b"b".as_ptr() as *const _, 1);
        }
        let mut cles: Vec<u64> = evenements(&r, 100).into_iter().map(|e| e.1).collect();
        cles.sort();
        assert_eq!(cles, vec![1, 2]);
        fermer(&[l1, e1, l2, e2]);
    }
}
