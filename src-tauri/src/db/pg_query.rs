//! Albero e query Postgres. Le query ad hoc passano dal protocollo semplice (valori già testo, NULL distinto, più
//! istruzioni); una sola SELECT/WITH/VALUES/TABLE gira dentro un cursore (`BEGIN` + `DECLARE … NO SCROLL CURSOR` +
//! `FETCH 500`), tenuto aperto dal core per le pagine successive. Qui anche il controllo lessicale "di aiuto" delle
//! istruzioni distruttive (non è una garanzia: un'istruzione può fare danni anche senza essere segnalata).
use super::pg::{error_text, map_error, sanitize, tls_connector, Handle, Sessions, CONNECT_TIMEOUT, LOST};
use super::DbTls;
use crate::lock;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tauri::async_runtime::Mutex as AsyncMutex;
use tokio_postgres::{Client, NoTls, SimpleQueryMessage};

/// Righe per pagina.
pub const PAGE: usize = 500;
/// Righe lette al massimo per risultato (cursore).
pub const MAX_TOTAL: usize = 100_000;
/// Byte massimi di una cella (oltre: troncata, con indicazione).
pub const MAX_CELL: usize = 64 * 1024;
pub const EXPIRED: &str = "risultato scaduto, riesegui la query";
pub const IDLE_CLOSED: &str = "risultato chiuso dopo 90 s di inattività: riesegui la query";
/// Un cursore aperto (che tiene una transazione e i suoi lock) si chiude da solo dopo tanto tempo senza pagine.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
pub const USER_TX: &str = "Chiudi prima la transazione aperta (COMMIT o ROLLBACK)";
pub const CANCELLED: &str = "Query annullata";
pub const DECLINED: &str = "annullato";
pub const DESTRUCTIVE_TITLE: &str = "Istruzione distruttiva";
pub const BTN_RUN: &str = "Esegui";

// ---------------------------------------------------------------------------------------------------------------
// Analisi lessicale
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum K {
    Word,
    Open,
    Close,
    Other,
}

#[derive(Debug, Clone)]
struct Tok {
    k: K,
    /// Parola in maiuscolo (vuota se non è una parola).
    up: String,
    /// Profondità del gruppo di parentesi che contiene il token (aperta e chiusa: quella del genitore).
    depth: usize,
    /// Intervallo del token nel testo.
    s: usize,
    e: usize,
}

/// Un'istruzione di primo livello: l'intervallo nel testo (senza spazi, commenti e `;` ai bordi) e i suoi token.
#[derive(Debug)]
struct Stmt {
    start: usize,
    end: usize,
    toks: Vec<Tok>,
}

