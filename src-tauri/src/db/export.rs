//! Esportazione dei risultati in CSV e JSON. Il percorso viene solo dal selettore nativo di salvataggio aperto dal core
//! (mai dalla WebView). Si scrive in un file temporaneo nella stessa cartella e poi si rinomina.
//!
//! Cosa contiene il file: tutte le righe del risultato (quelle già consegnate alla WebView, tenute in copia dal core, più
//! le restanti lette dal cursore), fino al limite di 100 000. Postgres: le celle oltre 64 KB sono quelle già troncate
//! (il core non tiene l'originale). MongoDB: i documenti sono interi, senza troncamenti. L'esportazione consuma il
//! cursore: dopo, il risultato è chiuso.
use super::mongo::{self, cell_text, doc_json, MongoSessions};
use super::pg::Sessions;
use super::pg_query::Queries;
use mongodb::bson::{Bson, Document};
use serde::Serialize;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Csv,
    Json,
}

impl Format {
    pub fn file_name(self) -> &'static str {
        match self {
            Format::Csv => "risultato.csv",
            Format::Json => "risultato.json",
        }
    }
    pub fn filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Format::Csv => ("CSV", &["csv"]),
            Format::Json => ("JSON", &["json"]),
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    pub path: String,
    pub rows: u64,
}

// ---- CSV (RFC 4180) ----

/// Un campo: `None` (NULL) = vuoto senza virgolette; stringa vuota = `""`; virgolette se contiene `,` `"` CR o LF o ha
/// spazi iniziali o finali; le virgolette si raddoppiano.
pub fn csv_field(v: Option<&str>, out: &mut String) {
    let Some(s) = v else { return };
    let needs = s.is_empty() || s.contains([',', '"', '\r', '\n']) || s.starts_with(' ') || s.ends_with(' ');
    if !needs {
        out.push_str(s);
        return;
    }
    out.push('"');
    for c in s.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
}

fn csv_row<'a>(fields: impl Iterator<Item = Option<&'a str>>, out: &mut String) {
    for (i, f) in fields.enumerate() {
        if i > 0 {
            out.push(',');
        }
        csv_field(f, out);
    }
    out.push_str("\r\n");
}

pub fn csv_table(columns: &[String], rows: &[Vec<Option<String>>]) -> String {
    let mut out = String::new();
    csv_row(columns.iter().map(|c| Some(c.as_str())), &mut out);
    for r in rows {
        csv_row(r.iter().map(|v| v.as_deref()), &mut out);
    }
    out
}

/// Colonne = unione delle chiavi di primo livello di tutti i documenti (ordine di prima comparsa, `_id` per prima).
pub fn mongo_csv(docs: &[Document]) -> String {
    let cols = mongo::columns_of(docs);
    let mut out = String::new();
    csv_row(cols.iter().map(|c| Some(c.as_str())), &mut out);
    for d in docs {
        let cells: Vec<Option<String>> = cols
            .iter()
            .map(|c| match d.get(c) {
                None | Some(Bson::Null) => None,
                Some(v) => Some(cell_text(v)),
            })
            .collect();
        csv_row(cells.iter().map(|v| v.as_deref()), &mut out);
    }
    out
}

// ---- JSON ----

/// Nomi di colonna unici: i doppi ricevono il suffisso `_2`, `_3`… (saltando i nomi già presi).
pub fn unique_names(columns: &[String]) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = columns.iter().cloned().collect();
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(columns.len());
    for c in columns {
        if used.insert(c.clone()) {
            out.push(c.clone());
            continue;
        }
        let mut n = 2;
        let name = loop {
            let cand = format!("{c}_{n}");
            if !seen.contains(&cand) && !used.contains(&cand) {
                break cand;
            }
            n += 1;
        };
        seen.insert(name.clone());
        used.insert(name.clone());
        out.push(name);
    }
    out
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Array di oggetti `{colonna: testo|null}`, con l'ordine delle colonne conservato.
pub fn pg_json(columns: &[String], rows: &[Vec<Option<String>>]) -> String {
    let names: Vec<String> = unique_names(columns).iter().map(|n| json_str(n)).collect();
    let mut out = String::from("[");
    for (ri, r) in rows.iter().enumerate() {
        out.push_str(if ri == 0 { "\n  {" } else { ",\n  {" });
        for (i, name) in names.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(name);
            out.push_str(": ");
            match r.get(i).and_then(|v| v.as_deref()) {
                Some(s) => out.push_str(&json_str(s)),
                None => out.push_str("null"),
            }
        }
        out.push('}');
    }
    out.push_str(if rows.is_empty() { "]\n" } else { "\n]\n" });
    out
}

/// Array dei documenti come JSON esteso rilassato (valori JSON veri, non stringhe), con l'ordine dei campi originale.
pub fn mongo_json(docs: &[Document]) -> String {
    let mut out = String::from("[");
    for (i, d) in docs.iter().enumerate() {
        out.push_str(if i == 0 { "\n  " } else { ",\n  " });
        out.push_str(&doc_json(d, false));
    }
    out.push_str(if docs.is_empty() { "]\n" } else { "\n]\n" });
    out
}

