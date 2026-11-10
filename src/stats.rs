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
}
