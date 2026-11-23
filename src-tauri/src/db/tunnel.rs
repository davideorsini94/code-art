//! Tunnel SSH verso un database: un inoltro locale (`ssh -O forward -L`) aggiunto alla connessione condivisa del
//! server SSH (tappa 3b). Il server SSH deve essere già connesso; l'inoltro sta solo su 127.0.0.1.
use crate::ssh::conn::{ConnState, Connections};
use crate::ssh::Server;
use std::net::TcpListener;

pub const NOT_CONNECTED: &str = "Connetti prima il server SSH";

/// Un inoltro aperto: serve per toglierlo (`close`).
#[derive(Debug, Clone, PartialEq)]
pub struct Tunnel {
    pub server: Server,
    pub spec: String,
    pub port: u16,
}

/// Una porta libera su 127.0.0.1: si lega una porta qualsiasi e la si rilascia (la finestra di tempo prima che
/// ssh la riapra è minima; `open` riprova una volta se nel frattempo è stata presa).
pub fn pick_port() -> Result<u16, String> {
    let l = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("nessuna porta locale libera: {e}"))?;
    l.local_addr().map(|a| a.port()).map_err(|e| e.to_string())
}

/// `127.0.0.1:<locale>:<host>:<porta>`; un indirizzo IPv6 va tra parentesi quadre.
pub fn forward_spec(local: u16, host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("127.0.0.1:{local}:[{host}]:{port}")
    } else {
        format!("127.0.0.1:{local}:{host}:{port}")
    }
}

fn port_in_use(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("address already in use") || e.contains("could not request local forwarding")
}

/// Apre l'inoltro verso `host:port` (raggiunti dal server SSH) e restituisce la porta locale.
pub fn open(conns: &Connections, server: &Server, host: &str, port: u16) -> Result<Tunnel, String> {
    open_with(conns.state_of(&server.id), pick_port, &mut |spec| conns.forward(server, spec), server, host, port)
}

fn open_with(
    state: Option<ConnState>,
    pick: impl Fn() -> Result<u16, String>,
    forward: &mut dyn FnMut(&str) -> Result<(), String>,
    server: &Server,
    host: &str,
    port: u16,
) -> Result<Tunnel, String> {
    if state != Some(ConnState::Connected) {
        return Err(NOT_CONNECTED.into());
    }
    let mut last = String::new();
    for _ in 0..2 {
        let local = pick()?;
        let spec = forward_spec(local, host, port);
        match forward(&spec) {
            Ok(()) => return Ok(Tunnel { server: server.clone(), spec, port: local }),
            Err(e) if port_in_use(&e) => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// Toglie l'inoltro (anche se la connessione SSH è già caduta: allora non c'è più nulla da togliere).
pub fn close(conns: &Connections, t: &Tunnel) {
    conns.cancel_forward(&t.server, &t.spec);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::ServerSource;

    fn server() -> Server {
        Server {
            id: "s1".into(),
            source: ServerSource::Saved,
            name: "s".into(),
            host: "bastion.test".into(),
            port: None,
            user: None,
            identity_file: None,
            start_dir: None,
            has_password: false,
        }
    }

    #[test]
    fn spec_is_loopback_only_and_brackets_ipv6() {
        assert_eq!(forward_spec(50000, "db.internal", 5432), "127.0.0.1:50000:db.internal:5432");
        assert_eq!(forward_spec(50000, "fe80::1", 5432), "127.0.0.1:50000:[fe80::1]:5432");
    }

    #[test]
    fn picked_port_is_free_and_changes() {
        let p = pick_port().unwrap();
        assert!(p > 0);
        // libera davvero: si può legare di nuovo
        drop(TcpListener::bind(("127.0.0.1", p)).unwrap());
    }

    #[test]
    fn needs_a_connected_ssh_server() {
        let mut never = |_: &str| -> Result<(), String> { panic!("nessun inoltro senza connessione") };
        for st in [None, Some(ConnState::Connecting), Some(ConnState::Disconnected), Some(ConnState::Error)] {
            assert_eq!(open_with(st, || Ok(1), &mut never, &server(), "h", 1).unwrap_err(), NOT_CONNECTED);
        }
    }

    #[test]
    fn retries_once_when_the_port_was_taken() {
        let mut specs = vec![];
        let ports = std::cell::Cell::new(40000u16);
        let pick = || {
            ports.set(ports.get() + 1);
            Ok(ports.get())
        };
        let mut fwd = |s: &str| {
            specs.push(s.to_string());
            if specs.len() == 1 { Err("bind [127.0.0.1]:40001: Address already in use".to_string()) } else { Ok(()) }
        };
        let t = open_with(Some(ConnState::Connected), pick, &mut fwd, &server(), "db", 5432).unwrap();
        assert_eq!(t.port, 40002);
        assert_eq!(t.spec, "127.0.0.1:40002:db:5432");
        assert_eq!(specs.len(), 2);
        // due volte: errore; un altro errore: nessun secondo tentativo
        let mut n = 0;
        let mut always = |_: &str| {
            n += 1;
            Err::<(), _>("Address already in use".to_string())
        };
        assert!(open_with(Some(ConnState::Connected), || Ok(5), &mut always, &server(), "db", 1).is_err());
        assert_eq!(n, 2);
        let mut m = 0;
        let mut other = |_: &str| {
            m += 1;
            Err::<(), _>("Permission denied".to_string())
        };
        assert_eq!(open_with(Some(ConnState::Connected), || Ok(5), &mut other, &server(), "db", 1).unwrap_err(), "Permission denied");
        assert_eq!(m, 1);
    }
}
