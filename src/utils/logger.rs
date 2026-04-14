use std::time::{SystemTime, UNIX_EPOCH};

fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

pub fn log_info(msg: &str) {
    println!("[{}] [INFO]  {}", timestamp(), msg);
}

pub fn log_error(msg: &str) {
    eprintln!("[{}] [ERROR] {}", timestamp(), msg);
}

pub fn log_debug(msg: &str) {
    if std::env::var("DEBUG").is_ok() {
        println!("[{}] [DEBUG] {}", timestamp(), msg);
    }
}

pub fn log_warn(msg: &str) {
    println!("[{}] [WARN]  {}", timestamp(), msg);
}
