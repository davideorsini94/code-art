//! Modifiche in griglia per Postgres: dai cambiamenti dell'utente alle istruzioni `UPDATE`/`INSERT`/`DELETE`.
//! `plan_edits` è pura: i nomi vengono citati con `quote_ident`, ogni valore è un parametro `$n::<tipo>` (testo
//! convertito dal server nel tipo della colonna, che arriva dal catalogo e mai dalla WebView). L'anteprima inserisce i
//! valori nel testo SOLO per essere letta: non viene mai eseguita.
use super::pg::Sessions;
use super::pg_query::{qerr, PgColumn, Queries};
use serde::{Deserialize, Serialize};
use tokio_postgres::types::Type;

pub const APPLY_TITLE: &str = "Applica modifiche";
pub const BTN_APPLY: &str = "Applica";
/// Valori più lunghi si accorciano nelle anteprime e nei messaggi.
pub const PREVIEW_VALUE_MAX: usize = 200;
/// Quante anteprime elenca la conferma nativa.
pub const CONFIRM_LISTED: usize = 10;
pub(super) const MAX_EDITS: usize = 10_000;

/// Una coppia (nome di colonna, valore testuale); `None` = NULL (distinto da la stringa vuota).
pub type Cell = (String, Option<String>);

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RowEdit {
    Update { pk: Vec<Cell>, set: Vec<Cell> },
    Insert { values: Vec<Cell> },
    Delete { pk: Vec<Cell> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableRef {
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Update,
    Insert,
    Delete,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub kind: Kind,
    pub sql: String,
    pub params: Vec<Option<String>>,
    pub preview: String,
}

#[derive(Debug, Serialize)]
pub struct ApplyResult {
    pub affected: u64,
}

pub fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Letterale tra apici per l'anteprima (apici raddoppiati); valori lunghi accorciati.
pub(super) fn literal(v: &Option<String>) -> String {
    match v {
        None => "NULL".into(),
        Some(s) => {
            let (t, cut) = clip(s, PREVIEW_VALUE_MAX);
            format!("'{}{}'", t.replace('\'', "''"), if cut { "…" } else { "" })
        }
    }
}

pub(super) fn clip(s: &str, max: usize) -> (&str, bool) {
    if s.chars().count() <= max {
        return (s, false);
    }
    let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
    (&s[..end], true)
}

fn table_sql(t: &TableRef) -> String {
    format!("{}.{}", quote_ident(&t.schema), quote_ident(&t.table))
}

struct Builder<'a> {
    columns: &'a [PgColumn],
    params: Vec<Option<String>>,
}

impl<'a> Builder<'a> {
    fn column(&self, name: &str) -> Result<&'a PgColumn, String> {
        self.columns.iter().find(|c| c.name == name).ok_or_else(|| format!("Colonna sconosciuta: {}", clip(name, 60).0))
    }

    /// `"col" = $n::tipo` (o la parte letterale per l'anteprima).
    fn assign(&mut self, name: &str, v: &Option<String>, preview: &mut Vec<String>, sql: &mut Vec<String>, op: &str) -> Result<(), String> {
        let c = self.column(name)?;
        if let Some(s) = v
            && s.contains('\0')
        {
            return Err(format!("Colonna {}: il valore contiene un carattere NUL, non ammesso", c.name));
        }
        self.params.push(v.clone());
        let n = self.params.len();
        sql.push(format!("{} {op} ${n}::{}", quote_ident(&c.name), c.cast_type));
        preview.push(format!("{} {op} {}", quote_ident(&c.name), literal(v)));
        Ok(())
    }
}

pub(super) fn check_no_dups(cells: &[Cell], what: &str) -> Result<(), String> {
    for (i, (n, _)) in cells.iter().enumerate() {
        if cells[..i].iter().any(|(m, _)| m == n) {
            return Err(format!("{what}: colonna ripetuta ({})", clip(n, 60).0));
        }
    }
    Ok(())
}