/// Dopo la virgoletta di apertura in `i`: l'indice subito dopo quella di chiusura (`''`/`""` raddoppiati; con
/// `backslash` anche `\x`, per le stringhe `E'…'`). Non chiusa: fino alla fine.
fn skip_quoted(b: &[u8], mut i: usize, backslash: bool) -> usize {
    let q = b[i];
    i += 1;
    while i < b.len() {
        if backslash && b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == q {
            if b.get(i + 1) == Some(&q) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

fn is_word_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_word_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

/// Spezza il testo in istruzioni di primo livello (separate da `;` fuori da parentesi, commenti e stringhe).
fn lex(sql: &str) -> Vec<Stmt> {
    let b = sql.as_bytes();
    let n = b.len();
    let (mut i, mut depth) = (0usize, 0usize);
    let mut out: Vec<Stmt> = Vec::new();
    let mut cur: Vec<Tok> = Vec::new();
    let (mut first, mut last) = (0usize, 0usize);
    macro_rules! push {
        ($k:expr, $up:expr, $s:expr, $e:expr, $d:expr) => {{
            if cur.is_empty() {
                first = $s;
            }
            last = $e;
            cur.push(Tok { k: $k, up: $up, depth: $d, s: $s, e: $e });
        }};
    }
    macro_rules! finish {
        () => {{
            if !cur.is_empty() {
                out.push(Stmt { start: first, end: last, toks: std::mem::take(&mut cur) });
            }
        }};
    }
    while i < n {
        let c = b[i];
        match c {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0c => i += 1,
            b'-' if b.get(i + 1) == Some(&b'-') => {
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                // commenti annidati
                let mut d = 1;
                i += 2;
                while i < n && d > 0 {
                    if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                        d += 1;
                        i += 2;
                    } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                        d -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            b'\'' | b'"' => {
                let s = i;
                i = skip_quoted(b, i, false);
                push!(K::Other, String::new(), s, i, depth);
            }
            b'$' => {
                // `$tag$ … $tag$`; `$1` è un parametro
                let s = i;
                let mut j = i + 1;
                if j < n && is_word_start(b[j]) {
                    while j < n && is_word_char(b[j]) && b[j] != b'$' {
                        j += 1;
                    }
                }
                if j < n && b[j] == b'$' {
                    let tag = &sql[i..=j];
                    i = match sql[j + 1..].find(tag) {
                        Some(p) => j + 1 + p + tag.len(),
                        None => n,
                    };
                } else {
                    i += 1;
                }
                push!(K::Other, String::new(), s, i, depth);
            }
            b';' if depth == 0 => {
                finish!();
                i += 1;
            }
            b'(' => {
                push!(K::Open, String::new(), i, i + 1, depth);
                depth += 1;
                i += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                push!(K::Close, String::new(), i, i + 1, depth);
                i += 1;
            }
            c if is_word_start(c) => {
                let s = i;
                while i < n && is_word_char(b[i]) {
                    i += 1;
                }
                if i - s == 1 && (c == b'e' || c == b'E') && b.get(i) == Some(&b'\'') {
                    // stringa E'…' con escape a barra rovesciata
                    i = skip_quoted(b, i, true);
                    push!(K::Other, String::new(), s, i, depth);
                } else {
                    push!(K::Word, sql[s..i].to_ascii_uppercase(), s, i, depth);
                }
            }
            _ => {
                push!(K::Other, String::new(), i, i + 1, depth);
                i += 1;
            }
        }
    }
    finish!();
    out
}

fn is_word(t: &Tok, w: &str) -> bool {
    t.k == K::Word && t.up == w
}

/// Istruzione segnalata dal controllo.
#[derive(Debug, Clone, PartialEq)]
pub struct Flag {
    pub reason: &'static str,
    pub text: String,
}

fn analyze(st: &Stmt) -> Option<&'static str> {
    let t = &st.toks;
    if t.iter().any(|x| x.depth == 0 && is_word(x, "DROP")) {
        return Some("DROP");
    }
    if t.iter().any(|x| x.depth == 0 && is_word(x, "TRUNCATE")) {
        return Some("TRUNCATE");
    }
    for (i, tk) in t.iter().enumerate() {
        let reason = if is_word(tk, "UPDATE") {
            "UPDATE senza WHERE"
        } else if is_word(tk, "DELETE") {
            "DELETE senza WHERE"
        } else {
            continue;
        };
        // un'istruzione, non `FOR UPDATE`, `ON DELETE`, `DO UPDATE`…: all'inizio dell'istruzione, di un gruppo tra
        // parentesi (corpo di una CTE) o subito dopo una CTE
        let starts = i == 0 || matches!(t[i - 1].k, K::Open | K::Close);
        if !starts {
            continue;
        }
        // il WHERE conta solo allo stesso livello, dentro lo stesso gruppo
        let has_where = t[i + 1..].iter().take_while(|x| x.depth >= tk.depth).any(|x| x.depth == tk.depth && is_word(x, "WHERE"));
        if !has_where {
            return Some(reason);
        }
    }
    None
}

/// Istruzioni di `sql` da confermare: `UPDATE`/`DELETE` senza `WHERE` di primo livello, `DROP`, `TRUNCATE`. Ignora
/// commenti, stringhe (anche `E''` e `$tag$…$tag$`) e identificatori tra virgolette. Controllo di aiuto, non una garanzia.
pub fn destructive(sql: &str) -> Vec<Flag> {
    lex(sql)
        .iter()
        .filter_map(|st| analyze(st).map(|reason| Flag { reason, text: sql[st.start..st.end].chars().take(200).collect() }))
        .collect()
}

/// Testo del dialogo di conferma.
pub fn confirm_message(flags: &[Flag]) -> String {
    let mut m = String::from("Questa query contiene istruzioni che possono modificare o cancellare molti dati:\n");
    for f in flags {
        m.push_str(&format!("\n• {}: {}", f.reason, f.text));
    }
    m.push_str("\n\nIl controllo è di aiuto, non una garanzia.");
    m
}

/// Una sola istruzione che può restituire righe attraverso un cursore.
fn cursor_candidate(st: &Stmt) -> bool {
    let Some(first) = st.toks.first() else { return false };
    if first.k != K::Word {
        return false;
    }
    match first.up.as_str() {
        "SELECT" | "VALUES" | "TABLE" => true,
        // `WITH x AS (…) INSERT/UPDATE/DELETE` non va in un cursore
        "WITH" => !st.toks.iter().enumerate().any(|(i, x)| {
            i > 0 && x.depth == 0 && x.k == K::Word && matches!(x.up.as_str(), "INSERT" | "UPDATE" | "DELETE" | "MERGE") && st.toks[i - 1].k == K::Close
        }),
        _ => false,
    }
}

/// Il testo contiene un'istruzione che apre una transazione (`BEGIN`, `START TRANSACTION`). Solo così una transazione
/// dell'utente resta aperta tra una query e l'altra: funzioni, procedure e blocchi `DO` non possono lasciarla aperta.
fn opens_transaction(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|st| st.toks.first().is_some_and(|t| is_word(t, "BEGIN") || is_word(t, "START")))
}

/// Un identificatore come lo vede il server: tra virgolette così com'è (`""` = `"`), senza virgolette in minuscolo
/// (solo ASCII, come Postgres in UTF-8). `None` se il token non è un identificatore.
fn ident(sql: &str, t: &Tok) -> Option<String> {
    let raw = &sql[t.s..t.e];
    match t.k {
        K::Word => Some(raw.to_ascii_lowercase()),
        K::Other if raw.len() >= 3 && raw.starts_with('"') && raw.ends_with('"') => {
            let inner = &raw[1..raw.len() - 1];
            // una virgoletta rimasta sola = identificatore non chiuso
            if inner.replace("\"\"", "").contains('"') {
                return None;
            }
            Some(inner.replace("\"\"", "\""))
        }
        _ => None,
    }
}

/// La lettura di una sola tabella, così stretta che ogni riga del risultato è una riga di quella tabella: una sola
/// istruzione `SELECT * FROM <tabella>` (nome con o senza schema, citato o no) seguita solo da `WHERE`, `ORDER BY`,
/// `LIMIT`, `OFFSET`; niente alias, virgole, `JOIN`, `UNION`/`INTERSECT`/`EXCEPT`, raggruppamenti o sottoquery nel
/// `FROM`. Restituisce schema (se scritto) e tabella come li risolve il server.
fn plain_table_read(sql: &str) -> Option<(Option<String>, String)> {
    let stmts = lex(sql);
    let [st] = stmts.as_slice() else { return None };
    let t = &st.toks;
    let text = |x: &Tok| &sql[x.s..x.e];
    if t.len() < 4 || !is_word(&t[0], "SELECT") || text(&t[1]) != "*" || !is_word(&t[2], "FROM") {
        return None;
    }
    let first = ident(sql, &t[3])?;
    let mut i = 4;
    let (schema, table) = if t.get(i).is_some_and(|x| text(x) == ".") {
        let second = ident(sql, t.get(i + 1)?)?;
        i += 2;
        (Some(first), second)
    } else {
        (None, first)
    };
    if let Some(x) = t.get(i)
        && !(x.k == K::Word && matches!(x.up.as_str(), "WHERE" | "ORDER" | "LIMIT" | "OFFSET"))
    {
        return None;
    }
    let forbidden = |x: &Tok| {
        x.depth == 0 && x.k == K::Word && matches!(x.up.as_str(), "UNION" | "INTERSECT" | "EXCEPT" | "JOIN" | "GROUP" | "HAVING" | "WINDOW" | "FOR" | "INTO" | "FETCH")
    };
    if t[i..].iter().any(forbidden) {
        return None;
    }
    Some((schema, table))
}

// ---------------------------------------------------------------------------------------------------------------
// Tipi
// ---------------------------------------------------------------------------------------------------------------

/// La tabella (dal catalogo) letta da una query modificabile in griglia.
#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EditableTable {
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PgTable {
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PgSchema {
    pub name: String,
    pub tables: Vec<PgTable>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PgTree {
    pub schemas: Vec<PgSchema>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PgColumn {
    pub name: String,
    /// Tipo con i modificatori (`character varying(10)`), da mostrare.
    #[serde(rename = "type")]
    pub type_name: String,
    /// Tipo senza modificatori (`character varying`, `bpchar`, `numeric`) per i parametri `$n::tipo`: un cast esplicito
    /// a `varchar(10)` troncherebbe in silenzio, mentre l'assegnazione alla colonna fa rispettare lunghezza e precisione.
    #[serde(skip)]
    pub cast_type: String,
    pub nullable: bool,
    pub is_pk: bool,
}

/// Il protocollo semplice non dice il tipo delle colonne: solo il nome.
#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueryColumn {
    pub name: String,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    /// Presente solo se restano righe da leggere (cursore aperto).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_id: Option<u64>,
    pub columns: Vec<QueryColumn>,
    pub rows: Vec<Vec<Option<String>>>,
    pub has_more: bool,
    /// Righe/righe modificate di ogni istruzione eseguita, in ordine (solo senza cursore).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affected: Option<Vec<u64>>,
    /// Celle troncate a 64 KB: coppie `[riga della pagina, colonna]`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub truncated: Vec<[usize; 2]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<String>,
    /// Solo MongoDB: il documento intero (JSON esteso canonico) di ogni riga; `None` oltre 1 MB (non modificabile).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<Option<String>>,
    /// Solo MongoDB: l'`_id` di ogni riga (JSON esteso canonico), da ridare così com'è a `db_mongo_replace`/`delete`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<Option<String>>,
    /// Solo Postgres: la tabella, se il testo è una lettura semplice di una sola tabella (griglia modificabile).
    pub editable_table: Option<EditableTable>,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueryPage {
    pub rows: Vec<Vec<Option<String>>>,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub truncated: Vec<[usize; 2]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<String>,
    /// Solo MongoDB: i documenti interi, gli `_id` e le colonne di questa pagina (l'unione dei campi di primo livello).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<Option<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<Option<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<QueryColumn>,
}

/// Tronca a `MAX_CELL` byte su un confine di carattere. `true` se ha troncato.
pub fn truncate_cell(s: &str) -> (String, bool) {
    if s.len() <= MAX_CELL {
        return (s.to_string(), false);
    }
    let mut e = MAX_CELL;
    while !s.is_char_boundary(e) {
        e -= 1;
    }
    (s[..e].to_string(), true)
}

pub fn qerr(e: &tokio_postgres::Error) -> String {
    if let Some(d) = e.as_db_error() {
        if d.code().code() == "57014" {
            return CANCELLED.into();
        }
        return format!("{} (SQLSTATE {})", d.message(), d.code().code());
    }
    if e.is_closed() {
        return LOST.into();
    }
    sanitize(&error_text(e), None)
}

#[derive(Default)]
struct Collected {
    columns: Vec<QueryColumn>,
    rows: Vec<Vec<Option<String>>>,
    truncated: Vec<[usize; 2]>,
    counts: Vec<u64>,
    /// L'ultimo insieme di righe aveva più di `PAGE` righe: le altre sono state scartate.
    overflow: bool,
}

/// Raccoglie la risposta del protocollo semplice: l'ultimo insieme di righe (al massimo `PAGE`) e i conteggi.
fn collect(msgs: Vec<SimpleQueryMessage>) -> Collected {
    let mut out = Collected::default();
    type Set = (Vec<QueryColumn>, Vec<Vec<Option<String>>>, Vec<[usize; 2]>, bool);
    let mut cur: Option<Set> = None;
    for m in msgs {
        match m {
            SimpleQueryMessage::RowDescription(cols) => {
                cur = Some((cols.iter().map(|c| QueryColumn { name: c.name().to_string() }).collect(), Vec::new(), Vec::new(), false));
            }
            SimpleQueryMessage::Row(r) => {
                let set = cur.get_or_insert_with(|| (Vec::new(), Vec::new(), Vec::new(), false));
                if set.1.len() >= PAGE {
                    set.3 = true;
                    continue;
                }
                let ri = set.1.len();
                let mut row = Vec::with_capacity(r.len());
                for ci in 0..r.len() {
                    row.push(r.get(ci).map(|v| {
                        let (s, t) = truncate_cell(v);
                        if t {
                            set.2.push([ri, ci]);
                        }
                        s
                    }));
                }
                set.1.push(row);
            }
            SimpleQueryMessage::CommandComplete(n) => {
                if let Some((c, r, t, o)) = cur.take() {
                    out.columns = c;
                    out.rows = r;
                    out.truncated = t;
                    out.overflow = o;
                }
                out.counts.push(n);
            }
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Sessioni di query
// ---------------------------------------------------------------------------------------------------------------

struct Open {
    conn: String,
    generation: u64,
    cursor: String,
    total: usize,
    /// Copia delle colonne e di tutte le righe già consegnate alla WebView (per l'esportazione; al massimo `MAX_TOTAL`).
    columns: Vec<QueryColumn>,
    rows: Vec<Vec<Option<String>>>,
    /// Risultato finito (cursore già chiuso con `COMMIT`, o nessun cursore): resta solo la copia delle righe, per
    /// l'esportazione, finché non lo sostituisce una nuova query della connessione o `close`.
    done: bool,
    /// Cursore chiuso per inattività: pagine ed esportazione rispondono `IDLE_CLOSED`.
    idle_closed: bool,
    /// Ultimo uso del cursore (apertura, pagina, esportazione).
    last_used: Instant,
}

type Results = Arc<Mutex<HashMap<u64, Open>>>;

/// I risultati con un cursore aperto (id unici per avvio dell'app). Per connessione c'è al più un risultato aperto,
/// condiviso da tutte le schede: una nuova query chiude il precedente (con `COMMIT`: gli effetti di funzioni chiamate
/// dalla SELECT si conservano; `ROLLBACK` solo dopo un errore o un annullamento). Un risultato appartiene alla
/// generazione della sessione: dopo una riconnessione è scaduto. Un cursore inattivo per `idle` si chiude da solo.
///
/// Le transazioni aperte dall'utente (`BEGIN` nel suo testo) non vengono mai concluse dal core: finché una è aperta,
/// niente cursore (la query gira direttamente) e niente modifiche dalla griglia.
pub struct Queries {
    next: AtomicU64,
    results: Results,
    /// Per connessione: il risultato aperto (o nessuno); il lucchetto serializza le query della stessa connessione.
    conns: Mutex<HashMap<String, Arc<AsyncMutex<Option<u64>>>>>,
    /// Connessioni (con la generazione della sessione) dove l'utente ha eseguito un `BEGIN`/`START`: lì si controlla col
    /// server se la transazione è ancora aperta. Altrove non può esserlo.
    maybe_tx: Mutex<HashMap<String, u64>>,
    idle: Duration,
}

impl Default for Queries {
    fn default() -> Self {
        Self::new()
    }
}

async fn end_txn(c: &Client, commit: bool) {
    if commit {
        if c.simple_query("COMMIT").await.is_err() {
            let _ = c.simple_query("ROLLBACK").await;
        }
    } else {
        let _ = c.simple_query("ROLLBACK").await;
    }
}

/// La transazione dell'utente è aperta? `statement_timestamp()` coincide con `now()` (l'inizio della transazione) solo
/// nel primo comando di una transazione: fuori da un blocco ogni messaggio ne apre una sua, quindi coincidono; dentro un
/// `BEGIN` precedente no. In una transazione fallita il server rifiuta tutto con 25P02: aperta anche quella.
async fn server_in_transaction(c: &Client) -> Result<bool, String> {
    match c.simple_query("SELECT now() = statement_timestamp()").await {
        Ok(msgs) => Ok(!msgs.iter().any(|m| matches!(m, SimpleQueryMessage::Row(r) if r.get(0) == Some("t")))),
        Err(e) if e.code().map(|c| c.code()) == Some("25P02") => Ok(true),
        Err(e) => Err(qerr(&e)),
    }
}

/// La tabella del catalogo per `schema.tabella` (o `tabella` risolta con `search_path`), con i nomi esatti.
async fn resolve_table(c: &Client, schema: Option<&str>, table: &str) -> Option<EditableTable> {
    let name = match schema {
        Some(s) => format!("{}.{}", super::pg_edit::quote_ident(s), super::pg_edit::quote_ident(table)),
        None => super::pg_edit::quote_ident(table),
    };
    let row = c
        .query_opt(
            "SELECT n.nspname::text, c.relname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.oid = to_regclass($1::text) AND c.relkind IN ('r','p','v','m')",
            &[&name],
        )
        .await
        .ok()??;
    Some(EditableTable { schema: row.get(0), table: row.get(1) })
}

impl Queries {
    pub fn new() -> Self {
        Self::with_idle(IDLE_TIMEOUT)
    }

    /// Con un'attesa d'inattività diversa (i test la accorciano).
    pub fn with_idle(idle: Duration) -> Self {
        Queries {
            next: AtomicU64::new(1),
            results: Arc::new(Mutex::new(HashMap::new())),
            conns: Mutex::new(HashMap::new()),
            maybe_tx: Mutex::new(HashMap::new()),
            idle,
        }
    }

    fn conn_lock(&self, id: &str) -> Arc<AsyncMutex<Option<u64>>> {
        lock(&self.conns).entry(id.to_string()).or_default().clone()
    }

    /// C'è una transazione aperta dall'utente su questa sessione? (lucchetto della connessione già preso)
    async fn user_tx_open(&self, h: &Handle, id: &str) -> Result<bool, String> {
        if lock(&self.maybe_tx).get(id) != Some(&h.generation) {
            return Ok(false);
        }
        let open = server_in_transaction(&h.client).await?;
        if !open {
            lock(&self.maybe_tx).remove(id);
        }
        Ok(open)
    }

    /// Connessione in esclusiva (le altre query aspettano): chiude un eventuale risultato aperto, che tiene una
    /// transazione, e dà la sessione. Con una transazione dell'utente aperta: errore `USER_TX` (il core non la chiude).
    /// Il lucchetto resta preso finché vive la guardia.
    pub async fn exclusive(&self, sessions: &Sessions, id: &str) -> Result<(tokio::sync::OwnedMutexGuard<Option<u64>>, Handle), String> {
        let mut guard = self.conn_lock(id).lock_owned().await;
        let h = sessions.handle(id)?;
        self.close_locked(&mut guard, Some(&h)).await;
        if self.user_tx_open(&h, id).await? {
            return Err(USER_TX.into());
        }
        Ok((guard, h))
    }

    /// Errore `USER_TX` se l'utente ha una transazione aperta (prima di chiedere conferme che poi verrebbero rifiutate).
    pub async fn check_no_user_tx(&self, sessions: &Sessions, id: &str) -> Result<(), String> {
        let l = self.conn_lock(id);
        let _cur = l.lock().await;
        let h = sessions.handle(id)?;
        if self.user_tx_open(&h, id).await? { Err(USER_TX.into()) } else { Ok(()) }
    }

    /// Chiude il risultato aperto della connessione (lucchetto già preso).
    async fn close_locked(&self, cur: &mut Option<u64>, h: Option<&Handle>) {
        let Some(rid) = cur.take() else { return };
        let open = lock(&self.results).remove(&rid);
        if let (Some(o), Some(h)) = (open, h)
            && !o.done
            && o.generation == h.generation
        {
            end_txn(&h.client, true).await;
        }
    }

    pub async fn query(&self, sessions: &Sessions, id: &str, text: &str) -> Result<QueryResult, String> {
        let stmts = lex(text);
        if stmts.is_empty() {
            return Err("Scrivi un'istruzione da eseguire".into());
        }
        let l = self.conn_lock(id);
        let mut cur = l.lock().await;
        let h = sessions.handle(id)?;
        self.close_locked(&mut cur, Some(&h)).await;
        let in_tx = self.user_tx_open(&h, id).await?;
        // dentro una transazione dell'utente niente cataloghi: un errore la farebbe fallire (e le modifiche sono rifiutate)
        let editable_table = match (in_tx, plain_table_read(text)) {
            (false, Some((schema, table))) => resolve_table(&h.client, schema.as_deref(), &table).await,
            _ => None,
        };
        if !in_tx && stmts.len() == 1 && cursor_candidate(&stmts[0]) {
            let stext = &text[stmts[0].start..stmts[0].end];
            if let Some(mut r) = self.open_cursor(&h, id, stext, &mut cur, &l).await? {
                r.editable_table = editable_table;
                return Ok(r);
            }
        }
        if opens_transaction(&stmts) {
            // da qui in poi si chiede al server, prima di ogni cursore o modifica, se la transazione è ancora aperta
            lock(&self.maybe_tx).insert(id.to_string(), h.generation);
        }
        let msgs = h.client.simple_query(text).await.map_err(|e| qerr(&e))?;
        let c = collect(msgs);
        let mut notices = Vec::new();
        if c.overflow {
            notices.push("solo le prime 500 righe: usa una sola SELECT per la paginazione".to_string());
        }
        if in_tx {
            notices.push("transazione aperta: la query gira fuori dal cursore (al massimo 500 righe). Chiudila con COMMIT o ROLLBACK".to_string());
        }
        let result_id = (!c.columns.is_empty()).then(|| self.retain_done(h.generation, id, &c.columns, &c.rows, &mut cur));
        Ok(QueryResult { result_id, columns: c.columns, rows: c.rows, has_more: false, affected: Some(c.counts), truncated: c.truncated, notices, editable_table, ..Default::default() })
    }

    /// Tiene la copia di un risultato già finito (nessun cursore aperto), come risultato della connessione.
    fn retain_done(&self, generation: u64, id: &str, columns: &[QueryColumn], rows: &[Vec<Option<String>>], cur: &mut Option<u64>) -> u64 {
        let rid = self.next.fetch_add(1, Ordering::Relaxed);
        lock(&self.results).insert(
            rid,
            Open {
                conn: id.to_string(),
                generation,
                cursor: String::new(),
                total: rows.len(),
                columns: columns.to_vec(),
                rows: rows.to_vec(),
                done: true,
                idle_closed: false,
                last_used: Instant::now(),
            },
        );
        *cur = Some(rid);
        rid
    }

    /// `Ok(None)` = il testo non può stare in un cursore (SQLSTATE 0A000): va eseguito direttamente.
    async fn open_cursor(&self, h: &Handle, id: &str, stext: &str, cur: &mut Option<u64>, conn: &Arc<AsyncMutex<Option<u64>>>) -> Result<Option<QueryResult>, String> {
        let rid = self.next.fetch_add(1, Ordering::Relaxed);
        let name = format!("codeart_c{rid}");
        h.client.simple_query("BEGIN").await.map_err(|e| qerr(&e))?;
        if let Err(e) = h.client.simple_query(&format!("DECLARE {name} NO SCROLL CURSOR FOR {stext}")).await {
            end_txn(&h.client, false).await;
            if e.code().map(|c| c.code()) == Some("0A000") {
                return Ok(None);
            }
            return Err(qerr(&e));
        }
        let page = match fetch(&h.client, &name).await {
            Ok(p) => p,
            Err(e) => {
                end_txn(&h.client, false).await;
                return Err(e);
            }
        };
        let has_more = page.rows.len() == PAGE;
        if has_more {
            lock(&self.results).insert(
                rid,
                Open {
                    conn: id.to_string(),
                    generation: h.generation,
                    cursor: name,
                    total: page.rows.len(),
                    columns: page.columns.clone(),
                    rows: page.rows.clone(),
                    done: false,
                    idle_closed: false,
                    last_used: Instant::now(),
                },
            );
            *cur = Some(rid);
            self.watch_idle(rid, conn.clone(), Arc::downgrade(&h.client));
        } else {
            end_txn(&h.client, true).await;
        }
        let result_id = if has_more { rid } else { self.retain_done(h.generation, id, &page.columns, &page.rows, cur) };
        Ok(Some(QueryResult { result_id: Some(result_id), columns: page.columns, rows: page.rows, has_more, truncated: page.truncated, ..Default::default() }))
    }

    /// Chiude il cursore `rid` dopo `idle` senza pagine né esportazioni: `COMMIT` (come una chiusura normale) e
    /// risultato segnato come chiuso per inattività. Una sessione nel frattempo sostituita non ha più il client.
    fn watch_idle(&self, rid: u64, conn: Arc<AsyncMutex<Option<u64>>>, client: Weak<Client>) {
        let results = self.results.clone();
        let idle = self.idle;
        tauri::async_runtime::spawn(async move {
            let mut wait = idle;
            loop {
                tokio::time::sleep(wait).await;
                let cur = conn.lock().await;
                if *cur != Some(rid) {
                    return;
                }
                let last = match lock(&results).get(&rid) {
                    Some(o) if !o.done => o.last_used,
                    _ => return,
                };
                let elapsed = last.elapsed();
                if elapsed < idle {
                    wait = idle - elapsed;
                    continue;
                }
                if let Some(o) = lock(&results).get_mut(&rid) {
                    o.done = true;
                    o.idle_closed = true;
                }
                if let Some(c) = client.upgrade() {
                    end_txn(&c, true).await;
                }
                drop(cur);
                return;
            }
        });
    }

    /// Pagina successiva di un risultato con cursore aperto.
    pub async fn more(&self, sessions: &Sessions, result_id: u64) -> Result<QueryPage, String> {
        let conn = lock(&self.results).get(&result_id).map(|o| o.conn.clone()).ok_or(EXPIRED)?;
        let l = self.conn_lock(&conn);
        let mut cur = l.lock().await;
        let info = lock(&self.results).get(&result_id).map(|o| (o.generation, o.cursor.clone(), o.total, o.done, o.idle_closed));
        let (generation, name, total, done, idle_closed) = match info {
            Some(i) if *cur == Some(result_id) => i,
            _ => return Err(EXPIRED.into()),
        };
        if idle_closed {
            return Err(IDLE_CLOSED.into());
        }
        if done {
            return Ok(QueryPage::default());
        }
        let h = match sessions.handle(&conn) {
            Ok(h) if h.generation == generation => h,
            _ => {
                lock(&self.results).remove(&result_id);
                *cur = None;
                return Err(EXPIRED.into());
            }
        };
        let page = match fetch(&h.client, &name).await {
            Ok(p) => p,
            Err(e) => {
                end_txn(&h.client, false).await;
                lock(&self.results).remove(&result_id);
                *cur = None;
                return Err(e);
            }
        };
        let total = total + page.rows.len();
        let mut notices = Vec::new();
        let mut has_more = page.rows.len() == PAGE;
        if has_more && total >= MAX_TOTAL {
            has_more = false;
            notices.push(format!("limite di {MAX_TOTAL} righe raggiunto: le altre non vengono lette"));
        }
        if !has_more {
            end_txn(&h.client, true).await;
        }
        if let Some(o) = lock(&self.results).get_mut(&result_id) {
            o.total = total;
            o.rows.extend(page.rows.iter().cloned());
            o.done = !has_more;
            o.last_used = Instant::now();
        }
        Ok(QueryPage { rows: page.rows, has_more, truncated: page.truncated, notices, ..Default::default() })
    }

    /// Per l'esportazione: colonne e tutte le righe del risultato (quelle già consegnate, tenute in copia, più le
    /// restanti, fino a `MAX_TOTAL`). Consuma il cursore: alla fine il risultato è finito (`hasMore = false`) ma resta
    /// tenuto, quindi si può esportare di nuovo. Le celle sono quelle già troncate a 64 KB.
    pub async fn drain(&self, sessions: &Sessions, result_id: u64) -> Result<(Vec<QueryColumn>, Vec<Vec<Option<String>>>), String> {
        let (columns, mut rows) = {
            let g = lock(&self.results);
            let o = g.get(&result_id).ok_or(EXPIRED)?;
            if o.idle_closed {
                return Err(IDLE_CLOSED.into());
            }
            (o.columns.clone(), o.rows.clone())
        };
        loop {
            let page = self.more(sessions, result_id).await?;
            rows.extend(page.rows);
            if !page.has_more {
                return Ok((columns, rows));
            }
        }
    }

    /// Chiude un risultato (COMMIT). Già chiuso o scaduto: nulla.
    pub async fn close(&self, sessions: &Sessions, result_id: u64) -> Result<(), String> {
        let Some(conn) = lock(&self.results).get(&result_id).map(|o| o.conn.clone()) else { return Ok(()) };
        let l = self.conn_lock(&conn);
        let mut cur = l.lock().await;
        if *cur == Some(result_id) {
            let h = sessions.handle(&conn).ok();
            self.close_locked(&mut cur, h.as_ref()).await;
        } else {
            lock(&self.results).remove(&result_id);
        }
        Ok(())
    }
}

/// Annulla la query in corso sulla connessione (stesso TLS della sessione). Il lavoro in corso termina con
/// l'errore «Query annullata», fa `ROLLBACK` e scarta il risultato.
pub async fn cancel(sessions: &Sessions, id: &str) -> Result<(), String> {
    let h = sessions.handle(id)?;
    let run = async {
        match h.params.tls {
            DbTls::Disable => h.cancel.cancel_query(NoTls).await,
            _ => h.cancel.cancel_query(tls_connector(&h.params)?).await,
        }
        .map_err(|e| map_error(&e, None))
    };
    tokio::time::timeout(CONNECT_TIMEOUT, run).await.unwrap_or_else(|_| Err(super::pg::TIMED_OUT.into()))
}

struct Page {
    columns: Vec<QueryColumn>,
    rows: Vec<Vec<Option<String>>>,
    truncated: Vec<[usize; 2]>,
}

async fn fetch(client: &Client, cursor: &str) -> Result<Page, String> {
    let msgs = client.simple_query(&format!("FETCH {PAGE} FROM {cursor}")).await.map_err(|e| qerr(&e))?;
    let c = collect(msgs);
    Ok(Page { columns: c.columns, rows: c.rows, truncated: c.truncated })
}

// ---------------------------------------------------------------------------------------------------------------
// Albero
// ---------------------------------------------------------------------------------------------------------------

const SYSTEM_SCHEMAS: [&str; 2] = ["pg_catalog", "information_schema"];

/// Schemi, tabelle e viste (esclusi gli schemi di sistema). Solo query con parametri.
pub async fn tree(sessions: &Sessions, id: &str) -> Result<PgTree, String> {
    let c = sessions.client(id)?;
    let excl: Vec<String> = SYSTEM_SCHEMAS.iter().map(|s| s.to_string()).collect();
    let schemas = c
        .query("SELECT n.nspname::text FROM pg_namespace n WHERE n.nspname::text <> ALL($1::text[]) AND n.nspname !~ '^pg_(toast|temp)' ORDER BY 1", &[&excl])
        .await
        .map_err(|e| qerr(&e))?;
    let rels = c
        .query(
            "SELECT n.nspname::text, c.relname::text, c.relkind::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r','p','v','m') AND n.nspname::text <> ALL($1::text[]) AND n.nspname !~ '^pg_(toast|temp)' ORDER BY 1, 2",
            &[&excl],
        )
        .await
        .map_err(|e| qerr(&e))?;
    let mut out: Vec<PgSchema> = schemas.iter().map(|r| PgSchema { name: r.get(0), tables: Vec::new() }).collect();
    for r in &rels {
        let (schema, name, kind): (String, String, String) = (r.get(0), r.get(1), r.get(2));
        if let Some(s) = out.iter_mut().find(|s| s.name == schema) {
            s.tables.push(PgTable { name, kind: if matches!(kind.as_str(), "v" | "m") { "view" } else { "table" }.into() });
        }
    }
    Ok(PgTree { schemas: out })
}

/// Colonne di una tabella o vista, nell'ordine della definizione.
pub async fn columns(sessions: &Sessions, id: &str, schema: &str, table: &str) -> Result<Vec<PgColumn>, String> {
    let c = sessions.client(id)?;
    let rows = c
        .query(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), format_type(a.atttypid, -1), NOT a.attnotnull, \
                    EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = c.oid AND i.indisprimary AND a.attnum = ANY(i.indkey)) \
             FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname::text = $1 AND c.relname::text = $2 AND c.relkind IN ('r','p','v','m') AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            &[&schema, &table],
        )
        .await
        .map_err(|e| qerr(&e))?;
    Ok(rows.iter().map(|r| PgColumn { name: r.get(0), type_name: r.get(1), cast_type: r.get(2), nullable: r.get(3), is_pk: r.get(4) }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reasons(sql: &str) -> Vec<&'static str> {
        destructive(sql).into_iter().map(|f| f.reason).collect()
    }

    #[test]
    fn flags_update_delete_without_where() {
        assert_eq!(reasons("DELETE FROM t"), ["DELETE senza WHERE"]);
        assert_eq!(reasons("update t set a = 1"), ["UPDATE senza WHERE"]);
        assert!(reasons("DELETE FROM t WHERE id = 1").is_empty());
        assert!(reasons("UPDATE t SET a = 1 WHERE a IS NULL; DELETE FROM u WHERE x").is_empty());
        assert_eq!(reasons("SELECT 1; DELETE FROM t; UPDATE u SET a=1 WHERE a=1"), ["DELETE senza WHERE"]);
        // non sono istruzioni
        assert!(reasons("SELECT * FROM t FOR UPDATE").is_empty());
        assert!(reasons("CREATE TABLE a (b int REFERENCES c ON DELETE CASCADE ON UPDATE CASCADE)").is_empty());
        assert!(reasons("INSERT INTO t VALUES (1) ON CONFLICT (a) DO UPDATE SET b = 2").is_empty());
        assert!(reasons("SELECT 1").is_empty());
        assert!(reasons("").is_empty());
    }

    #[test]
    fn flags_drop_and_truncate() {
        assert_eq!(reasons("DROP TABLE t"), ["DROP"]);
        assert_eq!(reasons("drop schema s cascade"), ["DROP"]);
        assert_eq!(reasons("TRUNCATE t"), ["TRUNCATE"]);
        assert_eq!(reasons("ALTER TABLE t DROP COLUMN c"), ["DROP"]);
        assert_eq!(reasons("SELECT 1; DROP TABLE t; TRUNCATE u"), ["DROP", "TRUNCATE"]);
    }

    #[test]
    fn ignores_comments_strings_and_quoted_identifiers() {
        assert!(reasons("-- DROP TABLE t\nSELECT 1").is_empty());
        assert!(reasons("/* DELETE FROM t */ SELECT 1").is_empty());
        assert!(reasons("/* a /* DROP TABLE t */ ancora commento DROP TABLE t */ SELECT 1").is_empty());
        assert!(reasons("SELECT 'DROP TABLE t; DELETE FROM t'").is_empty());
        assert!(reasons("SELECT 'it''s; DROP TABLE t'").is_empty());
        assert!(reasons(r"SELECT E'a\'; DROP TABLE t'").is_empty());
        assert!(reasons("SELECT $$ DROP TABLE t; $$").is_empty());
        assert!(reasons("SELECT $f$ DELETE FROM t; $$ ancora $f$").is_empty());
        assert!(reasons(r#"SELECT "DROP" FROM "DELETE""#).is_empty());
        assert!(reasons("DO $body$ BEGIN DELETE FROM t; END $body$").is_empty());
        // un commento dopo il DELETE non lo salva, e un WHERE in un commento o in una stringa nemmeno
        assert_eq!(reasons("DELETE FROM t -- WHERE x"), ["DELETE senza WHERE"]);
        assert_eq!(reasons("DELETE FROM t /* WHERE x */"), ["DELETE senza WHERE"]);
        assert_eq!(reasons("DELETE FROM t RETURNING 'WHERE'"), ["DELETE senza WHERE"]);
        // la fine di un commento o di una stringa non chiusi
        assert!(reasons("SELECT 1 -- DROP").is_empty());
        assert!(reasons("SELECT 'aperta DROP").is_empty());
    }

    #[test]
    fn where_only_counts_at_top_level() {
        // WHERE in una sottoquery o in una CTE non conta
        assert_eq!(reasons("DELETE FROM t USING (SELECT id FROM u WHERE a) s"), ["DELETE senza WHERE"]);
        assert_eq!(reasons("UPDATE t SET a = (SELECT 1 FROM u WHERE b)"), ["UPDATE senza WHERE"]);
        assert!(reasons("UPDATE t SET a = (SELECT 1 FROM u WHERE b) WHERE t.id = 1").is_empty());
        assert_eq!(reasons("WITH x AS (DELETE FROM a WHERE id = 1 RETURNING id) DELETE FROM t"), ["DELETE senza WHERE"]);
        assert!(reasons("WITH x AS (DELETE FROM a WHERE id = 1 RETURNING id) DELETE FROM t WHERE id IN (SELECT id FROM x)").is_empty());
        // una DELETE dentro una CTE, senza WHERE, è segnalata
        assert_eq!(reasons("WITH x AS (DELETE FROM a RETURNING id) SELECT * FROM x"), ["DELETE senza WHERE"]);
        assert!(reasons("WITH x AS (DELETE FROM a WHERE id = 1 RETURNING id) SELECT * FROM x").is_empty());
        // un `;` dentro le parentesi non separa
        assert_eq!(reasons("SELECT (1;2); DELETE FROM t").len(), 1);
    }

    #[test]
    fn flag_text_is_the_statement_trimmed_and_capped() {
        let f = destructive("  -- c\n DELETE FROM t ; SELECT 1");
        assert_eq!(f[0].text, "DELETE FROM t");
        let long = format!("DELETE FROM t RETURNING '{}'", "é".repeat(500));
        assert_eq!(destructive(&long)[0].text.chars().count(), 200);
        let m = confirm_message(&f);
        assert!(m.contains("DELETE FROM t") && m.contains("non una garanzia"));
    }

    #[test]
    fn splits_statements_and_picks_cursor_candidates() {
        let s = lex("SELECT 1; ; /* x */ ; SELECT 'a;b' ;");
        assert_eq!(s.len(), 2);
        let cand = |q: &str| {
            let s = lex(q);
            s.len() == 1 && cursor_candidate(&s[0])
        };
        assert!(cand("select 1;"));
        assert!(cand("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(cand("VALUES (1)"));
        assert!(cand("TABLE t"));
        assert!(!cand("WITH x AS (DELETE FROM t RETURNING *) DELETE FROM u"));
        assert!(!cand("WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x"));
        assert!(!cand("INSERT INTO t VALUES (1)"));
        assert!(!cand("SHOW server_version"));
        assert!(!cand("SELECT 1; SELECT 2"));
        assert!(!cand("-- solo un commento"));
    }

    #[test]
    fn plain_table_read_is_strict() {
        let s = |x: &str| Some(x.to_string());
        let plain_table_read = |q: &str| plain_table_read(q).map(|(a, b)| (a, Some(b)));
        assert_eq!(plain_table_read(r#"SELECT * FROM "public"."users""#), Some((s("public"), s("users"))));
        assert_eq!(plain_table_read(r#"SELECT * FROM "public"."users";"#), Some((s("public"), s("users"))));
        assert_eq!(plain_table_read(r#"select * from "Ugly ""Name"""."Ta ""ble" where id > 3 order by id desc, "x" limit 10 offset 5"#), Some((s("Ugly \"Name\""), s("Ta \"ble"))));
        // senza virgolette: minuscole come fa il server; senza schema: lo risolve il server col search_path
        assert_eq!(plain_table_read("SELECT * FROM Public.Users WHERE a IN (SELECT b FROM c UNION SELECT d FROM e)"), Some((s("public"), s("users"))));
        assert_eq!(plain_table_read("SELECT * FROM users WHERE a IS DISTINCT FROM b"), Some((None, s("users"))));
        // tutto il resto: niente griglia modificabile
        for q in [
            r#"SELECT * FROM "public"."users" UNION ALL SELECT * FROM "public"."users_archive""#,
            r#"SELECT * FROM "public"."users", "public"."orders""#,
            r#"SELECT * FROM "public"."users"; SELECT * FROM "public"."other""#,
            r#"SELECT * FROM "public"."users" u JOIN "public"."orders" o ON o.u = u.id"#,
            r#"SELECT * FROM "public"."users" AS u"#,
            r#"SELECT * FROM "public"."users" WHERE true INTERSECT SELECT * FROM x"#,
            r#"SELECT * FROM "public"."users" WHERE true EXCEPT SELECT * FROM x"#,
            r#"SELECT * FROM (SELECT * FROM "public"."users") s"#,
            r#"SELECT * FROM ONLY "public"."users""#,
            r#"SELECT * FROM "public"."users" GROUP BY id"#,
            r#"SELECT * FROM "public"."users" FOR UPDATE"#,
            r#"SELECT * FROM "public"."users" FETCH FIRST 3 ROWS ONLY"#,
            r#"SELECT id, name FROM "public"."users""#,
            r#"SELECT u.* FROM "public"."users" u"#,
            r#"SELECT DISTINCT * FROM "public"."users""#,
            r#"WITH x AS (SELECT 1) SELECT * FROM "public"."users""#,
            r#"SELECT * FROM U&"d\0061t""#,
            r#"SELECT * FROM "public"."users"#,
            r#"SELECT * FROM """#,
            "SELECT * FROM 'users'",
            "SELECT *",
            "TABLE users",
        ] {
            assert_eq!(plain_table_read(q), None, "{q}");
        }
        // il nome citato resta com'è: "Users" non è users
        assert_eq!(plain_table_read(r#"select * from "public"."Users""#), Some((s("public"), s("Users"))));
        assert_eq!(plain_table_read(r#"select * from public.Users"#), Some((s("public"), s("users"))));
    }

    #[test]
    fn transaction_openers_are_detected() {
        let o = |q: &str| opens_transaction(&lex(q));
        assert!(o("BEGIN"));
        assert!(o("begin; update t set a = 1 where id = 7"));
        assert!(o("START TRANSACTION ISOLATION LEVEL SERIALIZABLE"));
        assert!(o("SELECT 1; /* x */ begin isolation level read committed"));
        assert!(!o("DO $$ BEGIN PERFORM 1; END $$"));
        assert!(!o("SELECT 'BEGIN'"));
        assert!(!o("COMMIT"));
        assert!(!o("SELECT begin FROM t"));
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        assert_eq!(truncate_cell("ciao"), ("ciao".to_string(), false));
        let exact = "a".repeat(MAX_CELL);
        assert!(!truncate_cell(&exact).1);
        let big = "a".repeat(MAX_CELL + 1);
        let (s, t) = truncate_cell(&big);
        assert!(t && s.len() == MAX_CELL);
        // un carattere a 3 byte a cavallo del limite: si taglia prima
        let mut s = "a".repeat(MAX_CELL - 1);
        s.push('€');
        let (c, t) = truncate_cell(&s);
        assert!(t && c.len() == MAX_CELL - 1);
    }

    #[test]
    fn collect_keeps_last_set_and_counts() {
        let c = collect(vec![SimpleQueryMessage::CommandComplete(0), SimpleQueryMessage::CommandComplete(3)]);
        assert_eq!(c.counts, [0, 3]);
        assert!(c.rows.is_empty() && !c.overflow);
    }

    // ---- integrazione: cluster temporaneo -------------------------------------------------------------------

    use super::super::pg::{PgParams, Sessions};
    use super::super::testutil::Cluster;
    use crate::ssh::conn::{Connections, SshPaths};

    fn cell(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    #[test]
    #[ignore = "avvia un cluster Postgres temporaneo (initdb e pg_ctl nel PATH)"]
    fn live_cluster_tree_and_queries() {
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
            let sessions = Arc::new(Sessions::new(conns, Arc::new(|_| {})));
            sessions.connect("c1", p.clone(), Some("test-pass".into()), None).await.unwrap();
            let q = Arc::new(Queries::new());

            // albero con nomi strani
            let r = q
                .query(
                    &sessions,
                    "c1",
                    r#"CREATE SCHEMA "Ugly ""Name""";
                       CREATE TABLE "Ugly ""Name"""."Ta ""ble" (id int PRIMARY KEY, "Nome Col" text NOT NULL, note varchar(20));
                       CREATE VIEW "Ugly ""Name"""."una vista" AS SELECT 1 AS x;
                       CREATE TABLE plain (a int, b text)"#,
                )
                .await
                .unwrap();
            assert_eq!(r.affected, Some(vec![0, 0, 0, 0]));
            let tree = tree(&sessions, "c1").await.unwrap();
            let names: Vec<&str> = tree.schemas.iter().map(|s| s.name.as_str()).collect();
            assert!(names.contains(&"public") && names.contains(&"Ugly \"Name\""), "{names:?}");
            assert!(!names.iter().any(|n| *n == "pg_catalog" || *n == "information_schema" || n.starts_with("pg_toast") || n.starts_with("pg_temp")));
            let ugly = tree.schemas.iter().find(|s| s.name == "Ugly \"Name\"").unwrap();
            let t: Vec<(&str, &str)> = ugly.tables.iter().map(|t| (t.name.as_str(), t.kind.as_str())).collect();
            assert_eq!(t, [("Ta \"ble", "table"), ("una vista", "view")]);
            let cols = columns(&sessions, "c1", "Ugly \"Name\"", "Ta \"ble").await.unwrap();
            let c: Vec<(&str, &str, bool, bool)> = cols.iter().map(|c| (c.name.as_str(), c.type_name.as_str(), c.nullable, c.is_pk)).collect();
            assert_eq!(c, [("id", "integer", false, true), ("Nome Col", "text", false, false), ("note", "character varying(20)", true, false)]);
            assert!(columns(&sessions, "c1", "public", "non_esiste").await.unwrap().is_empty());

            // paginazione: 1 200 righe = 500 + 500 + 200
            let r = q.query(&sessions, "c1", "SELECT g AS n, 'riga ' || g AS t FROM generate_series(1, 1200) g;").await.unwrap();
            assert_eq!(r.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["n", "t"]);
            assert_eq!(r.rows.len(), 500);
            assert!(r.has_more && r.affected.is_none());
            assert_eq!(r.rows[0], [cell("1"), cell("riga 1")]);
            let rid = r.result_id.expect("cursore aperto");
            let p2 = q.more(&sessions, rid).await.unwrap();
            assert_eq!((p2.rows.len(), p2.has_more), (500, true));
            assert_eq!(p2.rows[0][0], cell("501"));
            let p3 = q.more(&sessions, rid).await.unwrap();
            assert_eq!((p3.rows.len(), p3.has_more), (200, false));
            assert_eq!(p3.rows[199][0], cell("1200"));
            // finito: tenuto (per l'esportazione), nessuna riga in più
            let p4 = q.more(&sessions, rid).await.unwrap();
            assert!(p4.rows.is_empty() && !p4.has_more);
            assert_eq!(q.drain(&sessions, rid).await.unwrap().1.len(), 1200);
            // la sessione è tornata libera (nessuna transazione rimasta aperta)
            let st = sessions.client("c1").unwrap().simple_query("SELECT txid_current_if_assigned()").await.unwrap();
            assert!(!st.is_empty());

            // una nuova query chiude il risultato precedente; close è idempotente
            let r1 = q.query(&sessions, "c1", "SELECT g FROM generate_series(1, 700) g").await.unwrap();
            let id1 = r1.result_id.unwrap();
            let r2 = q.query(&sessions, "c1", "SELECT g FROM generate_series(1, 700) g").await.unwrap();
            assert_eq!(q.more(&sessions, id1).await.unwrap_err(), EXPIRED);
            q.close(&sessions, r2.result_id.unwrap()).await.unwrap();
            q.close(&sessions, r2.result_id.unwrap()).await.unwrap();
            assert_eq!(q.more(&sessions, r2.result_id.unwrap()).await.unwrap_err(), EXPIRED);
            // dopo la chiusura si può scrivere (nessuna transazione appesa)
            q.query(&sessions, "c1", "INSERT INTO plain VALUES (1, 'x')").await.unwrap();

            // NULL distinto da stringa vuota; tipi resi come testo
            let r = q
                .query(
                    &sessions,
                    "c1",
                    "SET timezone = 'UTC'; SELECT NULL::text AS nul, ''::text AS vuoto, 12345678901234567890.123456789::numeric AS num, \
                     '2024-03-05 10:20:30.5+00'::timestamptz AS ts, '{\"a\": [1, 2]}'::jsonb AS j, '\\xdeadbeef'::bytea AS b, \
                     ARRAY[1,2,3]::int[] AS arr, true AS bo",
                )
                .await
                .unwrap();
            assert!(!r.has_more && r.result_id.is_some());
            let row = &r.rows[0];
            assert_eq!(row[0], None);
            assert_eq!(row[1], cell(""));
            assert_eq!(row[2], cell("12345678901234567890.123456789"));
            assert!(row[3].as_deref().unwrap().starts_with("2024-03-05 10:20:30.5"), "{:?}", row[3]);
            assert_eq!(row[4], cell("{\"a\": [1, 2]}"));
            assert_eq!(row[5], cell("\\xdeadbeef"));
            assert_eq!(row[6], cell("{1,2,3}"));
            assert_eq!(row[7], cell("t"));

            // più istruzioni: conteggi; ultimo insieme di righe
            let r = q
                .query(
                    &sessions,
                    "c1",
                    "CREATE TABLE m (a int); INSERT INTO m SELECT generate_series(1,3); UPDATE m SET a = a + 1 WHERE a > 1; DELETE FROM m WHERE a = 4; SELECT a FROM m ORDER BY a",
                )
                .await
                .unwrap();
            assert_eq!(r.affected, Some(vec![0, 3, 2, 1, 2]));
            assert_eq!(r.rows, [vec![cell("1")], vec![cell("3")]]);
            assert!(!r.has_more && r.notices.is_empty());
            // più istruzioni con un insieme grande: solo 500 righe e un avviso
            let r = q.query(&sessions, "c1", "SELECT 1; SELECT g FROM generate_series(1, 800) g").await.unwrap();
            assert_eq!(r.rows.len(), 500);
            assert_eq!(r.notices.len(), 1);
            // WITH con DML e RETURNING: eseguito direttamente
            let r = q.query(&sessions, "c1", "WITH x AS (INSERT INTO m VALUES (9) RETURNING a) SELECT a FROM x").await.unwrap();
            assert_eq!(r.rows, [vec![cell("9")]]);
            assert!(r.result_id.is_some());
            // senza righe restituite (solo conteggi): nessun risultato da esportare
            assert!(q.query(&sessions, "c1", "INSERT INTO m VALUES (10)").await.unwrap().result_id.is_none());
            // errore: messaggio del server, la sessione resta usabile
            let e = q.query(&sessions, "c1", "SELECT * FROM non_esiste").await.unwrap_err();
            assert!(e.contains("non_esiste") && e.contains("42P01"), "{e}");
            assert_eq!(q.query(&sessions, "c1", "SELECT 1").await.unwrap().rows, [vec![cell("1")]]);
            assert!(q.query(&sessions, "c1", " -- niente\n ").await.is_err());
            assert!(q.query(&sessions, "zzz", "SELECT 1").await.is_err());

            // cella grande troncata
            let r = q.query(&sessions, "c1", "SELECT repeat('é', 40000) AS big, 'ok' AS small").await.unwrap();
            assert_eq!(r.truncated, [[0, 0]]);
            assert!(r.rows[0][0].as_ref().unwrap().len() <= MAX_CELL);
            assert_eq!(r.rows[0][1], cell("ok"));

            // annullamento di pg_sleep entro ~2 s; la sessione resta usabile
            let (q2, s2) = (q.clone(), sessions.clone());
            let started = Instant::now();
            let h = tauri::async_runtime::spawn(async move { q2.query(&s2, "c1", "SELECT pg_sleep(30)").await });
            tokio::time::sleep(Duration::from_millis(500)).await;
            cancel(&sessions, "c1").await.unwrap();
            let e = h.await.unwrap().unwrap_err();
            assert_eq!(e, CANCELLED);
            assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
            assert_eq!(q.query(&sessions, "c1", "SELECT 7").await.unwrap().rows, [vec![cell("7")]]);
            // annullamento durante una pagina successiva: risultato scartato
            let r = q.query(&sessions, "c1", "SELECT g, pg_sleep(0.02) FROM generate_series(1, 1200) g").await.unwrap();
            let rid = r.result_id.unwrap();
            let (q2, s2) = (q.clone(), sessions.clone());
            let h = tauri::async_runtime::spawn(async move { q2.more(&s2, rid).await });
            tokio::time::sleep(Duration::from_millis(500)).await;
            cancel(&sessions, "c1").await.unwrap();
            assert_eq!(h.await.unwrap().unwrap_err(), CANCELLED);
            assert_eq!(q.more(&sessions, rid).await.unwrap_err(), EXPIRED);
            assert_eq!(q.query(&sessions, "c1", "SELECT 8").await.unwrap().rows, [vec![cell("8")]]);

            // riconnessione: i risultati aperti scadono
            let r = q.query(&sessions, "c1", "SELECT g FROM generate_series(1, 700) g").await.unwrap();
            let rid = r.result_id.unwrap();
            sessions.disconnect("c1");
            sessions.connect("c1", p.clone(), Some("test-pass".into()), None).await.unwrap();
            assert_eq!(q.more(&sessions, rid).await.unwrap_err(), EXPIRED);
            assert_eq!(q.query(&sessions, "c1", "SELECT 1").await.unwrap().rows.len(), 1);
            sessions.disconnect("c1");
            assert!(cancel(&sessions, "c1").await.is_err());
        });
    }

    #[test]
    #[ignore = "avvia un cluster Postgres temporaneo (initdb e pg_ctl nel PATH)"]
    fn live_cluster_user_transactions_idle_cursor_and_editable_table() {
        use super::super::pg_edit::{self, RowEdit};
        let Some(cluster) = Cluster::start() else {
            eprintln!("initdb/pg_ctl non trovati: saltato");
            return;
        };
        let p = PgParams { host: "127.0.0.1".into(), port: cluster.port, user: Some("codeart".into()), database: Some("postgres".into()), tls: DbTls::Disable, accept_invalid_certs: false };
        tauri::async_runtime::block_on(async {
            let conns = Arc::new(Connections::new(SshPaths::new(std::env::temp_dir()), Arc::new(|_| {})));
            let sessions = Arc::new(Sessions::new(conns, Arc::new(|_| {})));
            sessions.connect("c1", p.clone(), Some("test-pass".into()), None).await.unwrap();
            let q = Arc::new(Queries::with_idle(Duration::from_millis(700)));
            q.query(
                &sessions,
                "c1",
                r#"CREATE TABLE acc (id int PRIMARY KEY, balance int);
                   INSERT INTO acc VALUES (7, 100);
                   INSERT INTO acc SELECT g, g FROM generate_series(100, 1300) g;
                   CREATE TABLE "Users" (id int PRIMARY KEY);
                   CREATE TABLE users (id int PRIMARY KEY);
                   CREATE VIEW v AS SELECT 1 AS x"#,
            )
            .await
            .unwrap();
            let client = sessions.client("c1").unwrap();
            assert!(!server_in_transaction(&client).await.unwrap());

            // ---- I1: la transazione dell'utente non viene mai conclusa dal core ----
            q.query(&sessions, "c1", "BEGIN; UPDATE acc SET balance = 0 WHERE id = 7").await.unwrap();
            assert!(server_in_transaction(&client).await.unwrap());
            // una SELECT grande dentro la transazione: niente cursore (che farebbe COMMIT), vede l'UPDATE non confermato
            let r = q.query(&sessions, "c1", "SELECT * FROM acc ORDER BY id").await.unwrap();
            assert!(!r.has_more && r.affected.is_some());
            assert_eq!(r.rows.len(), 500);
            assert_eq!(r.rows[0], [cell("7"), cell("0")]);
            assert!(r.notices.iter().any(|n| n.contains("transazione aperta")), "{:?}", r.notices);
            assert_eq!(r.editable_table, None);
            // le modifiche dalla griglia sono rifiutate (prima della conferma e in esecuzione)
            assert_eq!(q.check_no_user_tx(&sessions, "c1").await.unwrap_err(), USER_TX);
            let stmts = pg_edit::plan(&sessions, "c1", "public", "acc", &[RowEdit::Update { pk: vec![("id".into(), cell("7"))], set: vec![("balance".into(), cell("5"))] }]).await.unwrap();
            assert_eq!(pg_edit::execute(&q, &sessions, "c1", &stmts).await.unwrap_err(), USER_TX);
            // ancora aperta: anche dopo un errore (transazione fallita, 25P02)
            assert!(q.query(&sessions, "c1", "SELECT 1/0").await.is_err());
            assert_eq!(q.check_no_user_tx(&sessions, "c1").await.unwrap_err(), USER_TX);
            q.query(&sessions, "c1", "ROLLBACK").await.unwrap();
            // l'UPDATE è sparito
            assert_eq!(q.query(&sessions, "c1", "SELECT balance FROM acc WHERE id = 7").await.unwrap().rows, [vec![cell("100")]]);
            q.check_no_user_tx(&sessions, "c1").await.unwrap();
            // fuori dalla transazione: di nuovo cursore e modifiche
            let r = q.query(&sessions, "c1", "SELECT * FROM acc").await.unwrap();
            assert!(r.has_more);
            assert_eq!(pg_edit::execute(&q, &sessions, "c1", &stmts).await.unwrap().affected, 1);
            // BEGIN … COMMIT nello stesso testo; START TRANSACTION … COMMIT in due volte
            q.query(&sessions, "c1", "BEGIN; UPDATE acc SET balance = 1 WHERE id = 7; COMMIT").await.unwrap();
            q.check_no_user_tx(&sessions, "c1").await.unwrap();
            q.query(&sessions, "c1", "START TRANSACTION; INSERT INTO users VALUES (1)").await.unwrap();
            assert_eq!(q.check_no_user_tx(&sessions, "c1").await.unwrap_err(), USER_TX);
            q.query(&sessions, "c1", "COMMIT").await.unwrap();
            q.check_no_user_tx(&sessions, "c1").await.unwrap();
            assert_eq!(q.query(&sessions, "c1", "SELECT count(*) FROM users").await.unwrap().rows, [vec![cell("1")]]);

            // ---- I2: un cursore inattivo si chiude da solo ----
            let r = q.query(&sessions, "c1", "SELECT g FROM generate_series(1, 3000) g").await.unwrap();
            let rid = r.result_id.unwrap();
            assert!(r.has_more && server_in_transaction(&client).await.unwrap());
            // l'uso lo tiene aperto
            for _ in 0..2 {
                tokio::time::sleep(Duration::from_millis(400)).await;
                assert!(q.more(&sessions, rid).await.unwrap().has_more);
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
            assert_eq!(q.more(&sessions, rid).await.unwrap_err(), IDLE_CLOSED);
            assert_eq!(q.drain(&sessions, rid).await.unwrap_err(), IDLE_CLOSED);
            // transazione conclusa (COMMIT): la sessione è libera, le query successive vanno
            assert!(!server_in_transaction(&client).await.unwrap());
            assert_eq!(q.query(&sessions, "c1", "SELECT 3").await.unwrap().rows, [vec![cell("3")]]);
            q.close(&sessions, rid).await.unwrap();
            // un cursore chiuso prima del tempo non viene toccato dal controllo
            let r = q.query(&sessions, "c1", "SELECT g FROM generate_series(1, 3000) g").await.unwrap();
            q.close(&sessions, r.result_id.unwrap()).await.unwrap();
            q.query(&sessions, "c1", "BEGIN").await.unwrap();
            tokio::time::sleep(Duration::from_millis(1200)).await;
            assert!(server_in_transaction(&client).await.unwrap(), "il controllo d'inattività non deve chiudere la transazione dell'utente");
            q.query(&sessions, "c1", "ROLLBACK").await.unwrap();

            // ---- I5: la tabella modificabile viene dal core, col nome esatto del catalogo ----
            let et = |s: &str, t: &str| Some(EditableTable { schema: s.into(), table: t.into() });
            assert_eq!(q.query(&sessions, "c1", r#"SELECT * FROM "public"."acc" WHERE id = 7"#).await.unwrap().editable_table, et("public", "acc"));
            assert_eq!(q.query(&sessions, "c1", "select * from ACC order by id limit 600").await.unwrap().editable_table, et("public", "acc"));
            assert_eq!(q.query(&sessions, "c1", r#"select * from "Users""#).await.unwrap().editable_table, et("public", "Users"));
            assert_eq!(q.query(&sessions, "c1", "select * from Users").await.unwrap().editable_table, et("public", "users"));
            assert_eq!(q.query(&sessions, "c1", "SELECT * FROM v").await.unwrap().editable_table, et("public", "v"));
            assert_eq!(q.query(&sessions, "c1", "SELECT * FROM acc UNION ALL SELECT * FROM acc").await.unwrap().editable_table, None);
            assert_eq!(q.query(&sessions, "c1", r#"SELECT * FROM "public"."users", "public"."acc""#).await.unwrap().editable_table, None);
            assert_eq!(q.query(&sessions, "c1", r#"SELECT * FROM "public"."users"; SELECT * FROM "public"."acc""#).await.unwrap().editable_table, None);
            assert!(q.query(&sessions, "c1", "SELECT * FROM non_esiste").await.is_err());
            sessions.disconnect("c1");
        });
    }
}
