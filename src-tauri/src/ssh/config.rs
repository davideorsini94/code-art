//! Lettura (solo lettura) di `~/.ssh/config`: i blocchi `Host` con alias concreti. Si legge il minimo che serve a
//! mostrare l'elenco; OpenSSH applica la configurazione da sé quando ci si connette per alias.

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConfigHost {
    pub alias: String,
    pub host_name: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
}

/// Divide una riga in (chiave in minuscolo, resto): separatore spazio/tab oppure `=`. Commenti e righe vuote: `None`.
fn split_line(line: &str) -> Option<(String, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let end = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let key = line[..end].to_ascii_lowercase();
    let rest = line[end..].trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim();
    (!rest.is_empty()).then_some((key, rest))
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(s)
}

fn is_concrete(alias: &str) -> bool {
    !alias.is_empty() && !alias.contains(['*', '?', '!'])
}

/// Blocchi `Host` con alias concreti (scartati `*`, `?`, `!`); `Include` e `Match` non si interpretano. Per ogni
/// chiave vale il primo valore, come in OpenSSH.
pub fn parse_ssh_config(text: &str) -> Vec<ConfigHost> {
    let mut out: Vec<ConfigHost> = Vec::new();
    // Indici in `out` del blocco corrente (un blocco può avere più alias); vuoto fuori da un `Host`.
    let mut current: Vec<usize> = Vec::new();
    let mut set: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
    for line in text.lines() {
        let Some((key, rest)) = split_line(line) else { continue };
        match key.as_str() {
            "host" => {
                current.clear();
                set.clear();
                for alias in rest.split_whitespace().take_while(|t| !t.starts_with('#')).map(unquote) {
                    if is_concrete(alias) && !out.iter().any(|h| h.alias == alias) {
                        out.push(ConfigHost { alias: alias.to_string(), ..Default::default() });
                        current.push(out.len() - 1);
                    }
                }
            }
            "match" => {
                current.clear();
                set.clear();
            }
            "hostname" | "user" | "port" | "identityfile" => {
                let name: &'static str = match key.as_str() {
                    "hostname" => "hostname",
                    "user" => "user",
                    "port" => "port",
                    _ => "identityfile",
                };
                if !set.insert(name) {
                    continue;
                }
                let value = unquote(rest);
                for &i in &current {
                    let h = &mut out[i];
                    match name {
                        "hostname" => h.host_name = Some(value.to_string()),
                        "user" => h.user = Some(value.to_string()),
                        "port" => h.port = value.parse().ok().filter(|p| *p > 0),
                        _ => h.identity_file = Some(value.to_string()),
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_aliases_and_fields() {
        let t = "Host web db\n  HostName example.test\n  User dev\n  Port 2222\n  IdentityFile ~/.ssh/id_dev\n\nHost other\n  HostName o.example.test\n";
        let h = parse_ssh_config(t);
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].alias, "web");
        assert_eq!(h[1].alias, "db");
        assert_eq!(h[1].host_name.as_deref(), Some("example.test"));
        assert_eq!(h[0].user.as_deref(), Some("dev"));
        assert_eq!(h[0].port, Some(2222));
        assert_eq!(h[0].identity_file.as_deref(), Some("~/.ssh/id_dev"));
        assert_eq!(h[2].user, None);
    }

    #[test]
    fn wildcards_are_dropped() {
        let h = parse_ssh_config("Host *\n  User x\nHost *.example.test web? !bad ok\n  HostName a\n");
        assert_eq!(h.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(), ["ok"]);
        assert_eq!(h[0].host_name.as_deref(), Some("a"));
    }

    #[test]
    fn keys_are_case_insensitive_and_equals_is_a_separator() {
        let h = parse_ssh_config("hOsT=srv\n  HOSTNAME = example.test\n\tuser=dev\n  PORT=22\n");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].host_name.as_deref(), Some("example.test"));
        assert_eq!(h[0].user.as_deref(), Some("dev"));
        assert_eq!(h[0].port, Some(22));
    }

    #[test]
    fn comments_include_and_match_are_ignored_and_first_value_wins() {
        let t = "# commento\nInclude ~/.ssh/other\nHost a # no\n  # User nobody\n  User first\n  User second\nMatch host a\n  User matched\n";
        let h = parse_ssh_config(t);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].alias, "a");
        assert_eq!(h[0].user.as_deref(), Some("first"));
    }

    #[test]
    fn bad_port_is_ignored() {
        let h = parse_ssh_config("Host a\n Port abc\nHost b\n Port 0\n");
        assert_eq!(h[0].port, None);
        assert_eq!(h[1].port, None);
    }
}
