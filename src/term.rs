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

#[cfg(test)]
mod tests {
    // Les couleurs restent désactivées pendant les tests (init() n'est jamais appelé) :
    // on vérifie le rendu texte, et le calcul de largeur sur des séquences ANSI écrites à la main.
    use super::*;

    #[test]
    fn sans_couleur_le_texte_est_inchange() {
        assert!(!enabled());
        assert_eq!(paint("1;32", "ok"), "ok");
        assert_eq!(status(404), "404");
        assert_eq!(method_word("GET"), "GET");
    }

    #[test]
    fn tailles_lisibles() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1024), "1.0 KB");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn tailles_plafonnees_au_giga() {
        assert_eq!(bytes(2048 * 1024 * 1024 * 1024), "2048.0 GB");
    }

    #[test]
    fn durees_en_micro_milli_et_secondes() {
        assert_eq!(micros(0), "0 µs");
        assert_eq!(micros(999), "999 µs");
        assert_eq!(micros(1_000), "1.00 ms");
        assert_eq!(micros(12_345), "12.35 ms");
        assert_eq!(micros(1_000_000), "1.00 s");
        assert_eq!(micros(2_500_000), "2.50 s");
    }

    #[test]
    fn temps_de_fonctionnement() {
        assert_eq!(uptime(0), "0s");
        assert_eq!(uptime(59), "59s");
        assert_eq!(uptime(60), "1m 00s");
        assert_eq!(uptime(3599), "59m 59s");
        assert_eq!(uptime(3600), "1h 00m 00s");
        assert_eq!(uptime(90061), "25h 01m 01s");
    }

    #[test]
    fn largeur_visible_ignore_les_sequences_ansi() {
        assert_eq!(visible_len("abc"), 3);
        assert_eq!(visible_len("\x1b[1;32mabc\x1b[0m"), 3);
        assert_eq!(visible_len("\x1b[38;5;208m◆\x1b[0m x"), 3);
        assert_eq!(visible_len(""), 0);
    }

    #[test]
    fn largeur_visible_compte_les_caracteres_unicode() {
        assert_eq!(visible_len("µs"), 2);
        assert_eq!(visible_len("é→█"), 3);
    }

    #[test]
    fn remplissage_a_droite_et_a_gauche() {
        assert_eq!(pad("ab", 5), "ab   ");
        assert_eq!(pad_left("ab", 5), "   ab");
        assert_eq!(pad("abcdef", 3), "abcdef");
        assert_eq!(pad_left("abcdef", 3), "abcdef");
    }

    #[test]
    fn remplissage_tient_compte_des_couleurs() {
        let colore = "\x1b[32mok\x1b[0m";
        assert_eq!(visible_len(&pad(colore, 6)), 6);
        assert!(pad(colore, 6).starts_with(colore));
        assert_eq!(visible_len(&pad_left(colore, 6)), 6);
    }

    #[test]
    fn methode_alignee_sur_six_colonnes() {
        assert_eq!(method("GET"), "GET   ");
        assert_eq!(method("DELETE"), "DELETE");
        assert_eq!(method("OPTIONS"), "OPTIONS");
    }

    #[test]
    fn barre_proportionnelle() {
        assert_eq!(bar(5, 10, 10), format!("{}{}", "█".repeat(5), "░".repeat(5)));
        assert_eq!(bar(10, 10, 4), "████");
        assert_eq!(bar(0, 10, 4), "░░░░");
    }

    #[test]
    fn barre_bornee_et_maximum_nul() {
        assert_eq!(bar(50, 10, 4), "████");
        assert_eq!(bar(3, 0, 4), "    ");
        assert_eq!(visible_len(&bar(1, 3, 7)), 7);
    }

    #[test]
    fn horloge_au_format_hh_mm_ss_mmm() {
        let c = clock();
        let b = c.as_bytes();
        assert_eq!(c.len(), 12, "{}", c);
        assert!(b[2] == b':' && b[5] == b':' && b[8] == b'.');
        assert!(c.chars().filter(|ch| ch.is_ascii_digit()).count() == 9);
    }
}
