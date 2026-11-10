//! Journal du serveur : une ligne par requête (format « access ») et des événements typés.
use crate::term;

/// Ligne d'accès : heure, méthode, statut, chemin, taille, durée, client, hôte virtuel.
pub fn access(method: &str, status: u16, path: &str, bytes_out: u64, micros: u64, peer: &str, vhost: &str) {
    let mut p = path.to_string();
    if p.chars().count() > 34 {
        p = format!("{}…", p.chars().take(33).collect::<String>());
    }
    let slow = micros >= 50_000;
    let dur = term::micros(micros);
    println!(
        "{}  {} {} {} {} {}  {}  {}",
        term::gray(&term::clock()),
        term::method(method),
        term::pad(&term::status(status), 4),
        term::pad(&p, 34),
        term::pad_left(&term::dim(&term::bytes(bytes_out)), 9),
        term::pad_left(&if slow { term::yellow(&dur) } else { term::dim(&dur) }, 10),
        term::gray(&term::pad(peer, 21)),
        term::gray(vhost)
    );
}

fn event(tag: &str, colored_tag: String, msg: &str) {
    let _ = tag;
    println!("{}  {} {}", term::gray(&term::clock()), colored_tag, msg);
}

pub fn info(tag: &str, msg: &str) {
    event(tag, term::cyan(&format!("{:<6}", tag)), msg);
}

pub fn ok(tag: &str, msg: &str) {
    event(tag, term::green(&format!("{:<6}", tag)), msg);
}

pub fn warn(tag: &str, msg: &str) {
    event(tag, term::yellow(&format!("{:<6}", tag)), msg);
}

pub fn error(tag: &str, msg: &str) {
    eprintln!("{}  {} {}", term::gray(&term::clock()), term::red(&format!("{:<6}", tag)), msg);
}
