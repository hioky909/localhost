use std::collections::HashMap;
use crate::http::request::HttpRequest;
use crate::http::response::HttpResponse;
use crate::utils::logger::{log_info, log_warn, log_error};

pub fn execute_cgi(
    req: &HttpRequest,
    script_path: &str,
    interpreter: &str,
    server_name: &str,
    server_port: &str,
) -> HttpResponse {
    log_info(&format!("CGI execute: {} via {}", script_path, interpreter));

    // Create pipes: [read_end, write_end]
    let mut stdin_pipe = [0i32; 2];
    let mut stdout_pipe = [0i32; 2];

    unsafe {
        if libc::pipe(stdin_pipe.as_mut_ptr()) < 0 {
            log_error("Failed to create stdin pipe");
            return HttpResponse::error(500, "CGI pipe error");
        }
        if libc::pipe(stdout_pipe.as_mut_ptr()) < 0 {
            log_error("Failed to create stdout pipe");
            libc::close(stdin_pipe[0]);
            libc::close(stdin_pipe[1]);
            return HttpResponse::error(500, "CGI pipe error");
        }

        let pid = libc::fork();
        if pid < 0 {
            log_error("Fork failed");
            libc::close(stdin_pipe[0]);
            libc::close(stdin_pipe[1]);
            libc::close(stdout_pipe[0]);
            libc::close(stdout_pipe[1]);
            return HttpResponse::error(500, "CGI fork error");
        }

        if pid == 0 {
            // Child process
            // Redirect stdin to read end of stdin_pipe
            libc::dup2(stdin_pipe[0], libc::STDIN_FILENO);
            libc::dup2(stdout_pipe[1], libc::STDOUT_FILENO);

            // Close unused pipe ends
            libc::close(stdin_pipe[0]);
            libc::close(stdin_pipe[1]);
            libc::close(stdout_pipe[0]);
            libc::close(stdout_pipe[1]);

            // Build environment
            let env_vars = build_env(req, script_path, server_name, server_port);

            // Convert to C strings
            let mut env_cstrings: Vec<std::ffi::CString> = Vec::new();
            for (k, v) in &env_vars {
                let kv = format!("{}={}", k, v);
                if let Ok(cs) = std::ffi::CString::new(kv) {
                    env_cstrings.push(cs);
                }
            }
            let mut env_ptrs: Vec<*const libc::c_char> = env_cstrings.iter().map(|s| s.as_ptr()).collect();
            env_ptrs.push(std::ptr::null());

            // argv: [interpreter, script_path, NULL]
            let interp_cstr = std::ffi::CString::new(interpreter).unwrap_or_default();
            let script_cstr = std::ffi::CString::new(script_path).unwrap_or_default();
            let argv: Vec<*const libc::c_char> = vec![
                interp_cstr.as_ptr(),
                script_cstr.as_ptr(),
                std::ptr::null(),
            ];

            libc::execve(interp_cstr.as_ptr(), argv.as_ptr(), env_ptrs.as_ptr());
            // execve failed
            libc::_exit(1);
        }

        // Parent process
        // Close unused ends
        libc::close(stdin_pipe[0]);
        libc::close(stdout_pipe[1]);

        // Write request body to child stdin
        if !req.body.is_empty() {
            let mut written = 0;
            while written < req.body.len() {
                let n = libc::write(
                    stdin_pipe[1],
                    req.body[written..].as_ptr() as *const libc::c_void,
                    req.body.len() - written,
                );
                if n <= 0 {
                    break;
                }
                written += n as usize;
            }
        }
        libc::close(stdin_pipe[1]);

        // Read CGI output with a loop limit
        let mut output = Vec::new();
        let mut buf = [0u8; 4096];
        // Set stdout_pipe[0] to non-blocking
        let flags = libc::fcntl(stdout_pipe[0], libc::F_GETFL, 0);
        libc::fcntl(stdout_pipe[0], libc::F_SETFL, flags | libc::O_NONBLOCK);

        // Wait for child to finish with timeout loop
        let mut iterations = 0;
        loop {
            let n = libc::read(stdout_pipe[0], buf.as_mut_ptr() as *mut libc::c_void, buf.len());
            if n > 0 {
                output.extend_from_slice(&buf[..n as usize]);
                iterations = 0;
            } else if n == 0 {
                break;
            } else {
                let err = *libc::__errno_location();
                if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                    // Check if child is done
                    let mut status = 0i32;
                    let wpid = libc::waitpid(pid, &mut status, libc::WNOHANG);
                    if wpid == pid {
                        // Child done, drain remaining
                        loop {
                            let n2 = libc::read(stdout_pipe[0], buf.as_mut_ptr() as *mut libc::c_void, buf.len());
                            if n2 <= 0 {
                                break;
                            }
                            output.extend_from_slice(&buf[..n2 as usize]);
                        }
                        break;
                    }
                    iterations += 1;
                    if iterations > 1000 {
                        log_warn("CGI timeout");
                        libc::kill(pid, libc::SIGTERM);
                        libc::waitpid(pid, &mut status, 0);
                        break;
                    }
                    // Small spin wait
                    std::thread::sleep(std::time::Duration::from_millis(5));
                } else {
                    break;
                }
            }
        }
        libc::close(stdout_pipe[0]);

        // Wait for child
        let mut status = 0i32;
        libc::waitpid(pid, &mut status, libc::WNOHANG);

        parse_cgi_output(&output)
    }
}

