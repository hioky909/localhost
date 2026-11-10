//! Rendu terminal : couleurs ANSI (désactivées hors TTY ou avec NO_COLOR), tailles, durées.
use std::sync::atomic::{AtomicBool, Ordering};

static COLOR: AtomicBool = AtomicBool::new(false);

/// Active les couleurs si la sortie est un terminal (ou si FORCE_COLOR est défini).
pub fn init(no_color_flag: bool) {
    let tty = unsafe { libc::isatty(1) } == 1;
    let forced = std::env::var_os("FORCE_COLOR").is_some();
    let disabled = no_color_flag || std::env::var_os("NO_COLOR").is_some();
    COLOR.store(!disabled && (tty || forced), Ordering::Relaxed);
}

pub fn enabled() -> bool {
    COLOR.load(Ordering::Relaxed)
}

pub fn paint(code: &str, s: &str) -> String {
    if enabled() {
        format!("\x1b[{}m{}\x1b[0m", code, s)
    } else {
        s.to_string()
    }
}

pub fn bold(s: &str) -> String { paint("1", s) }
pub fn dim(s: &str) -> String { paint("2", s) }
pub fn green(s: &str) -> String { paint("32", s) }
pub fn red(s: &str) -> String { paint("31", s) }
pub fn yellow(s: &str) -> String { paint("33", s) }
pub fn cyan(s: &str) -> String { paint("36", s) }
pub fn magenta(s: &str) -> String { paint("35", s) }
pub fn blue(s: &str) -> String { paint("34", s) }
pub fn gray(s: &str) -> String { paint("90", s) }
/// Couleur de marque (orange Rust, 256 couleurs).
pub fn brand(s: &str) -> String { paint("38;5;208", s) }
pub fn brand_bold(s: &str) -> String { paint("1;38;5;208", s) }

/// Méthode HTTP colorée (sans remplissage).
pub fn method_word(m: &str) -> String {
    match m {
        "GET" => paint("1;36", m),
        "POST" => paint("1;33", m),
        "DELETE" => paint("1;31", m),
        _ => paint("1;35", m),
    }
}

/// Méthode HTTP colorée, alignée sur 6 caractères.
pub fn method(m: &str) -> String {
    pad(&method_word(m), 6)
}

/// Code de statut coloré par classe (2xx vert, 3xx bleu, 4xx jaune, 5xx rouge).
pub fn status(code: u16) -> String {
    let s = code.to_string();
    match code {
        200..=299 => paint("1;32", &s),
        300..=399 => paint("1;34", &s),
        400..=499 => paint("1;33", &s),
        _ => paint("1;31", &s),
    }
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    if n < 1024 {
        return format!("{} B", n);
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", v, UNITS[i])
}

pub fn micros(us: u64) -> String {
    if us < 1_000 {
        format!("{} µs", us)
    } else if us < 1_000_000 {
        format!("{:.2} ms", us as f64 / 1_000.0)
    } else {
        format!("{:.2} s", us as f64 / 1_000_000.0)
    }
}

pub fn uptime(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{}h {:02}m {:02}s", h, m, s)
    } else if m > 0 {
        format!("{}m {:02}s", m, s)
    } else {
        format!("{}s", s)
    }
}

/// Longueur affichée (sans les séquences ANSI).
pub fn visible_len(s: &str) -> usize {
    let mut n = 0;
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c == 'm' {
                in_esc = false;
            }
        } else if c == '\x1b' {
            in_esc = true;
        } else {
            n += 1;
        }
    }
    n
}

pub fn pad(s: &str, width: usize) -> String {
    let len = visible_len(s);
    if len >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - len))
    }
}

pub fn pad_left(s: &str, width: usize) -> String {
    let len = visible_len(s);
    if len >= width {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(width - len), s)
    }
}

/// Barre horizontale proportionnelle (blocs Unicode).
pub fn bar(value: u64, max: u64, width: usize) -> String {
    if max == 0 {
        return " ".repeat(width);
    }
    let filled = ((value as f64 / max as f64) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!("{}{}", "█".repeat(filled), gray(&"░".repeat(width - filled)))
}

/// Heure locale HH:MM:SS.mmm (fuseau du système via localtime_r).
pub fn clock() -> String {
    let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
    unsafe { libc::gettimeofday(&mut tv, std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = tv.tv_sec;
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:02}:{:02}:{:02}.{:03}", tm.tm_hour, tm.tm_min, tm.tm_sec, tv.tv_usec / 1000)
}