// ---- file ----

/// Scrive in un file temporaneo nella stessa cartella e rinomina; senza BOM (UTF-8).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let err = |e: std::io::Error| format!("Scrittura non riuscita: {e}");
    let name = path.file_name().ok_or("Percorso non valido")?.to_string_lossy().into_owned();
    let tmp = path.with_file_name(format!(".{name}.codeart-{}.tmp", std::process::id()));
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        err(e)
    })
}

/// Raccoglie tutte le righe del risultato (consumando il cursore) e le scrive in `path`. Restituisce il numero di righe.
pub async fn export_to(
    queries: &Queries,
    sessions: &Sessions,
    mongo_sessions: &MongoSessions,
    result_id: u64,
    format: Format,
    path: &Path,
) -> Result<u64, String> {
    let (text, rows) = if mongo::is_mongo_result(result_id) {
        let docs = mongo_sessions.drain(result_id).await?;
        let text = match format {
            Format::Csv => mongo_csv(&docs),
            Format::Json => mongo_json(&docs),
        };
        (text, docs.len())
    } else {
        let (cols, rows) = queries.drain(sessions, result_id).await?;
        let names: Vec<String> = cols.into_iter().map(|c| c.name).collect();
        let text = match format {
            Format::Csv => csv_table(&names, &rows),
            Format::Json => pg_json(&names, &rows),
        };
        (text, rows.len())
    };
    write_atomic(path, text.as_bytes())?;
    Ok(rows as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::pg_query::EXPIRED;
    use mongodb::bson::doc;

    fn s(v: &[Option<&str>]) -> Vec<Option<String>> {
        v.iter().map(|x| x.map(String::from)).collect()
    }

    fn field(v: Option<&str>) -> String {
        let mut o = String::new();
        csv_field(v, &mut o);
        o
    }

    #[test]
    fn csv_escaping() {
        assert_eq!(field(Some("abc")), "abc");
        assert_eq!(field(Some("a,b")), "\"a,b\"");
        assert_eq!(field(Some("di \"x\"")), "\"di \"\"x\"\"\"");
        assert_eq!(field(Some("a\nb")), "\"a\nb\"");
        assert_eq!(field(Some("a\rb")), "\"a\rb\"");
        assert_eq!(field(Some(" lead")), "\" lead\"");
        assert_eq!(field(Some("trail ")), "\"trail \"");
        assert_eq!(field(Some("in mezzo ok")), "in mezzo ok");
        assert_eq!(field(None), "");
        assert_eq!(field(Some("")), "\"\"");
        assert_eq!(field(Some("è ü 日本")), "è ü 日本");
    }

    #[test]
    fn csv_table_has_header_crlf_and_null_vs_empty() {
        let cols = vec!["a".to_string(), "b,c".to_string()];
        let rows = vec![s(&[Some("1"), None]), s(&[Some(""), Some("x\"y")])];
        assert_eq!(csv_table(&cols, &rows), "a,\"b,c\"\r\n1,\r\n\"\",\"x\"\"y\"\r\n");
        assert!(!csv_table(&cols, &rows).starts_with('\u{feff}'));
    }

    #[test]
    fn json_duplicates_and_nulls() {
        let cols = vec!["?column?".to_string(), "?column?".to_string(), "a".into(), "a".into(), "a_2".into(), "a".into()];
        assert_eq!(unique_names(&cols), ["?column?", "?column?_2", "a", "a_3", "a_2", "a_4"]);
        let rows = vec![s(&[Some("1"), Some("1"), None, Some("q\"\n"), Some(""), Some("é")])];
        let v: serde_json::Value = serde_json::from_str(&pg_json(&cols, &rows)).unwrap();
        assert_eq!(v[0]["?column?"], "1");
        assert_eq!(v[0]["?column?_2"], "1");
        assert!(v[0]["a"].is_null());
        assert_eq!(v[0]["a_3"], "q\"\n");
        assert_eq!(v[0]["a_2"], "");
        assert_eq!(v[0]["a_4"], "é");
        let v: serde_json::Value = serde_json::from_str(&pg_json(&["x".into()], &[])).unwrap();
        assert_eq!(v, serde_json::json!([]));
    }

    #[test]
    fn mongo_union_columns_and_json_values() {
        let docs = vec![doc! { "a": 1, "_id": 7, "nome": "x,y" }, doc! { "b": true, "a": Bson::Null, "_id": 8 }];
        assert_eq!(mongo_csv(&docs), "_id,a,nome,b\r\n7,1,\"x,y\",\r\n8,,,true\r\n");
        let v: serde_json::Value = serde_json::from_str(&mongo_json(&docs)).unwrap();
        assert_eq!(v[0]["nome"], "x,y");
        assert_eq!(v[1]["b"], true);
        assert!(v[1]["a"].is_null());
        assert_eq!(v.as_array().unwrap().len(), 2);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&mongo_json(&[])).unwrap(), serde_json::json!([]));
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("r.csv");
        std::fs::write(&p, "vecchio").unwrap();
        write_atomic(&p, b"nuovo").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "nuovo");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    #[ignore = "avvia un cluster Postgres temporaneo (initdb e pg_ctl nel PATH)"]
    fn live_cluster_exports_all_rows_after_one_page() {
        use super::super::pg::{PgParams, Sessions};
        use super::super::testutil::Cluster;
        use super::super::DbTls;
        use crate::ssh::conn::Connections;
        use crate::ssh::conn::SshPaths;
        use std::sync::Arc;
        let Some(cluster) = Cluster::start() else {
            eprintln!("initdb/pg_ctl non trovati: saltato");
            return;
        };
        let p = PgParams {
            host: "127.0.0.1".into(),
            port: cluster.port,
            user: Some("codeart".into()),
            database: Some("postgres".into()),
            tls: DbTls::Disable,
            accept_invalid_certs: false,
        };
        tauri::async_runtime::block_on(async {
            let conns = Arc::new(Connections::new(SshPaths::new(std::env::temp_dir()), Arc::new(|_| {})));
            let sessions = Arc::new(Sessions::new(conns.clone(), Arc::new(|_| {})));
            sessions.connect("c1", p, Some("test-pass".into()), None).await.unwrap();
            let mongo_sessions = MongoSessions::new(conns, Arc::new(|_| {}));
            let q = Queries::new();
            let sql = "SELECT n AS id, CASE WHEN n % 100 = 0 THEN NULL ELSE 'riga, \"' || n || '\"' END AS t, n AS id FROM generate_series(1, 1200) n";
            let r = q.query(&sessions, "c1", sql).await.unwrap();
            assert_eq!(r.rows.len(), 500);
            let rid = r.result_id.unwrap();
            // una pagina in più letta dalla WebView prima dell'esportazione
            assert_eq!(q.more(&sessions, rid).await.unwrap().rows.len(), 500);
            let dir = tempfile::tempdir().unwrap();
            let csv = dir.path().join("r.csv");
            let n = export_to(&q, &sessions, &mongo_sessions, rid, Format::Csv, &csv).await.unwrap();
            assert_eq!(n, 1200);
            let text = std::fs::read_to_string(&csv).unwrap();
            let lines: Vec<&str> = text.split("\r\n").collect();
            assert_eq!(lines.len(), 1202, "intestazione + 1200 righe + coda vuota");
            assert_eq!(lines[0], "id,t,id");
            assert_eq!(lines[1], "1,\"riga, \"\"1\"\"\",1");
            assert_eq!(lines[100], "100,,100");
            assert_eq!(lines[1200], "1200,,1200");
            // il risultato è finito (nessuna riga in più) ma tenuto: si può esportare di nuovo
            let p = q.more(&sessions, rid).await.unwrap();
            assert!(p.rows.is_empty() && !p.has_more);
            assert_eq!(export_to(&q, &sessions, &mongo_sessions, rid, Format::Csv, &csv).await.unwrap(), 1200);
            // JSON su un nuovo risultato
            let rid = q.query(&sessions, "c1", sql).await.unwrap().result_id.unwrap();
            let json = dir.path().join("r.json");
            assert_eq!(export_to(&q, &sessions, &mongo_sessions, rid, Format::Json, &json).await.unwrap(), 1200);
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
            let a = v.as_array().unwrap();
            assert_eq!(a.len(), 1200);
            assert_eq!(a[0]["id"], "1");
            assert_eq!(a[0]["id_2"], "1");
            assert!(a[99]["t"].is_null());
            assert_eq!(a[1199]["id"], "1200");
            // un risultato piccolo (3 righe, nessun cursore aperto) ha un resultId ed è esportabile
            let r = q.query(&sessions, "c1", "SELECT g AS n, NULL::text AS t FROM generate_series(1, 3) g").await.unwrap();
            assert!(!r.has_more);
            let rid = r.result_id.expect("risultato piccolo tenuto");
            let small = dir.path().join("s.csv");
            assert_eq!(export_to(&q, &sessions, &mongo_sessions, rid, Format::Csv, &small).await.unwrap(), 3);
            assert_eq!(std::fs::read_to_string(&small).unwrap(), "n,t\r\n1,\r\n2,\r\n3,\r\n");
            // una nuova query sostituisce il risultato; close è idempotente
            let r2 = q.query(&sessions, "c1", "SELECT 1 AS x").await.unwrap();
            assert_eq!(export_to(&q, &sessions, &mongo_sessions, rid, Format::Csv, &small).await.unwrap_err(), EXPIRED);
            q.close(&sessions, r2.result_id.unwrap()).await.unwrap();
            assert_eq!(export_to(&q, &sessions, &mongo_sessions, r2.result_id.unwrap(), Format::Csv, &small).await.unwrap_err(), EXPIRED);
        });
    }
}
