//! Config em %APPDATA%\monitorbt\config.ini
//!
//! ```ini
//! threshold=20
//! interval_min=5
//! hidden=2CDA46E97245
//! ```

use std::collections::HashSet;
use std::path::PathBuf;

pub struct Config {
    pub threshold: u8,
    pub interval_min: u32,
    pub hidden: HashSet<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self { threshold: 20, interval_min: 5, hidden: HashSet::new() }
    }
}

fn path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("monitorbt").join("config.ini"))
}

impl Config {
    pub fn load() -> Self {
        let mut cfg = Config::default();
        let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else {
            return cfg;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "threshold" => cfg.threshold = v.parse().unwrap_or(cfg.threshold).min(100),
                "interval_min" => cfg.interval_min = v.parse().unwrap_or(cfg.interval_min).max(1),
                "hidden" if !v.is_empty() => {
                    cfg.hidden.insert(v.to_ascii_uppercase());
                }
                _ => {}
            }
        }
        cfg
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut s = format!("threshold={}\ninterval_min={}\n", self.threshold, self.interval_min);
        let mut hidden: Vec<_> = self.hidden.iter().collect();
        hidden.sort();
        for h in hidden {
            s.push_str(&format!("hidden={h}\n"));
        }
        let _ = std::fs::write(p, s);
    }
}
