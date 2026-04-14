#[allow(dead_code)]
pub struct ReadBuffer {
    pub data: Vec<u8>,
}

#[allow(dead_code)]
impl ReadBuffer {
    pub fn new() -> Self {
        ReadBuffer { data: Vec::with_capacity(4096) }
    }

    pub fn read_from_fd(&mut self, fd: i32) -> std::io::Result<usize> {
        let mut tmp = [0u8; 4096];
        let n = unsafe { libc::read(fd, tmp.as_mut_ptr() as *mut libc::c_void, tmp.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            return Err(err);
        }
        let n = n as usize;
        self.data.extend_from_slice(&tmp[..n]);
        Ok(n)
    }
}
