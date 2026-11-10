//! Compteurs en mémoire, mis à jour par la boucle d'événements (un seul thread : pas de verrou).
use std::collections::VecDeque;
use std::time::Instant;

const WINDOW: usize = 4096;

pub struct Stats {
    pub started: Instant,
    pub requests: u64,
    pub by_class: [u64; 6], // index = status / 100
    pub by_method: [u64; 4], // GET, POST, DELETE, autre
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub accepted: u64,
    pub active: u64,
    pub peak: u64,
    pub timeouts: u64,
    pub cgi_runs: u64,
    pub cgi_failures: u64,
    pub uploads: u64,
    latencies: VecDeque<u64>,
    tick_at: Instant,
    tick_requests: u64,
}

impl Stats {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            requests: 0,
            by_class: [0; 6],
            by_method: [0; 4],
            bytes_in: 0,
            bytes_out: 0,
            accepted: 0,
            active: 0,
            peak: 0,
            timeouts: 0,
            cgi_runs: 0,
            cgi_failures: 0,
            uploads: 0,
            latencies: VecDeque::with_capacity(WINDOW),
            tick_at: now,
            tick_requests: 0,
        }
    }

    pub fn connection_opened(&mut self) {
        self.accepted += 1;
        self.active += 1;
        self.peak = self.peak.max(self.active);
    }

    pub fn connection_closed(&mut self) {
        self.active = self.active.saturating_sub(1);
    }

    pub fn record(&mut self, method: &str, status: u16, micros: u64, bytes_in: u64, bytes_out: u64) {
        self.requests += 1;
        let class = ((status / 100) as usize).min(5);
        self.by_class[class] += 1;
        let m = match method {
            "GET" => 0,
            "POST" => 1,
            "DELETE" => 2,
            _ => 3,
        };
        self.by_method[m] += 1;
        self.bytes_in += bytes_in;
        self.bytes_out += bytes_out;
        if self.latencies.len() == WINDOW {
            self.latencies.pop_front();
        }
        self.latencies.push_back(micros);
    }

    /// Percentile (0–100) sur les 4096 dernières requêtes, en microsecondes.
    pub fn percentile(&self, p: f64) -> u64 {
        if self.latencies.is_empty() {
            return 0;
        }
        let mut v: Vec<u64> = self.latencies.iter().copied().collect();
        v.sort_unstable();
        let idx = ((p / 100.0) * (v.len() - 1) as f64).round() as usize;
        v[idx.min(v.len() - 1)]
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// Requêtes depuis le dernier « tick » et débit correspondant ; remet le compteur à zéro.
    pub fn tick(&mut self) -> (u64, f64) {
        let n = self.requests - self.tick_requests;
        let secs = self.tick_at.elapsed().as_secs_f64().max(0.001);
        self.tick_requests = self.requests;
        self.tick_at = Instant::now();
        (n, n as f64 / secs)
    }

    pub fn success_rate(&self) -> f64 {
        if self.requests == 0 {
            return 100.0;
        }
        // Les 4xx sont des erreurs du client : seul un 5xx est un échec du serveur.
        (self.requests - self.by_class[5]) as f64 * 100.0 / self.requests as f64
    }

    /// Instantané JSON (plat) servi sur /__localhost/status.
    pub fn to_json(&self, version: &str, pid: u32, listeners: &[String]) -> String {
        format!(
            concat!(
                "{{\"server\":\"localhost\",\"version\":\"{}\",\"pid\":{},\"uptime_s\":{},",
                "\"listeners\":\"{}\",\"requests\":{},\"active\":{},\"peak\":{},\"accepted\":{},",
                "\"s1xx\":{},\"s2xx\":{},\"s3xx\":{},\"s4xx\":{},\"s5xx\":{},",
                "\"get\":{},\"post\":{},\"delete\":{},\"other\":{},",
                "\"bytes_in\":{},\"bytes_out\":{},\"p50_us\":{},\"p95_us\":{},\"p99_us\":{},",
                "\"timeouts\":{},\"cgi_runs\":{},\"cgi_failures\":{},\"uploads\":{}}}"
            ),
            version,
            pid,
            self.uptime_secs(),
            listeners.join(","),
            self.requests,
            self.active,
            self.peak,
            self.accepted,
            self.by_class[1],
            self.by_class[2],
            self.by_class[3],
            self.by_class[4],
            self.by_class[5],
            self.by_method[0],
            self.by_method[1],
            self.by_method[2],
            self.by_method[3],
            self.bytes_in,
            self.bytes_out,
            self.percentile(50.0),
            self.percentile(95.0),
            self.percentile(99.0),
            self.timeouts,
            self.cgi_runs,
            self.cgi_failures,
            self.uploads
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_et_taux_de_succes() {
        let mut s = Stats::new();
        for i in 1..=100 {
            s.record("GET", if i <= 95 { 200 } else { 500 }, i * 10, 100, 1000);
        }
        assert_eq!(s.requests, 100);
        assert_eq!(s.by_class[2], 95);
        assert_eq!(s.percentile(50.0), 510);
        assert!((s.success_rate() - 95.0).abs() < 1e-9);
        s.record("GET", 404, 10, 0, 0);
        assert!((s.success_rate() - 95.0 * 100.0 / 101.0 - 100.0 / 101.0).abs() < 1e-9);
        assert!(s.to_json("1.0.0", 1, &["127.0.0.1:8080".into()]).contains("\"s5xx\":5"));
    }

    #[test]
    fn etat_initial() {
        let s = Stats::new();
        assert_eq!(s.requests, 0);
        assert_eq!(s.percentile(50.0), 0);
        assert_eq!(s.success_rate(), 100.0);
        assert_eq!(s.uptime_secs(), 0);
    }

    #[test]
    fn connexions_actives_et_pic() {
        let mut s = Stats::new();
        s.connection_opened();
        s.connection_opened();
        s.connection_opened();
        s.connection_closed();
        assert_eq!((s.accepted, s.active, s.peak), (3, 2, 3));
        s.connection_closed();
        s.connection_closed();
        s.connection_closed(); // fermeture en trop : pas de dépassement négatif
        assert_eq!((s.active, s.peak), (0, 3));
    }

    #[test]
    fn repartition_par_methode_et_par_classe() {
        let mut s = Stats::new();
        for (m, code) in [("GET", 200), ("GET", 304), ("POST", 201), ("DELETE", 204), ("PUT", 501), ("GET", 404), ("GET", 101)] {
            s.record(m, code, 1, 0, 0);
        }
        assert_eq!(s.by_method, [4, 1, 1, 1]);
        assert_eq!(s.by_class, [0, 1, 3, 1, 1, 1]);
    }

    #[test]
    fn statut_hors_norme_classe_comme_erreur_serveur() {
        let mut s = Stats::new();
        s.record("GET", 999, 1, 0, 0);
        assert_eq!(s.by_class[5], 1);
    }

    #[test]
    fn cumule_les_octets() {
        let mut s = Stats::new();
        s.record("POST", 201, 1, 1000, 50);
        s.record("GET", 200, 1, 80, 4096);
        assert_eq!((s.bytes_in, s.bytes_out), (1080, 4146));
    }

    #[test]
    fn percentiles_extremes() {
        let mut s = Stats::new();
        for v in [30, 10, 20] {
            s.record("GET", 200, v, 0, 0);
        }
        assert_eq!(s.percentile(0.0), 10);
        assert_eq!(s.percentile(50.0), 20);
        assert_eq!(s.percentile(100.0), 30);
        assert_eq!(s.percentile(250.0), 30);
    }

    #[test]
    fn fenetre_glissante_de_4096_mesures() {
        let mut s = Stats::new();
        for _ in 0..WINDOW {
            s.record("GET", 200, 1_000_000, 0, 0);
        }
        for _ in 0..WINDOW {
            s.record("GET", 200, 5, 0, 0);
        }
        // Les anciennes mesures (lentes) sont sorties de la fenêtre.
        assert_eq!(s.percentile(100.0), 5);
        assert_eq!(s.requests, 2 * WINDOW as u64);
    }

    #[test]
    fn tick_compte_les_requetes_depuis_le_precedent() {
        let mut s = Stats::new();
        for _ in 0..7 {
            s.record("GET", 200, 1, 0, 0);
        }
        let (n, debit) = s.tick();
        assert_eq!(n, 7);
        assert!(debit > 0.0);
        assert_eq!(s.tick().0, 0);
        s.record("GET", 200, 1, 0, 0);
        assert_eq!(s.tick().0, 1);
    }

    #[test]
    fn taux_de_succes_ignore_les_erreurs_client() {
        let mut s = Stats::new();
        s.record("GET", 404, 1, 0, 0);
        s.record("GET", 403, 1, 0, 0);
        assert_eq!(s.success_rate(), 100.0);
        s.record("GET", 502, 1, 0, 0);
        assert!((s.success_rate() - 200.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn json_complet_et_bien_forme() {
        let mut s = Stats::new();
        s.connection_opened();
        s.record("POST", 201, 42, 10, 20);
        s.cgi_runs = 3;
        s.uploads = 2;
        let j = s.to_json("9.9.9", 1234, &["127.0.0.1:8080".into(), "127.0.0.1:8081".into()]);
        assert!(j.starts_with('{') && j.ends_with('}'));
        for attendu in [
            "\"version\":\"9.9.9\"", "\"pid\":1234", "\"listeners\":\"127.0.0.1:8080,127.0.0.1:8081\"",
            "\"requests\":1", "\"active\":1", "\"s2xx\":1", "\"post\":1", "\"bytes_in\":10",
            "\"bytes_out\":20", "\"p50_us\":42", "\"cgi_runs\":3", "\"uploads\":2",
        ] {
            assert!(j.contains(attendu), "{} absent de {}", attendu, j);
        }
        assert_eq!(j.matches(':').count(), 27 + 2); // 27 clés + les « : » des adresses d'écoute
    }
}
