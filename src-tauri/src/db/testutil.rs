//! Cluster Postgres temporaneo per i test di integrazione (`#[ignore]`): `initdb` + `pg_ctl`, utente `codeart`,
//! password `test-pass`, porta libera. Richiede `initdb`/`pg_ctl` nel PATH (o in /opt/homebrew/bin).

/// Cluster temporaneo: `initdb` + `pg_ctl` in una cartella temporanea, porta libera, socket nella stessa cartella.
pub struct Cluster {
    pub dir: tempfile::TempDir,
    pub port: u16,
    pub pg_ctl: std::path::PathBuf,
}

pub fn find_bin(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).chain([std::path::PathBuf::from("/opt/homebrew/bin")]).map(|d| d.join(name)).find(|p| p.is_file())
}

impl Cluster {
    pub fn start() -> Option<Cluster> {
        use std::process::Command;
        let initdb = find_bin("initdb")?;
        let pg_ctl = find_bin("pg_ctl")?;
        let dir = tempfile::tempdir().unwrap();
        let pw = dir.path().join("pw");
        std::fs::write(&pw, "test-pass\n").unwrap();
        let data = dir.path().join("data");
        let st = Command::new(initdb)
            .env("LC_ALL", "en_US.UTF-8")
            .args(["-D"]).arg(&data)
            .args(["-U", "codeart", "--auth=scram-sha-256", "--locale=en_US.UTF-8"])
            .arg(format!("--pwfile={}", pw.display()))
            .output()
            .unwrap();
        assert!(st.status.success(), "initdb: {}", String::from_utf8_lossy(&st.stderr));
        let port = super::tunnel::pick_port().unwrap();
        let opts = format!("-p {port} -k {} -c listen_addresses=127.0.0.1", dir.path().display());
        let st = Command::new(&pg_ctl)
            .env("LC_ALL", "en_US.UTF-8")
            .args(["-D"]).arg(&data)
            .args(["-o", &opts, "-w", "-l"]).arg(dir.path().join("log"))
            .arg("start")
            .output()
            .unwrap();
        assert!(st.status.success(), "pg_ctl start: {} {}", String::from_utf8_lossy(&st.stderr), std::fs::read_to_string(dir.path().join("log")).unwrap_or_default());
        Some(Cluster { dir, port, pg_ctl })
    }

    pub fn stop(&self) {
        let _ = std::process::Command::new(&self.pg_ctl)
            .env("LC_ALL", "en_US.UTF-8")
            .args(["-D"]).arg(self.dir.path().join("data"))
            .args(["-m", "immediate", "-w", "stop"])
            .output();
    }
}

impl Drop for Cluster {
    fn drop(&mut self) {
        self.stop();
    }
}
