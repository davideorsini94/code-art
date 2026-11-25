//! Modifiche in griglia per Oracle: stesso disegno di `pg_edit` (piano puro, anteprima, una riga esatta per istruzione,
//! `ROLLBACK` a ogni errore), con parametri posizionali `:1…:n` tutti testuali e una conversione esplicita per tipo
//! di colonna scelta dal core sul `DATA_TYPE` del catalogo (mai dalla WebView): `TO_NUMBER`, `TO_DATE`, `TO_TIMESTAMP`,
//! `TO_TIMESTAMP_TZ` con i formati delle NLS della sessione, testo diretto per CHAR/VARCHAR2/NCHAR/NVARCHAR2. CLOB,
//! BLOB, RAW, LONG e gli altri tipi non si modificano in griglia. I nomi si citano esattamente come nel catalogo.
//!
//! In Oracle la stringa vuota è NULL: `Some("")` si tratta come NULL (parametro nullo, `NULL` nell'anteprima); una
//! chiave primaria non può essere NULL, quindi è rifiutata.
use super::ora::{Backend, OraSessions, Request};
use super::ora_query::{self, OpErr, OraColumn};
use super::pg_edit::{check_no_dups, clip, literal, quote_ident, ApplyResult, Cell, Kind, RowEdit, Statement, TableRef};
use super::pg_edit::MAX_EDITS;

/// Il formato del parametro per la colonna: l'espressione che lo converte, con `:n`. `None` = tipo non modificabile.
pub fn bind_expr(data_type: &str, n: usize) -> Option<String> {
    let t = data_type.trim().to_uppercase();
    match t.as_str() {
        "NUMBER" | "FLOAT" | "BINARY_FLOAT" | "BINARY_DOUBLE" => Some(format!("TO_NUMBER(:{n})")),
        "DATE" => Some(format!("TO_DATE(:{n},'YYYY-MM-DD HH24:MI:SS')")),
        "CHAR" | "VARCHAR2" | "NCHAR" | "NVARCHAR2" => Some(format!(":{n}")),
        // WITH LOCAL TIME ZONE: il testo mostrato è nel fuso della sessione e non si riscrive identico (finché non è
        // verificato con un server vero resta in sola lettura)
        _ if t.starts_with("TIMESTAMP") && t.contains("WITH LOCAL TIME ZONE") => None,
        _ if t.starts_with("TIMESTAMP") => {
            if t.contains("WITH TIME ZONE") {
                Some(format!("TO_TIMESTAMP_TZ(:{n},'YYYY-MM-DD HH24:MI:SS.FF TZH:TZM')"))
            } else {
                Some(format!("TO_TIMESTAMP(:{n},'YYYY-MM-DD HH24:MI:SS.FF')"))
            }
        }
        _ => None,
    }
}

fn table_sql(t: &TableRef) -> String {
    format!("{}.{}", quote_ident(&t.schema), quote_ident(&t.table))
}

/// La stringa vuota è NULL.
fn norm(v: &Option<String>) -> Option<String> {
    v.clone().filter(|s| !s.is_empty())
}

struct Builder<'a> {
    columns: &'a [OraColumn],
    params: Vec<Option<String>>,
}

impl<'a> Builder<'a> {
    fn column(&self, name: &str) -> Result<&'a OraColumn, String> {
        self.columns.iter().find(|c| c.name == name).ok_or_else(|| format!("Colonna sconosciuta: {}", clip(name, 60).0))
    }

    /// Aggiunge il parametro di `name` e dà (colonna citata, espressione col parametro, letterale dell'anteprima).
    fn bind(&mut self, name: &str, v: &Option<String>) -> Result<(String, String, String), String> {
        let c = self.column(name)?;
        let v = norm(v);
        if let Some(s) = &v
            && s.contains('\0')
        {
            return Err(format!("Colonna {}: il valore contiene un carattere NUL, non ammesso", c.name));
        }
        let n = self.params.len() + 1;
        let expr = bind_expr(&c.data_type, n).ok_or_else(|| format!("Colonna {}: il tipo {} non si modifica in griglia", c.name, c.data_type))?;
        self.params.push(v.clone());
        Ok((quote_ident(&c.name), expr, literal(&v)))
    }
}