fn build_env(req: &HttpRequest, script_path: &str, server_name: &str, server_port: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("REQUEST_METHOD".to_string(), req.method.clone());
    env.insert("PATH_INFO".to_string(), req.path.clone());
    env.insert("QUERY_STRING".to_string(), req.query_string.clone());
    env.insert("CONTENT_LENGTH".to_string(),
        req.headers.get("content-length").cloned().unwrap_or_default());
    env.insert("CONTENT_TYPE".to_string(),
        req.headers.get("content-type").cloned().unwrap_or_default());
    env.insert("SERVER_NAME".to_string(), server_name.to_string());
    env.insert("SERVER_PORT".to_string(), server_port.to_string());
    env.insert("HTTP_COOKIE".to_string(),
        req.headers.get("cookie").cloned().unwrap_or_default());
    env.insert("SCRIPT_FILENAME".to_string(), script_path.to_string());
    env.insert("SCRIPT_NAME".to_string(), req.path.clone());
    env.insert("SERVER_SOFTWARE".to_string(), "localhost/0.1.0".to_string());
    env.insert("GATEWAY_INTERFACE".to_string(), "CGI/1.1".to_string());
    env
}

fn parse_cgi_output(output: &[u8]) -> HttpResponse {
    // Find end of CGI headers (\r\n\r\n or \n\n)
    let header_end_rn = output.windows(4).position(|w| w == b"\r\n\r\n");
    let header_end_n = output.windows(2).position(|w| w == b"\n\n");

    let (header_end, body_start) = match (header_end_rn, header_end_n) {
        (Some(a), Some(b)) => {
            if a <= b { (a, a + 4) } else { (b, b + 2) }
        }
        (Some(a), None) => (a, a + 4),
        (None, Some(b)) => (b, b + 2),
        (None, None) => {
            // No headers, treat entire output as body
            return HttpResponse::new(200).with_body(output.to_vec(), "text/html");
        }
    };

    let header_str = match std::str::from_utf8(&output[..header_end]) {
        Ok(s) => s,
        Err(_) => return HttpResponse::error(500, "CGI output encoding error"),
    };

    let body = output[body_start..].to_vec();
    let mut status = 200u16;
    let mut content_type = "text/html".to_string();
    let mut extra_headers: Vec<(String, String)> = Vec::new();

    for line in header_str.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_lowercase();
            let value = line[colon+1..].trim().to_string();
            match key.as_str() {
                "status" => {
                    let code: u16 = value.splitn(2, ' ').next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(200);
                    status = code;
                }
                "content-type" => {
                    content_type = value;
                }
                _ => {
                    extra_headers.push((line[..colon].trim().to_string(), value));
                }
            }
        }
    }

    let mut resp = HttpResponse::new(status).with_body(body, &content_type);
    for (k, v) in extra_headers {
        resp = resp.with_header(&k, &v);
    }
    resp
}