/// La chiave deve comprendere tutte (e sole) le colonne della chiave primaria, mai NULL.
fn check_pk(columns: &[PgColumn], pk: &[Cell]) -> Result<(), String> {
    check_no_dups(pk, "Chiave")?;
    for (n, v) in pk {
        let c = columns.iter().find(|c| c.name == *n).ok_or_else(|| format!("Colonna sconosciuta: {}", clip(n, 60).0))?;
        if !c.is_pk {
            return Err(format!("La colonna {} non fa parte della chiave primaria", c.name));
        }
        if v.is_none() {
            return Err(format!("La chiave primaria ({}) non può essere NULL", c.name));
        }
    }
    for c in columns.iter().filter(|c| c.is_pk) {
        if !pk.iter().any(|(n, _)| *n == c.name) {
            return Err(format!("Manca la colonna {} della chiave primaria", c.name));
        }
    }
    Ok(())
}

pub fn plan_edits(table: &TableRef, columns: &[PgColumn], edits: &[RowEdit]) -> Result<Vec<Statement>, String> {
    if edits.is_empty() {
        return Err("Nessuna modifica da applicare".into());
    }
    if edits.len() > MAX_EDITS {
        return Err(format!("Troppe modifiche in una volta (al massimo {MAX_EDITS})"));
    }
    if columns.is_empty() {
        return Err("Tabella non trovata".into());
    }
    if !columns.iter().any(|c| c.is_pk) {
        return Err("La tabella non ha una chiave primaria: è in sola lettura".into());
    }
    let t = table_sql(table);
    let mut out = Vec::with_capacity(edits.len());
    for e in edits {
        let mut b = Builder { columns, params: Vec::new() };
        let (mut sql_sets, mut pre_sets, mut sql_where, mut pre_where) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let st = match e {
            RowEdit::Update { pk, set } => {
                if set.is_empty() {
                    return Err("Modifica senza colonne da cambiare".into());
                }
                check_no_dups(set, "Modifica")?;
                check_pk(columns, pk)?;
                for (n, v) in set {
                    b.assign(n, v, &mut pre_sets, &mut sql_sets, "=")?;
                }
                for (n, v) in pk {
                    b.assign(n, v, &mut pre_where, &mut sql_where, "=")?;
                }
                Statement {
                    kind: Kind::Update,
                    sql: format!("UPDATE {t} SET {} WHERE {}", sql_sets.join(", "), sql_where.join(" AND ")),
                    preview: format!("UPDATE {t} SET {} WHERE {}", pre_sets.join(", "), pre_where.join(" AND ")),
                    params: b.params,
                }
            }
            RowEdit::Delete { pk } => {
                check_pk(columns, pk)?;
                for (n, v) in pk {
                    b.assign(n, v, &mut pre_where, &mut sql_where, "=")?;
                }
                Statement {
                    kind: Kind::Delete,
                    sql: format!("DELETE FROM {t} WHERE {}", sql_where.join(" AND ")),
                    preview: format!("DELETE FROM {t} WHERE {}", pre_where.join(" AND ")),
                    params: b.params,
                }
            }
            RowEdit::Insert { values } => {
                check_no_dups(values, "Inserimento")?;
                if values.is_empty() {
                    Statement { kind: Kind::Insert, sql: format!("INSERT INTO {t} DEFAULT VALUES"), preview: format!("INSERT INTO {t} DEFAULT VALUES"), params: Vec::new() }
                } else {
                    let (mut names, mut casts, mut lits) = (Vec::new(), Vec::new(), Vec::new());
                    for (n, v) in values {
                        let c = b.column(n)?;
                        let (mut p, mut s) = (Vec::new(), Vec::new());
                        b.assign(n, v, &mut p, &mut s, "=")?;
                        names.push(quote_ident(&c.name));
                        casts.push(format!("${}::{}", b.params.len(), c.cast_type));
                        lits.push(literal(v));
                    }
                    Statement {
                        kind: Kind::Insert,
                        sql: format!("INSERT INTO {t} ({}) VALUES ({})", names.join(", "), casts.join(", ")),
                        preview: format!("INSERT INTO {t} ({}) VALUES ({})", names.join(", "), lits.join(", ")),
                        params: b.params,
                    }
                }
            }
        };
        out.push(st);
    }
    Ok(out)
}

pub fn previews(stmts: &[Statement]) -> Vec<String> {
    stmts.iter().map(|s| s.preview.clone()).collect()
}