/// La chiave deve comprendere tutte (e sole) le colonne della chiave primaria, mai NULL (e `''` è NULL).
fn check_pk(columns: &[OraColumn], pk: &[Cell]) -> Result<(), String> {
    check_no_dups(pk, "Chiave")?;
    for (n, v) in pk {
        let c = columns.iter().find(|c| c.name == *n).ok_or_else(|| format!("Colonna sconosciuta: {}", clip(n, 60).0))?;
        if !c.is_pk {
            return Err(format!("La colonna {} non fa parte della chiave primaria", c.name));
        }
        if norm(v).is_none() {
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

pub fn plan_edits(table: &TableRef, columns: &[OraColumn], edits: &[RowEdit]) -> Result<Vec<Statement>, String> {
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
        let st = match e {
            RowEdit::Update { pk, set } => {
                if set.is_empty() {
                    return Err("Modifica senza colonne da cambiare".into());
                }
                check_no_dups(set, "Modifica")?;
                check_pk(columns, pk)?;
                let (mut sql_sets, mut pre_sets, mut sql_where, mut pre_where) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
                for (n, v) in set {
                    let (col, expr, lit) = b.bind(n, v)?;
                    sql_sets.push(format!("{col} = {expr}"));
                    pre_sets.push(format!("{col} = {lit}"));
                }
                for (n, v) in pk {
                    let (col, expr, lit) = b.bind(n, v)?;
                    sql_where.push(format!("{col} = {expr}"));
                    pre_where.push(format!("{col} = {lit}"));
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
                let (mut sql_where, mut pre_where) = (Vec::new(), Vec::new());
                for (n, v) in pk {
                    let (col, expr, lit) = b.bind(n, v)?;
                    sql_where.push(format!("{col} = {expr}"));
                    pre_where.push(format!("{col} = {lit}"));
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
                    // `DEFAULT VALUES` esiste solo da Oracle 23ai: `VALUES (DEFAULT)` sulla prima colonna vale ovunque
                    let first = quote_ident(&columns[0].name);
                    let text = format!("INSERT INTO {t} ({first}) VALUES (DEFAULT)");
                    Statement { kind: Kind::Insert, sql: text.clone(), preview: text, params: Vec::new() }
                } else {
                    let (mut names, mut exprs, mut lits) = (Vec::new(), Vec::new(), Vec::new());
                    for (n, v) in values {
                        let (col, expr, lit) = b.bind(n, v)?;
                        names.push(col);
                        exprs.push(expr);
                        lits.push(lit);
                    }
                    Statement {
                        kind: Kind::Insert,
                        sql: format!("INSERT INTO {t} ({}) VALUES ({})", names.join(", "), exprs.join(", ")),
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

/// Colonne della tabella (dal catalogo, mai dalla WebView) e piano delle istruzioni.
pub async fn plan(sessions: &OraSessions, id: &str, owner: &str, table: &str, edits: &[RowEdit]) -> Result<Vec<Statement>, String> {
    let cols = ora_query::columns(sessions, id, owner, table).await?;
    plan_edits(&TableRef { schema: owner.into(), table: table.into() }, &cols, edits)
}

/// Le istruzioni in sequenza nella transazione del thread: ognuna deve toccare esattamente una riga, altrimenti
/// `ROLLBACK`; alla fine `COMMIT`. Un errore di connessione caduta resta tale (il thread esce).
pub fn run_apply(b: &mut dyn Backend, stmts: &[Statement]) -> Result<u64, OpErr> {
    let mut total = 0u64;
    for s in stmts {
        match b.exec(&s.sql, &s.params) {
            Ok(1) => total += 1,
            Ok(_) => {
                let _ = b.rollback();
                let what = if s.kind == Kind::Insert { "non ha inserito una riga" } else { "non corrisponde più a una sola riga (la riga è cambiata o cancellata?)" };
                return Err(OpErr::msg(format!("Nessuna modifica applicata: l'istruzione {what}: {}", clip(&s.preview, 300).0)));
            }
            Err(e) => {
                let _ = b.rollback();
                if e.is_connection_lost() {
                    return Err(OpErr::Db(e));
                }
                let err = OpErr::Db(e);
                return Err(OpErr::msg(format!("Errore del database: {} — istruzione: {}", clip(&err.text(), 500).0, clip(&s.preview, 300).0)));
            }
        }
    }
    if let Err(e) = b.commit() {
        let _ = b.rollback();
        return Err(OpErr::Db(e));
    }
    Ok(total)
}

pub async fn execute(sessions: &OraSessions, id: &str, stmts: &[Statement]) -> Result<ApplyResult, String> {
    let actor = sessions.actor(id)?;
    let stmts = stmts.to_vec();
    let affected = actor.request(|reply| Request::Apply { stmts, reply }).await?;
    Ok(ApplyResult { affected })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str, pk: bool) -> OraColumn {
        OraColumn { name: name.into(), type_name: ty.into(), data_type: ty.into(), nullable: !pk, is_pk: pk, editable: bind_expr(ty, 1).is_some() }
    }
    fn cell(n: &str, v: Option<&str>) -> Cell {
        (n.into(), v.map(str::to_string))
    }
    fn tref(s: &str, t: &str) -> TableRef {
        TableRef { schema: s.into(), table: t.into() }
    }

    #[test]
    fn every_column_type_has_its_conversion() {
        assert_eq!(bind_expr("NUMBER", 1).unwrap(), "TO_NUMBER(:1)");
        assert_eq!(bind_expr("FLOAT", 2).unwrap(), "TO_NUMBER(:2)");
        assert_eq!(bind_expr("BINARY_DOUBLE", 3).unwrap(), "TO_NUMBER(:3)");
        assert_eq!(bind_expr("BINARY_FLOAT", 3).unwrap(), "TO_NUMBER(:3)");
        assert_eq!(bind_expr("DATE", 1).unwrap(), "TO_DATE(:1,'YYYY-MM-DD HH24:MI:SS')");
        assert_eq!(bind_expr("TIMESTAMP(6)", 1).unwrap(), "TO_TIMESTAMP(:1,'YYYY-MM-DD HH24:MI:SS.FF')");
        assert_eq!(bind_expr("TIMESTAMP(3) WITH TIME ZONE", 1).unwrap(), "TO_TIMESTAMP_TZ(:1,'YYYY-MM-DD HH24:MI:SS.FF TZH:TZM')");
        for t in ["CHAR", "VARCHAR2", "NCHAR", "NVARCHAR2"] {
            assert_eq!(bind_expr(t, 4).unwrap(), ":4");
        }
        for t in ["CLOB", "NCLOB", "BLOB", "RAW", "LONG", "LONG RAW", "XMLTYPE", "ROWID", "INTERVAL DAY(2) TO SECOND(6)", "BFILE", "SDO_GEOMETRY", "TIMESTAMP(9) WITH LOCAL TIME ZONE", "JSON", "UROWID"] {
            assert!(bind_expr(t, 1).is_none(), "{t}");
        }
    }

    #[test]
    fn update_with_odd_names_types_and_null() {
        let cols = [
            col("Id Col", "NUMBER", true),
            col("price$#", "NUMBER", false),
            col("when", "DATE", false),
            col("ts_tz", "TIMESTAMP(6) WITH TIME ZONE", false),
            col("name", "VARCHAR2", false),
            col("lower", "TIMESTAMP(6)", false),
        ];
        let edits = [RowEdit::Update {
            pk: vec![cell("Id Col", Some("7"))],
            set: vec![
                cell("price$#", Some("12.50")),
                cell("when", Some("2024-03-09 07:05:01")),
                cell("ts_tz", Some("2024-03-09 07:05:01.000005 +02:00")),
                cell("name", None),
                cell("lower", Some("2024-03-09 07:05:01.5")),
            ],
        }];
        let s = plan_edits(&tref("My \"S\"", "lower tab"), &cols, &edits).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(
            s[0].sql,
            "UPDATE \"My \"\"S\"\"\".\"lower tab\" SET \"price$#\" = TO_NUMBER(:1), \"when\" = TO_DATE(:2,'YYYY-MM-DD HH24:MI:SS'), \
             \"ts_tz\" = TO_TIMESTAMP_TZ(:3,'YYYY-MM-DD HH24:MI:SS.FF TZH:TZM'), \"name\" = :4, \"lower\" = TO_TIMESTAMP(:5,'YYYY-MM-DD HH24:MI:SS.FF') \
             WHERE \"Id Col\" = TO_NUMBER(:6)"
        );
        assert_eq!(s[0].params, [Some("12.50".into()), Some("2024-03-09 07:05:01".into()), Some("2024-03-09 07:05:01.000005 +02:00".into()), None, Some("2024-03-09 07:05:01.5".into()), Some("7".into())]);
        assert!(s[0].preview.contains("\"name\" = NULL") && s[0].preview.contains("\"price$#\" = '12.50'"), "{}", s[0].preview);
        // i valori non entrano mai nel testo eseguito
        assert!(!s[0].sql.contains("12.50") && !s[0].sql.contains("2024"));
    }

    #[test]
    fn empty_string_is_null() {
        let cols = [col("ID", "NUMBER", true), col("NOTE", "VARCHAR2", false)];
        let s = plan_edits(&tref("S", "T"), &cols, &[RowEdit::Update { pk: vec![cell("ID", Some("1"))], set: vec![cell("NOTE", Some(""))] }]).unwrap();
        assert_eq!(s[0].params, [None, Some("1".into())]);
        assert!(s[0].preview.contains("\"NOTE\" = NULL"));
        let s = plan_edits(&tref("S", "T"), &cols, &[RowEdit::Insert { values: vec![cell("ID", Some("2")), cell("NOTE", Some(""))] }]).unwrap();
        assert_eq!(s[0].params, [Some("2".into()), None]);
        // la chiave non può essere NULL, nemmeno come stringa vuota
        for v in [None, Some("")] {
            let e = plan_edits(&tref("S", "T"), &cols, &[RowEdit::Delete { pk: vec![cell("ID", v)] }]).unwrap_err();
            assert!(e.contains("non può essere NULL"), "{e}");
        }
    }

    #[test]
    fn composite_pk_and_delete() {
        let cols = [col("A", "NUMBER", true), col("B", "VARCHAR2", true), col("C", "DATE", false)];
        let s = plan_edits(&tref("S", "T"), &cols, &[RowEdit::Delete { pk: vec![cell("B", Some("x")), cell("A", Some("1"))] }]).unwrap();
        assert_eq!(s[0].sql, "DELETE FROM \"S\".\"T\" WHERE \"B\" = :1 AND \"A\" = TO_NUMBER(:2)");
        assert_eq!(s[0].kind, Kind::Delete);
        let up = |pk: Vec<Cell>| plan_edits(&tref("S", "T"), &cols, &[RowEdit::Update { pk, set: vec![cell("C", Some("2024-01-01 00:00:00"))] }]);
        assert!(up(vec![cell("A", Some("1"))]).unwrap_err().contains("Manca la colonna B"));
        assert!(up(vec![cell("A", Some("1")), cell("B", Some("x")), cell("C", Some("y"))]).unwrap_err().contains("non fa parte"));
        assert!(up(vec![cell("A", Some("1")), cell("A", Some("2")), cell("B", Some("x"))]).unwrap_err().contains("ripetuta"));
        assert!(up(vec![cell("Z", Some("1"))]).unwrap_err().contains("sconosciuta"));
    }

    #[test]
    fn insert_forms() {
        let cols = [col("ID", "NUMBER", true), col("T", "TIMESTAMP(6)", false), col("D", "DATE", false)];
        let s = plan_edits(&tref("S", "T"), &cols, &[RowEdit::Insert { values: vec![cell("T", Some("2024-01-01 00:00:00.0")), cell("ID", Some("3"))] }]).unwrap();
        assert_eq!(s[0].sql, "INSERT INTO \"S\".\"T\" (\"T\", \"ID\") VALUES (TO_TIMESTAMP(:1,'YYYY-MM-DD HH24:MI:SS.FF'), TO_NUMBER(:2))");
        assert_eq!(s[0].params.len(), 2);
        // niente `DEFAULT VALUES` (solo 23ai)
        let s = plan_edits(&tref("S", "T"), &cols, &[RowEdit::Insert { values: vec![] }]).unwrap();
        assert_eq!(s[0].sql, "INSERT INTO \"S\".\"T\" (\"ID\") VALUES (DEFAULT)");
        assert!(s[0].params.is_empty());
    }

    #[test]
    fn lob_and_unsupported_columns_are_rejected_by_name() {
        let cols = [col("ID", "NUMBER", true), col("BODY", "CLOB", false), col("IMG", "BLOB", false), col("RAWV", "RAW", false), col("LNG", "LONG", false)];
        for c in ["BODY", "IMG", "RAWV", "LNG"] {
            let e = plan_edits(&tref("S", "T"), &cols, &[RowEdit::Update { pk: vec![cell("ID", Some("1"))], set: vec![cell(c, Some("x"))] }]).unwrap_err();
            assert!(e.contains(c) && e.contains("non si modifica"), "{e}");
            let e = plan_edits(&tref("S", "T"), &cols, &[RowEdit::Insert { values: vec![cell(c, None)] }]).unwrap_err();
            assert!(e.contains(c), "{e}");
        }
        // una tabella con colonne LOB resta modificabile sulle altre colonne
        assert!(plan_edits(&tref("S", "T"), &cols, &[RowEdit::Update { pk: vec![cell("ID", Some("1"))], set: vec![cell("ID", Some("2"))] }]).is_ok());
    }

    #[test]
    fn plan_preconditions() {
        let cols = [col("ID", "NUMBER", true), col("V", "VARCHAR2", false)];
        let t = tref("S", "T");
        assert!(plan_edits(&t, &cols, &[]).is_err());
        assert!(plan_edits(&t, &[], &[RowEdit::Insert { values: vec![] }]).unwrap_err().contains("non trovata"));
        let nopk = [col("V", "VARCHAR2", false)];
        assert!(plan_edits(&t, &nopk, &[RowEdit::Insert { values: vec![] }]).unwrap_err().contains("chiave primaria"));
        assert!(plan_edits(&t, &cols, &[RowEdit::Update { pk: vec![cell("ID", Some("1"))], set: vec![] }]).is_err());
        assert!(plan_edits(&t, &cols, &[RowEdit::Update { pk: vec![cell("ID", Some("1"))], set: vec![cell("V", Some("a\0b"))] }]).unwrap_err().contains("NUL"));
        assert!(plan_edits(&t, &cols, &[RowEdit::Insert { values: vec![cell("V", Some("a")), cell("V", Some("b"))] }]).unwrap_err().contains("ripetuta"));
        let many: Vec<RowEdit> = (0..=MAX_EDITS).map(|_| RowEdit::Insert { values: vec![] }).collect();
        assert!(plan_edits(&t, &cols, &many).unwrap_err().contains("Troppe"));
    }

    #[test]
    fn long_values_are_clipped_in_the_preview_only() {
        let cols = [col("ID", "NUMBER", true), col("V", "VARCHAR2", false)];
        let long = "x".repeat(500);
        let s = plan_edits(&tref("S", "T"), &cols, &[RowEdit::Update { pk: vec![cell("ID", Some("1"))], set: vec![cell("V", Some(&long))] }]).unwrap();
        assert_eq!(s[0].params[0].as_deref(), Some(long.as_str()));
        assert!(s[0].preview.len() < 400 && s[0].preview.contains('…'));
    }
}