/// Riepilogo per la conferma nativa: conteggi per tipo e le prime anteprime.
pub fn confirm_message(stmts: &[Statement]) -> String {
    let count = |k: Kind| stmts.iter().filter(|s| s.kind == k).count();
    let mut m = format!(
        "{} modifiche in una sola transazione: {} aggiornamenti, {} inserimenti, {} cancellazioni.\n",
        stmts.len(),
        count(Kind::Update),
        count(Kind::Insert),
        count(Kind::Delete)
    );
    for s in stmts.iter().take(CONFIRM_LISTED) {
        m.push_str(&format!("\n• {}", clip(&s.preview, 300).0));
    }
    if stmts.len() > CONFIRM_LISTED {
        m.push_str(&format!("\n… e altre {}", stmts.len() - CONFIRM_LISTED));
    }
    m
}

/// Colonne della tabella (dal catalogo, mai dalla WebView) e piano delle istruzioni.
pub async fn plan(sessions: &Sessions, id: &str, schema: &str, table: &str, edits: &[RowEdit]) -> Result<Vec<Statement>, String> {
    let cols = super::pg_query::columns(sessions, id, schema, table).await?;
    plan_edits(&TableRef { schema: schema.into(), table: table.into() }, &cols, edits)
}

fn db_error(e: &tokio_postgres::Error, stmt: &Statement) -> String {
    let msg = qerr(e);
    format!("Errore del database: {} — istruzione: {}", clip(&msg, 500).0, clip(&stmt.preview, 300).0)
}

/// `BEGIN` / istruzioni / `COMMIT`; ogni `UPDATE`/`DELETE`/`INSERT` deve toccare esattamente una riga, altrimenti
/// `ROLLBACK`. Prende la connessione in esclusiva (chiude un eventuale risultato aperto, che tiene una transazione);
/// con una transazione aperta dall'utente rifiuta (`USER_TX`): il suo `COMMIT`/`ROLLBACK` concluderebbe anche quella.
pub async fn execute(queries: &Queries, sessions: &Sessions, id: &str, stmts: &[Statement]) -> Result<ApplyResult, String> {
    let (_guard, h) = queries.exclusive(sessions, id).await?;
    let c = &h.client;
    c.simple_query("BEGIN").await.map_err(|e| qerr(&e))?;
    let mut total = 0u64;
    for s in stmts {
        let step = async {
            let types = vec![Type::TEXT; s.params.len()];
            let prepared = c.prepare_typed(&s.sql, &types).await.map_err(|e| db_error(&e, s))?;
            let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = s.params.iter().map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync)).collect();
            let n = c.execute(&prepared, &refs).await.map_err(|e| db_error(&e, s))?;
            if n != 1 {
                let what = if s.kind == Kind::Insert { "non ha inserito una riga" } else { "non corrisponde più a una sola riga (la riga è cambiata o cancellata?)" };
                return Err(format!("Nessuna modifica applicata: l'istruzione {what}: {}", clip(&s.preview, 300).0));
            }
            Ok::<u64, String>(n)
        };
        match step.await {
            Ok(n) => total += n,
            Err(e) => {
                let _ = c.simple_query("ROLLBACK").await;
                return Err(e);
            }
        }
    }
    if let Err(e) = c.simple_query("COMMIT").await {
        let _ = c.simple_query("ROLLBACK").await;
        return Err(qerr(&e));
    }
    Ok(ApplyResult { affected: total })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str, pk: bool) -> PgColumn {
        PgColumn { name: name.into(), type_name: ty.into(), cast_type: ty.into(), nullable: !pk, is_pk: pk }
    }
    fn cell(n: &str, v: Option<&str>) -> Cell {
        (n.into(), v.map(str::to_string))
    }
    fn tref(s: &str, t: &str) -> TableRef {
        TableRef { schema: s.into(), table: t.into() }
    }

    #[test]
    fn quotes_identifiers() {
        assert_eq!(quote_ident("a"), "\"a\"");
        assert_eq!(quote_ident("A \"b\" c"), "\"A \"\"b\"\" c\"");
        assert_eq!(quote_ident("select"), "\"select\"");
    }

    #[test]
    fn update_with_odd_names_types_and_null() {
        let cols = [col("Id Col", "integer", true), col("order", "numeric(10,2)", false), col("we\"ird", "timestamp with time zone", false), col("tags", "integer[]", false)];
        let edits = [RowEdit::Update {
            pk: vec![cell("Id Col", Some("7"))],
            set: vec![cell("order", Some("12.50")), cell("we\"ird", None), cell("tags", Some("{1,2}"))],
        }];
        let s = plan_edits(&tref("My \"S\"", "Ta ble"), &cols, &edits).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(
            s[0].sql,
            "UPDATE \"My \"\"S\"\"\".\"Ta ble\" SET \"order\" = $1::numeric(10,2), \"we\"\"ird\" = $2::timestamp with time zone, \"tags\" = $3::integer[] WHERE \"Id Col\" = $4::integer"
        );
        assert_eq!(s[0].params, [Some("12.50".into()), None, Some("{1,2}".into()), Some("7".into())]);
        assert!(s[0].preview.contains("\"we\"\"ird\" = NULL") && s[0].preview.contains("\"order\" = '12.50'"), "{}", s[0].preview);
    }

    #[test]
    fn parameters_cast_to_the_type_without_modifiers() {
        // il tipo mostrato ha la lunghezza, il cast no: l'assegnazione alla colonna la fa rispettare (niente troncamenti)
        let cols = [
            col("id", "integer", true),
            PgColumn { name: "v".into(), type_name: "character varying(10)".into(), cast_type: "character varying".into(), nullable: true, is_pk: false },
        ];
        let s = plan_edits(&tref("public", "t"), &cols, &[RowEdit::Update { pk: vec![cell("id", Some("1"))], set: vec![cell("v", Some("x"))] }]).unwrap();
        assert_eq!(s[0].sql, "UPDATE \"public\".\"t\" SET \"v\" = $1::character varying WHERE \"id\" = $2::integer");
        let s = plan_edits(&tref("public", "t"), &cols, &[RowEdit::Insert { values: vec![cell("v", Some("x"))] }]).unwrap();
        assert_eq!(s[0].sql, "INSERT INTO \"public\".\"t\" (\"v\") VALUES ($1::character varying)");
    }

    #[test]
    fn null_differs_from_empty_string() {
        let cols = [col("id", "integer", true), col("t", "text", false)];
        let a = plan_edits(&tref("public", "x"), &cols, &[RowEdit::Update { pk: vec![cell("id", Some("1"))], set: vec![cell("t", None)] }]).unwrap();
        let b = plan_edits(&tref("public", "x"), &cols, &[RowEdit::Update { pk: vec![cell("id", Some("1"))], set: vec![cell("t", Some(""))] }]).unwrap();
        assert_eq!(a[0].params[0], None);
        assert_eq!(b[0].params[0], Some(String::new()));
        assert!(a[0].preview.contains("= NULL") && b[0].preview.contains("= ''"));
    }

    #[test]
    fn insert_only_given_columns_and_defaults() {
        let cols = [col("id", "integer", true), col("j", "jsonb", false), col("b", "bytea", false)];
        let s = plan_edits(&tref("public", "t"), &cols, &[RowEdit::Insert { values: vec![cell("j", Some("{\"a\": 1}"))] }, RowEdit::Insert { values: vec![] }]).unwrap();
        assert_eq!(s[0].sql, "INSERT INTO \"public\".\"t\" (\"j\") VALUES ($1::jsonb)");
        assert_eq!(s[0].params, [Some("{\"a\": 1}".into())]);
        assert_eq!(s[1].sql, "INSERT INTO \"public\".\"t\" DEFAULT VALUES");
        assert!(s[1].params.is_empty());
    }

    #[test]
    fn delete_composite_pk() {
        let cols = [col("a", "integer", true), col("b", "text", true), col("c", "text", false)];
        let s = plan_edits(&tref("public", "t"), &cols, &[RowEdit::Delete { pk: vec![cell("b", Some("x'y")), cell("a", Some("1"))] }]).unwrap();
        assert_eq!(s[0].sql, "DELETE FROM \"public\".\"t\" WHERE \"b\" = $1::text AND \"a\" = $2::integer");
        assert_eq!(s[0].preview, "DELETE FROM \"public\".\"t\" WHERE \"b\" = 'x''y' AND \"a\" = '1'");
        // chiave incompleta, con colonna estranea, o con NULL
        let bad = |pk: Vec<Cell>| plan_edits(&tref("public", "t"), &cols, &[RowEdit::Delete { pk }]).unwrap_err();
        assert!(bad(vec![cell("a", Some("1"))]).contains("Manca"));
        assert!(bad(vec![cell("a", Some("1")), cell("b", Some("x")), cell("c", Some("y"))]).contains("non fa parte"));
        assert!(bad(vec![cell("a", None), cell("b", Some("x"))]).contains("NULL"));
        assert!(bad(vec![cell("a", Some("1")), cell("a", Some("2")), cell("b", Some("x"))]).contains("ripetuta"));
    }

    #[test]
    fn rejects_no_pk_unknown_columns_and_empty() {
        let nopk = [col("a", "integer", false)];
        let e = plan_edits(&tref("public", "t"), &nopk, &[RowEdit::Insert { values: vec![cell("a", Some("1"))] }]).unwrap_err();
        assert!(e.contains("chiave primaria"));
        let cols = [col("id", "integer", true), col("t", "text", false)];
        let e = plan_edits(&tref("public", "t"), &cols, &[RowEdit::Update { pk: vec![cell("id", Some("1"))], set: vec![cell("nope", Some("x"))] }]).unwrap_err();
        assert!(e.contains("sconosciuta"));
        let e = plan_edits(&tref("public", "t"), &cols, &[RowEdit::Insert { values: vec![cell("id\" ; DROP", Some("x"))] }]).unwrap_err();
        assert!(e.contains("sconosciuta"));
        assert!(plan_edits(&tref("public", "t"), &cols, &[]).is_err());
        assert!(plan_edits(&tref("public", "t"), &[], &[RowEdit::Insert { values: vec![] }]).is_err());
        assert!(plan_edits(&tref("public", "t"), &cols, &[RowEdit::Update { pk: vec![cell("id", Some("1"))], set: vec![] }]).is_err());
        assert!(plan_edits(&tref("public", "t"), &cols, &[RowEdit::Insert { values: vec![cell("t", Some("a\0b"))] }]).is_err());
    }

    #[test]
    fn preview_truncates_long_values_and_confirm_lists_ten() {
        let cols = [col("id", "integer", true), col("t", "text", false)];
        let long = "é".repeat(500);
        let edits: Vec<RowEdit> = (0..12).map(|i| RowEdit::Update { pk: vec![cell("id", Some(&i.to_string()))], set: vec![cell("t", Some(&long))] }).collect();
        let s = plan_edits(&tref("public", "t"), &cols, &edits).unwrap();
        // i parametri restano interi, l'anteprima è accorciata
        assert_eq!(s[0].params[0].as_deref(), Some(long.as_str()));
        assert!(s[0].preview.chars().count() < 400 && s[0].preview.contains('…'));
        let m = confirm_message(&s);
        assert!(m.contains("12 modifiche") && m.contains("12 aggiornamenti, 0 inserimenti, 0 cancellazioni"));
        assert_eq!(m.matches("\n• ").count(), 10);
        assert!(m.contains("altre 2"));
        assert_eq!(previews(&s).len(), 12);
    }

    // ---- integrazione: cluster temporaneo ----

    use super::super::pg::PgParams;
    use super::super::testutil::Cluster;
    use super::super::DbTls;
    use crate::ssh::conn::{Connections, SshPaths};
    use std::sync::Arc;

    #[test]
    #[ignore = "avvia un cluster Postgres temporaneo (initdb e pg_ctl nel PATH)"]
    fn live_cluster_apply_edits() {
        let Some(cluster) = Cluster::start() else {
            eprintln!("initdb/pg_ctl non trovati: saltato");
            return;
        };
        let p = PgParams { host: "127.0.0.1".into(), port: cluster.port, user: Some("codeart".into()), database: Some("postgres".into()), tls: DbTls::Disable, accept_invalid_certs: false };
        tauri::async_runtime::block_on(async {
            let conns = Arc::new(Connections::new(SshPaths::new(std::env::temp_dir()), Arc::new(|_| {})));
            let sessions = Arc::new(Sessions::new(conns, Arc::new(|_| {})));
            sessions.connect("c1", p, Some("test-pass".into()), None).await.unwrap();
            let q = Arc::new(Queries::new());
            q.query(
                &sessions,
                "c1",
                r#"SET timezone = 'UTC';
                   CREATE TABLE "Mix ""T""" (a int, "b c" text, "n" numeric(10,2), ts timestamptz, j jsonb, bin bytea, arr int[], PRIMARY KEY (a, "b c"));
                   INSERT INTO "Mix ""T""" VALUES (1, 'x', 1.10, '2024-01-01 00:00:00+00', '{"k": 1}', '\xaa', '{1}'), (2, 'y', 2.20, NULL, NULL, NULL, NULL), (3, 'z', 3.30, NULL, NULL, NULL, NULL);
                   CREATE TABLE nopk (a int)"#,
            )
            .await
            .unwrap();
            let schema = "public";
            let table = "Mix \"T\"";
            let sel = |where_: &str| format!("SELECT a, \"b c\", n::text, ts::text, j::text, encode(bin, 'hex'), arr::text FROM \"Mix \"\"T\"\"\" {where_} ORDER BY a, \"b c\"");
            let s = |v: &str| Some(v.to_string());

            // un colpo solo: aggiorna, inserisce, cancella
            let edits = vec![
                RowEdit::Update { pk: vec![cell("a", Some("1")), cell("b c", Some("x"))], set: vec![cell("n", Some("99.95")), cell("ts", Some("2025-06-07 08:09:10+00")), cell("j", Some("{\"z\": [1, 2]}")), cell("bin", Some("\\xdeadbeef")), cell("arr", Some("{4,5,6}"))] },
                RowEdit::Update { pk: vec![cell("a", Some("2")), cell("b c", Some("y"))], set: vec![cell("n", None)] },
                RowEdit::Insert { values: vec![cell("a", Some("10")), cell("b c", Some("O'Hara \"q\"")), cell("arr", Some("{}"))] },
                RowEdit::Delete { pk: vec![cell("a", Some("3")), cell("b c", Some("z"))] },
            ];
            let stmts = plan(&sessions, "c1", schema, table, &edits).await.unwrap();
            assert_eq!(stmts.len(), 4);
            let r = execute(&q, &sessions, "c1", &stmts).await.unwrap();
            assert_eq!(r.affected, 4);
            let rows = q.query(&sessions, "c1", &sel("")).await.unwrap().rows;
            assert_eq!(rows.len(), 3);
            assert_eq!(rows[0], [s("1"), s("x"), s("99.95"), s("2025-06-07 08:09:10+00"), s("{\"z\": [1, 2]}"), s("deadbeef"), s("{4,5,6}")]);
            assert_eq!(rows[1], [s("2"), s("y"), None, None, None, None, None]);
            assert_eq!(rows[2][..2], [s("10"), s("O'Hara \"q\"")]);
            assert_eq!(rows[2][6], s("{}"));
            // la sessione è libera (transazione conclusa)
            q.query(&sessions, "c1", "SELECT 1").await.unwrap();

            // riga cambiata nel frattempo: lo UPDATE tocca 0 righe → ROLLBACK, nulla applicato (neanche il primo)
            let edits = vec![
                RowEdit::Update { pk: vec![cell("a", Some("1")), cell("b c", Some("x"))], set: vec![cell("n", Some("1.00"))] },
                RowEdit::Update { pk: vec![cell("a", Some("1")), cell("b c", Some("cambiata"))], set: vec![cell("n", Some("2.00"))] },
            ];
            let stmts = plan(&sessions, "c1", schema, table, &edits).await.unwrap();
            let e = execute(&q, &sessions, "c1", &stmts).await.unwrap_err();
            assert!(e.contains("Nessuna modifica applicata") && e.contains("cambiata"), "{e}");
            let rows = q.query(&sessions, "c1", &sel("WHERE a = 1")).await.unwrap().rows;
            assert_eq!(rows[0][2], s("99.95"));

            // errore del server (valore non valido): messaggio italiano col testo del server, ROLLBACK
            let edits = vec![
                RowEdit::Delete { pk: vec![cell("a", Some("2")), cell("b c", Some("y"))] },
                RowEdit::Update { pk: vec![cell("a", Some("1")), cell("b c", Some("x"))], set: vec![cell("n", Some("non un numero"))] },
            ];
            let stmts = plan(&sessions, "c1", schema, table, &edits).await.unwrap();
            let e = execute(&q, &sessions, "c1", &stmts).await.unwrap_err();
            assert!(e.starts_with("Errore del database:") && e.contains("non un numero"), "{e}");
            assert_eq!(q.query(&sessions, "c1", &sel("WHERE a = 2")).await.unwrap().rows.len(), 1);

            // un risultato con cursore aperto non blocca l'applicazione
            let big = q.query(&sessions, "c1", "SELECT g FROM generate_series(1, 700) g").await.unwrap();
            assert!(big.result_id.is_some());
            let stmts = plan(&sessions, "c1", schema, table, &[RowEdit::Delete { pk: vec![cell("a", Some("10")), cell("b c", Some("O'Hara \"q\""))] }]).await.unwrap();
            assert_eq!(execute(&q, &sessions, "c1", &stmts).await.unwrap().affected, 1);

            // lunghezza e precisione della colonna rispettate: un valore troppo lungo è un errore, non un troncamento
            q.query(&sessions, "c1", "CREATE TABLE lim (id int PRIMARY KEY, v varchar(10), c char(3), n numeric(5,2), b bit(4)); INSERT INTO lim VALUES (1, 'corto', 'x', 1.00, B'0000')").await.unwrap();
            let edits = vec![
                RowEdit::Update { pk: vec![cell("id", Some("1"))], set: vec![cell("c", Some("ab"))] },
                RowEdit::Update { pk: vec![cell("id", Some("1"))], set: vec![cell("v", Some("quindici_carat"))] },
            ];
            let stmts = plan(&sessions, "c1", "public", "lim", &edits).await.unwrap();
            assert!(stmts[1].sql.contains("$1::character varying "), "{}", stmts[1].sql);
            let e = execute(&q, &sessions, "c1", &stmts).await.unwrap_err();
            assert!(e.contains("too long") || e.contains("troppo lungo"), "{e}");
            const LIM: &str = "SELECT v, c, n::text, b::text FROM lim";
            assert_eq!(q.query(&sessions, "c1", LIM).await.unwrap().rows, [vec![s("corto"), s("x  "), s("1.00"), s("0000")]]);
            let bad = |col: &str, v: &str| vec![RowEdit::Update { pk: vec![cell("id", Some("1"))], set: vec![cell(col, Some(v))] }];
            for (col, v) in [("c", "abcd"), ("n", "123456"), ("b", "10101")] {
                let stmts = plan(&sessions, "c1", "public", "lim", &bad(col, v)).await.unwrap();
                assert!(execute(&q, &sessions, "c1", &stmts).await.is_err(), "{col} = {v}");
            }
            // valori che entrano: char(3) non diventa char(1), bit(4) non diventa bit(1)
            let ok = vec![RowEdit::Update { pk: vec![cell("id", Some("1"))], set: vec![cell("v", Some("dieci_cara")), cell("c", Some("ab")), cell("n", Some("123.45")), cell("b", Some("1011"))] }];
            let stmts = plan(&sessions, "c1", "public", "lim", &ok).await.unwrap();
            execute(&q, &sessions, "c1", &stmts).await.unwrap();
            assert_eq!(q.query(&sessions, "c1", LIM).await.unwrap().rows, [vec![s("dieci_cara"), s("ab "), s("123.45"), s("1011")]]);

            // tabella senza chiave primaria, tabella inesistente, colonna sconosciuta
            let e = plan(&sessions, "c1", "public", "nopk", &[RowEdit::Insert { values: vec![cell("a", Some("1"))] }]).await.unwrap_err();
            assert!(e.contains("chiave primaria"), "{e}");
            assert!(plan(&sessions, "c1", "public", "non_esiste", &[RowEdit::Insert { values: vec![] }]).await.is_err());
            assert!(plan(&sessions, "c1", schema, table, &[RowEdit::Insert { values: vec![cell("zz", Some("1"))] }]).await.is_err());
            sessions.disconnect("c1");
        });
    }
}
