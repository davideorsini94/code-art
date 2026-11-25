//! Albero e query Oracle. Il lessico è quello della 4a (`pg_query`) adattato a Oracle: stringhe `q'[…]'` e `N'…'`,
//! commenti non annidati, `$` e `#` negli identificatori, blocchi PL/SQL (`BEGIN`/`DECLARE`/`CREATE … PROCEDURE|FUNCTION|
//! PACKAGE|TRIGGER|TYPE`) inviati come una sola istruzione fino a una riga con la sola `/` (convenzione di SQL*Plus), `;`
//! finale tolto dalle istruzioni semplici (Oracle lo rifiuta). Il controllo delle istruzioni distruttive è "di aiuto".
//!
//! I risultati con righe restano nel thread della sessione (`Results`, nello stato dell'attore): il `ResultSet` del
//! driver (`Statement::into_result_set`, che ne prende possesso e quindi non è legato a un prestito) sta dentro un
//! `RowCursor` e `More` lo continua. Qui anche i valori come testo, puri e provati senza server.
use super::mongo;
use super::ora::{Backend, OraErr, OraSessions, Request};
use super::pg_query::{
    Flag, PgSchema, PgTable, PgTree, QueryColumn, QueryPage, QueryResult, CANCELLED, EXPIRED, IDLE_CLOSED, MAX_CELL, MAX_TOTAL, PAGE,
};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::time::{Duration, Instant};

/// Gli id dei risultati Oracle stanno in `[2^61, 2^62)`: sotto Postgres, sopra MongoDB.
pub const ID_BASE: u64 = 1 << 61;
pub const UNSUPPORTED: &str = "<tipo non supportato>";
/// Byte massimi di un valore binario (RAW, BLOB, LONG RAW) mostrato in esadecimale.
pub const MAX_BIN: usize = 32 * 1024;

pub fn is_ora_result(result_id: u64) -> bool {
    (ID_BASE..mongo::ID_BASE).contains(&result_id)
}

// ---------------------------------------------------------------------------------------------------------------
// Errori delle operazioni
// ---------------------------------------------------------------------------------------------------------------

/// Esito negativo di un'operazione dell'attore: un errore del driver (da cui si capisce se la connessione è caduta)
/// o un messaggio già pronto.
#[derive(Debug, Clone, PartialEq)]
pub enum OpErr {
    Db(OraErr),
    Msg(String),
}

impl From<OraErr> for OpErr {
    fn from(e: OraErr) -> OpErr {
        OpErr::Db(e)
    }
}

impl OpErr {
    pub fn msg(m: impl Into<String>) -> OpErr {
        OpErr::Msg(m.into())
    }
    pub fn is_connection_lost(&self) -> bool {
        matches!(self, OpErr::Db(e) if e.is_connection_lost())
    }
    /// Il testo mostrato: «Query annullata» per ORA-01013, «connessione persa» se caduta, altrimenti il messaggio del server.
    pub fn text(&self) -> String {
        match self {
            OpErr::Msg(m) => m.clone(),
            OpErr::Db(e) if e.ora == Some(1013) => CANCELLED.to_string(),
            OpErr::Db(e) if e.is_connection_lost() => super::pg::LOST.to_string(),
            OpErr::Db(e) => e.message.trim().to_string(),
        }
    }
    fn in_statement(self, i: usize, n: usize) -> OpErr {
        match self {
            OpErr::Db(mut e) if n > 1 && e.ora != Some(1013) => {
                e.message = format!("Istruzione {} di {}: {}", i + 1, n, e.message.trim());
                OpErr::Db(e)
            }
            other => other,
        }
    }
}

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
    depth: usize,
    s: usize,
    e: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Block {
    /// SQL semplice (`;` finale tolto).
    No,
    /// `BEGIN … END;` o `DECLARE … END;`.
    Anonymous,
    /// `CREATE [OR REPLACE] PROCEDURE|FUNCTION|PACKAGE|TRIGGER|TYPE …`.
    Stored,
}

/// Un'istruzione di primo livello: l'intervallo nel testo (senza spazi, commenti e terminatori ai bordi; per i blocchi
/// PL/SQL il `;` finale resta) e i suoi token.
#[derive(Debug)]
struct Stmt {
    start: usize,
    end: usize,
    toks: Vec<Tok>,
    block: Block,
}

/// Dopo la virgoletta di apertura in `i`: l'indice subito dopo quella di chiusura (`''`/`""` raddoppiati).
fn skip_quoted(b: &[u8], mut i: usize) -> usize {
    let q = b[i];
    i += 1;
    while i < b.len() {
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

/// Il carattere che chiude una stringa `q'X…X'` aperta da `X`.
fn q_closer(d: u8) -> u8 {
    match d {
        b'[' => b']',
        b'{' => b'}',
        b'(' => b')',
        b'<' => b'>',
        c => c,
    }
}

/// `b[quote]` è l'apice di `q'X…X'` (il delimitatore `X` è `b[quote + 1]`): indice dopo l'apice di chiusura.
fn skip_q(b: &[u8], quote: usize) -> usize {
    let c = q_closer(b[quote + 1]);
    let mut i = quote + 2;
    while i + 1 < b.len() {
        if b[i] == c && b[i + 1] == b'\'' {
            return i + 2;
        }
        i += 1;
    }
    b.len()
}

fn is_word_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_word_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'$' | b'#') || c >= 0x80
}

fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | 0x0c)
}

/// Il `/` in `i` è solo sulla sua riga (spazi ammessi)?
fn lone_slash(b: &[u8], i: usize) -> bool {
    let mut j = i;
    while j > 0 {
        j -= 1;
        match b[j] {
            b'\n' => break,
            c if is_blank(c) => {}
            _ => return false,
        }
    }
    let mut k = i + 1;
    while k < b.len() && b[k] != b'\n' {
        if !is_blank(b[k]) {
            return false;
        }
        k += 1;
    }
    true
}

/// `Some(tipo)` appena i primi token dicono se l'istruzione è un blocco PL/SQL; `None` finché non si sa.
fn block_kind(cur: &[Tok]) -> Option<Block> {
    let first = cur.first()?;
    if first.k != K::Word {
        return Some(Block::No);
    }
    match first.up.as_str() {
        "BEGIN" | "DECLARE" => Some(Block::Anonymous),
        "CREATE" => {
            for t in &cur[1..] {
                if t.k != K::Word {
                    return Some(Block::No);
                }
                match t.up.as_str() {
                    "OR" | "REPLACE" | "EDITIONABLE" | "NONEDITIONABLE" | "FORCE" => continue,
                    "PROCEDURE" | "FUNCTION" | "PACKAGE" | "TRIGGER" | "TYPE" => return Some(Block::Stored),
                    _ => return Some(Block::No),
                }
            }
            None
        }
        _ => Some(Block::No),
    }
}

/// Spezza il testo in istruzioni di primo livello: `;` fuori da parentesi, commenti e stringhe, oppure una riga con la
/// sola `/`. Un blocco PL/SQL non si spezza sui `;` ma solo sulla `/` (o alla fine del testo).
fn lex(sql: &str) -> Vec<Stmt> {
    let b = sql.as_bytes();
    let n = b.len();
    let (mut i, mut depth) = (0usize, 0usize);
    let mut out: Vec<Stmt> = Vec::new();
    let mut cur: Vec<Tok> = Vec::new();
    let (mut first, mut last) = (0usize, 0usize);
    let mut block = Block::No;
    let mut decided = false;
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
                out.push(Stmt { start: first, end: last, toks: std::mem::take(&mut cur), block });
            }
            block = Block::No;
            decided = false;
            depth = 0;
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
            // i commenti di Oracle non si annidano
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < n && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i = (i + 2).min(n);
            }
            b'/' if (block != Block::No || depth == 0) && lone_slash(b, i) => {
                finish!();
                i += 1;
            }
            b'\'' | b'"' => {
                let s = i;
                i = skip_quoted(b, i);
                push!(K::Other, String::new(), s, i, depth);
            }
            b';' if block != Block::No => {
                push!(K::Other, String::new(), i, i + 1, depth);
                i += 1;
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
                let w = &sql[s..i];
                let quote = b.get(i) == Some(&b'\'');
                if quote && (w.eq_ignore_ascii_case("q") || w.eq_ignore_ascii_case("nq")) && b.get(i + 1).is_some_and(|d| !is_blank(*d) && *d != b'\n') {
                    // stringa q'X…X'
                    i = skip_q(b, i);
                    push!(K::Other, String::new(), s, i, depth);
                } else if quote && w.eq_ignore_ascii_case("n") {
                    i = skip_quoted(b, i);
                    push!(K::Other, String::new(), s, i, depth);
                } else {
                    push!(K::Word, w.to_ascii_uppercase(), s, i, depth);
                    if !decided
                        && let Some(k) = block_kind(&cur)
                    {
                        decided = true;
                        block = k;
                    }
                }
            }
            _ => {
                push!(K::Other, String::new(), i, i + 1, depth);
                i += 1;
            }
        }
    }
    if !cur.is_empty() {
        out.push(Stmt { start: first, end: last, toks: cur, block });
    }
    out
}

fn is_word(t: &Tok, w: &str) -> bool {
    t.k == K::Word && t.up == w
}

/// Le istruzioni da inviare al server, una per volta: senza `;` finale quelle semplici, intere (con il `;` finale) i
/// blocchi PL/SQL.
pub fn split_statements(sql: &str) -> Vec<String> {
    lex(sql).iter().map(|st| sql[st.start..st.end].to_string()).collect()
}

fn analyze(sql: &str, st: &Stmt) -> Option<&'static str> {
    let t = &st.toks;
    let semi = |x: &Tok| x.k == K::Other && &sql[x.s..x.e] == ";";
    match st.block {
        Block::Stored => return None,
        Block::No => {
            if t.first().is_some_and(|x| is_word(x, "ALTER")) && t.iter().any(|x| x.depth == 0 && is_word(x, "DROP")) {
                return Some("ALTER … DROP");
            }
            if t.first().is_some_and(|x| is_word(x, "PURGE")) {
                return Some("PURGE");
            }
            if t.iter().any(|x| x.depth == 0 && is_word(x, "DROP")) {
                return Some("DROP");
            }
            if t.iter().any(|x| x.depth == 0 && is_word(x, "TRUNCATE")) {
                return Some("TRUNCATE");
            }
        }
        Block::Anonymous => {}
    }
    // dentro `FORALL … ;` l'UPDATE/DELETE segue i limiti (`1..n`, `INDICES OF c`, `SAVE EXCEPTIONS`): è l'istruzione
    let mut forall = false;
    for (i, tk) in t.iter().enumerate() {
        if st.block == Block::Anonymous {
            if is_word(tk, "FORALL") {
                forall = true;
            } else if semi(tk) {
                forall = false;
            }
        }
        let reason = if is_word(tk, "UPDATE") {
            "UPDATE senza WHERE"
        } else if is_word(tk, "DELETE") {
            "DELETE senza WHERE"
        } else {
            continue;
        };
        // un'istruzione, non `FOR UPDATE`, `AFTER DELETE`, `THEN UPDATE` di un MERGE…: all'inizio, dopo una parentesi o
        // (nei blocchi) dopo `;`, `BEGIN`, `THEN`, `ELSE`, `LOOP`
        let starts = i == 0
            || forall
            || matches!(t[i - 1].k, K::Open | K::Close)
            || (st.block == Block::Anonymous && (semi(&t[i - 1]) || ["BEGIN", "THEN", "ELSE", "LOOP"].iter().any(|w| is_word(&t[i - 1], w))));
        if !starts {
            continue;
        }
        let has_where = t[i + 1..].iter().take_while(|x| x.depth >= tk.depth && !(st.block == Block::Anonymous && semi(x))).any(|x| x.depth == tk.depth && is_word(x, "WHERE"));
        if !has_where {
            return Some(reason);
        }
    }
    None
}

/// Istruzioni di `sql` da confermare: `UPDATE`/`DELETE` senza `WHERE` di primo livello (anche nei blocchi anonimi),
/// `DROP`, `ALTER … DROP`, `TRUNCATE`, `PURGE`. Ignora commenti, stringhe (anche `q'[…]'`) e identificatori tra
/// virgolette. Controllo di aiuto, non una garanzia (SQL dinamico e procedure non si vedono).
pub fn destructive(sql: &str) -> Vec<Flag> {
    lex(sql).iter().filter_map(|st| analyze(sql, st).map(|reason| Flag { reason, text: sql[st.start..st.end].chars().take(200).collect() })).collect()
}

/// Un identificatore come lo vede il server: tra virgolette così com'è (`""` = `"`), senza virgolette in maiuscolo.
fn ident(sql: &str, t: &Tok) -> Option<String> {
    let raw = &sql[t.s..t.e];
    match t.k {
        K::Word => Some(raw.to_uppercase()),
        K::Other if raw.len() >= 3 && raw.starts_with('"') && raw.ends_with('"') => {
            let inner = &raw[1..raw.len() - 1];
            if inner.replace("\"\"", "").contains('"') {
                return None;
            }
            Some(inner.replace("\"\"", "\""))
        }
        _ => None,
    }
}

/// La lettura di una sola tabella: una sola istruzione `SELECT * FROM [proprietario.]tabella` seguita solo da `WHERE`,
/// `ORDER BY`, `OFFSET … ROWS`, `FETCH FIRST|NEXT … ROWS ONLY`; niente alias, virgole, `JOIN`, insiemi, raggruppamenti,
/// `FOR UPDATE`, `CONNECT BY`. Restituisce proprietario (se scritto) e tabella come li risolve il server.
pub fn plain_table_read(sql: &str) -> Option<(Option<String>, String)> {
    let stmts = lex(sql);
    let [st] = stmts.as_slice() else { return None };
    if st.block != Block::No {
        return None;
    }
    let t = &st.toks;
    let text = |x: &Tok| &sql[x.s..x.e];
    if t.len() < 4 || !is_word(&t[0], "SELECT") || text(&t[1]) != "*" || !is_word(&t[2], "FROM") {
        return None;
    }
    let first = ident(sql, &t[3])?;
    let mut i = 4;
    let (owner, table) = if t.get(i).is_some_and(|x| text(x) == ".") {
        let second = ident(sql, t.get(i + 1)?)?;
        i += 2;
        (Some(first), second)
    } else {
        (None, first)
    };
    if let Some(x) = t.get(i)
        && !(x.k == K::Word && matches!(x.up.as_str(), "WHERE" | "ORDER" | "OFFSET" | "FETCH"))
    {
        return None;
    }
    let forbidden = |x: &Tok| {
        x.depth == 0
            && x.k == K::Word
            && matches!(x.up.as_str(), "UNION" | "INTERSECT" | "MINUS" | "JOIN" | "GROUP" | "HAVING" | "MODEL" | "FOR" | "INTO" | "CONNECT" | "START" | "PIVOT" | "UNPIVOT" | "SAMPLE")
    };
    if t[i..].iter().any(forbidden) {
        return None;
    }
    Some((owner, table))
}

// ---------------------------------------------------------------------------------------------------------------
// Valori come testo (funzioni pure)
// ---------------------------------------------------------------------------------------------------------------

/// Come si legge una colonna: dal tipo Oracle del driver.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColKind {
    /// Testo così com'è (VARCHAR2, CHAR, NVARCHAR2, NCHAR, ROWID, LONG, BINARY_FLOAT/DOUBLE, intervalli).
    Text,
    /// NUMBER e FLOAT: il testo del server, con lo zero iniziale che a volte manca (`.5`).
    Number,
    Date,
    /// Con la precisione delle frazioni di secondo e se ha il fuso orario.
    Timestamp(u8, bool),
    Clob,
    Nclob,
    Blob,
    /// RAW e LONG RAW.
    Raw,
    Unsupported,
}

pub fn col_kind(t: &oracle::sql_type::OracleType) -> ColKind {
    use oracle::sql_type::OracleType as T;
    match t {
        T::Varchar2(_) | T::NVarchar2(_) | T::Char(_) | T::NChar(_) | T::Rowid | T::Long | T::BinaryFloat | T::BinaryDouble | T::IntervalDS(..) | T::IntervalYM(_) => ColKind::Text,
        T::Number(..) | T::Float(_) => ColKind::Number,
        T::Date => ColKind::Date,
        T::Timestamp(p) => ColKind::Timestamp(*p, false),
        T::TimestampTZ(p) | T::TimestampLTZ(p) => ColKind::Timestamp(*p, true),
        T::CLOB => ColKind::Clob,
        T::NCLOB => ColKind::Nclob,
        T::BLOB => ColKind::Blob,
        T::Raw(_) | T::LongRaw => ColKind::Raw,
        _ => ColKind::Unsupported,
    }
}

/// Il testo di un NUMBER senza lo zero iniziale (`.5`, `-.5`) lo ottiene.
pub fn normalize_number(s: &str) -> String {
    let s = s.trim();
    match s.strip_prefix('-') {
        Some(r) if r.starts_with('.') => format!("-0{r}"),
        _ if s.starts_with('.') => format!("0{s}"),
        _ => s.to_string(),
    }
}

/// Esadecimale maiuscolo (come `RAWTOHEX`) dei primi `MAX_BIN` byte; `true` se ne ha scartati.
pub fn hex_text(bytes: &[u8]) -> (String, bool) {
    let cut = bytes.len() > MAX_BIN;
    let mut s = String::with_capacity(bytes.len().min(MAX_BIN) * 2);
    for b in &bytes[..bytes.len().min(MAX_BIN)] {
        s.push_str(&format!("{b:02X}"));
    }
    (s, cut)
}

/// Legge al più `limit + 1` byte (il byte in più dice se c'era altro) senza leggere un LOB intero in memoria.
pub fn read_limited(r: &mut impl Read, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(limit.min(8 * 1024) + 1);
    let mut buf = [0u8; 8 * 1024];
    while out.len() <= limit {
        let want = buf.len().min(limit + 1 - out.len());
        let k = r.read(&mut buf[..want])?;
        if k == 0 {
            break;
        }
        out.extend_from_slice(&buf[..k]);
    }
    Ok(out)
}

/// Testo (UTF-8) da una lettura limitata a `MAX_CELL` byte: se ce n'erano di più, taglia su un confine di carattere.
pub fn clob_text(mut bytes: Vec<u8>) -> (String, bool) {
    let cut = bytes.len() > MAX_CELL;
    if cut {
        bytes.truncate(MAX_CELL);
    }
    match String::from_utf8(bytes) {
        Ok(s) => (s, cut),
        Err(e) => {
            let ok = e.utf8_error().valid_up_to();
            let bytes = e.into_bytes();
            // un carattere spezzato alla fine (taglio) si scarta; byte non validi in mezzo diventano `�`
            if cut && ok + 4 > bytes.len() { (String::from_utf8_lossy(&bytes[..ok]).into_owned(), true) } else { (String::from_utf8_lossy(&bytes).into_owned(), cut) }
        }
    }
}

/// I campi di una data/ora come li dà il driver.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ts {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub nano: u32,
    pub tz_h: i32,
    pub tz_m: i32,
}

/// `YYYY-MM-DD HH24:MI:SS` (DATE) o con `.FF` (almeno una cifra) e `TZH:TZM`: il formato delle NLS della sessione, che
/// `TO_DATE`/`TO_TIMESTAMP`/`TO_TIMESTAMP_TZ` rileggono identico.
pub fn format_timestamp(t: &Ts, kind: ColKind) -> String {
    let mut s = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second);
    if let ColKind::Timestamp(prec, tz) = kind {
        let digits = prec.clamp(1, 9) as usize;
        s.push('.');
        s.push_str(&format!("{:09}", t.nano)[..digits]);
        if tz {
            let sign = if t.tz_h < 0 || t.tz_m < 0 { '-' } else { '+' };
            s.push_str(&format!(" {sign}{:02}:{:02}", t.tz_h.abs(), t.tz_m.abs()));
        }
    }
    s
}

fn ts_of(t: &oracle::sql_type::Timestamp) -> Ts {
    Ts {
        year: t.year(),
        month: t.month(),
        day: t.day(),
        hour: t.hour(),
        minute: t.minute(),
        second: t.second(),
        nano: t.nanosecond(),
        tz_h: t.tz_hour_offset(),
        tz_m: t.tz_minute_offset(),
    }
}

fn cut_text(s: String, ri: usize, ci: usize, cuts: &mut Vec<[usize; 2]>) -> String {
    let (t, c) = super::pg_query::truncate_cell(&s);
    if c {
        cuts.push([ri, ci]);
    }
    t
}

/// Una riga del driver come celle di testo (`None` = NULL) e le colonne troncate.
fn render_row(row: &oracle::Row, kinds: &[ColKind], ri: usize, cuts: &mut Vec<[usize; 2]>) -> Result<Vec<Option<String>>, OraErr> {
    use oracle::sql_type::{Blob, Clob, Nclob, Timestamp};
    let e = |e: oracle::Error| OraErr::from(&e);
    let io = |e: std::io::Error| OraErr::new(format!("Lettura del valore non riuscita: {e}"));
    let mut out = Vec::with_capacity(kinds.len());
    for (ci, kind) in kinds.iter().enumerate() {
        let cell = match kind {
            ColKind::Text => row.get::<usize, Option<String>>(ci).map_err(e)?.map(|s| cut_text(s, ri, ci, cuts)),
            ColKind::Number => row.get::<usize, Option<String>>(ci).map_err(e)?.map(|s| normalize_number(&s)),
            ColKind::Date | ColKind::Timestamp(..) => row.get::<usize, Option<Timestamp>>(ci).map_err(e)?.map(|t| format_timestamp(&ts_of(&t), *kind)),
            ColKind::Clob => match row.get::<usize, Option<Clob>>(ci).map_err(e)? {
                Some(mut l) => Some(lob_text(read_limited(&mut l, MAX_CELL).map_err(io)?, ri, ci, cuts)),
                None => None,
            },
            ColKind::Nclob => match row.get::<usize, Option<Nclob>>(ci).map_err(e)? {
                Some(mut l) => Some(lob_text(read_limited(&mut l, MAX_CELL).map_err(io)?, ri, ci, cuts)),
                None => None,
            },
            ColKind::Blob => match row.get::<usize, Option<Blob>>(ci).map_err(e)? {
                Some(mut l) => Some(bin_text(&read_limited(&mut l, MAX_BIN).map_err(io)?, ri, ci, cuts)),
                None => None,
            },
            ColKind::Raw => row.get::<usize, Option<Vec<u8>>>(ci).map_err(e)?.map(|b| bin_text(&b, ri, ci, cuts)),
            ColKind::Unsupported => Some(UNSUPPORTED.to_string()),
        };
        out.push(cell);
    }
    Ok(out)
}

fn lob_text(bytes: Vec<u8>, ri: usize, ci: usize, cuts: &mut Vec<[usize; 2]>) -> String {
    let (s, c) = clob_text(bytes);
    if c {
        cuts.push([ri, ci]);
    }
    s
}

fn bin_text(bytes: &[u8], ri: usize, ci: usize, cuts: &mut Vec<[usize; 2]>) -> String {
    let (s, c) = hex_text(bytes);
    if c {
        cuts.push([ri, ci]);
    }
    s
}

// ---------------------------------------------------------------------------------------------------------------
// Cursori e risultati (nello stato dell'attore)
// ---------------------------------------------------------------------------------------------------------------

/// Colonne e tutte le righe di un risultato (esportazione).
pub type Drained = (Vec<QueryColumn>, Vec<Vec<Option<String>>>);

/// Un insieme di righe aperto nel thread della sessione.
pub trait RowCursor: Send {
    fn columns(&self) -> Vec<QueryColumn>;
    /// Fino a `max` righe. `done` = le righe sono finite (il driver non ne ha altre).
    fn fetch(&mut self, max: usize) -> Result<Fetched, OraErr>;
}

#[derive(Debug, Default)]
pub struct Fetched {
    pub rows: Vec<Vec<Option<String>>>,
    pub truncated: Vec<[usize; 2]>,
    pub done: bool,
}

/// L'esito di un'istruzione.
pub enum Executed {
    Rows(Box<dyn RowCursor>),
    Affected(u64),
}

/// Il cursore vero: il `ResultSet` che possiede il suo `Statement`.
pub struct OraCursor {
    pub(super) rs: oracle::ResultSet<'static, oracle::Row>,
    pub(super) columns: Vec<QueryColumn>,
    pub(super) kinds: Vec<ColKind>,
}

impl RowCursor for OraCursor {
    fn columns(&self) -> Vec<QueryColumn> {
        self.columns.clone()
    }
    fn fetch(&mut self, max: usize) -> Result<Fetched, OraErr> {
        let mut f = Fetched::default();
        while f.rows.len() < max {
            match self.rs.next() {
                None => {
                    f.done = true;
                    break;
                }
                Some(Err(e)) => return Err(OraErr::from(&e)),
                Some(Ok(row)) => {
                    let ri = f.rows.len();
                    let cells = render_row(&row, &self.kinds, ri, &mut f.truncated)?;
                    f.rows.push(cells);
                }
            }
        }
        Ok(f)
    }
}

struct Held {
    columns: Vec<QueryColumn>,
    /// Copia di tutte le righe già consegnate (per l'esportazione; al massimo `MAX_TOTAL`).
    rows: Vec<Vec<Option<String>>>,
    total: usize,
    /// `None` = risultato finito (resta la copia).
    cursor: Option<Box<dyn RowCursor>>,
    idle_closed: bool,
    last_used: Instant,
}

/// I risultati della sessione, nello stato del thread: al più uno (una nuova query chiude il precedente).
#[derive(Default)]
pub struct Results {
    held: HashMap<u64, Held>,
}

impl Results {
    /// Chiude i cursori fermi da `idle` (la copia si scarta: pagine ed esportazione rispondono `IDLE_CLOSED`).
    pub fn expire(&mut self, now: Instant, idle: Duration) {
        for h in self.held.values_mut() {
            if h.cursor.is_some() && now.saturating_duration_since(h.last_used) >= idle {
                h.cursor = None;
                h.idle_closed = true;
                h.rows = Vec::new();
            }
        }
    }

    /// Quando scade il primo cursore aperto.
    pub fn next_deadline(&self, idle: Duration) -> Option<Instant> {
        self.held.values().filter(|h| h.cursor.is_some()).map(|h| h.last_used + idle).min()
    }

    /// Esegue le istruzioni in ordine (autocommit: ognuna conclusa dalla precedente); l'ultimo insieme di righe resta
    /// aperto come risultato `rid`, gli altri danno solo il numero di righe lette (fino a una pagina).
    pub fn query(&mut self, b: &mut dyn Backend, rid: u64, stmts: &[String]) -> Result<QueryResult, OpErr> {
        self.held.clear();
        let n = stmts.len();
        let mut counts: Vec<u64> = Vec::new();
        for (i, sql) in stmts.iter().enumerate() {
            let at = |e: OpErr| e.in_statement(i, n);
            match b.run(sql).map_err(|e| at(e.into()))? {
                Executed::Affected(k) => counts.push(k),
                Executed::Rows(mut cur) => {
                    let columns = cur.columns();
                    let f = cur.fetch(PAGE).map_err(|e| at(e.into()))?;
                    if i + 1 < n {
                        counts.push(f.rows.len() as u64);
                        continue;
                    }
                    let has_more = !f.done && f.rows.len() == PAGE;
                    let affected = if n > 1 {
                        counts.push(f.rows.len() as u64);
                        Some(counts)
                    } else {
                        None
                    };
                    self.held.insert(
                        rid,
                        Held {
                            columns: columns.clone(),
                            rows: f.rows.clone(),
                            total: f.rows.len(),
                            cursor: has_more.then_some(cur),
                            idle_closed: false,
                            last_used: Instant::now(),
                        },
                    );
                    return Ok(QueryResult { result_id: Some(rid), columns, rows: f.rows, has_more, affected, truncated: f.truncated, ..Default::default() });
                }
            }
        }
        Ok(QueryResult { affected: Some(counts), ..Default::default() })
    }

    /// Pagina successiva.
    pub fn more(&mut self, rid: u64) -> Result<QueryPage, OpErr> {
        let h = self.held.get_mut(&rid).ok_or_else(|| OpErr::msg(EXPIRED))?;
        if h.idle_closed {
            return Err(OpErr::msg(IDLE_CLOSED));
        }
        let Some(cur) = h.cursor.as_mut() else { return Ok(QueryPage::default()) };
        let f = match cur.fetch(PAGE) {
            Ok(f) => f,
            Err(e) => {
                self.held.remove(&rid);
                return Err(e.into());
            }
        };
        h.total += f.rows.len();
        let mut notices = Vec::new();
        let mut has_more = !f.done && f.rows.len() == PAGE;
        if has_more && h.total >= MAX_TOTAL {
            has_more = false;
            notices.push(format!("limite di {MAX_TOTAL} righe raggiunto: le altre non vengono lette"));
        }
        h.rows.extend(f.rows.iter().cloned());
        h.last_used = Instant::now();
        if !has_more {
            h.cursor = None;
        }
        Ok(QueryPage { rows: f.rows, has_more, truncated: f.truncated, notices, ..Default::default() })
    }

    /// Per l'esportazione: colonne e tutte le righe (quelle in copia più le restanti, fino a `MAX_TOTAL`).
    pub fn drain(&mut self, rid: u64) -> Result<Drained, OpErr> {
        let (columns, mut rows) = {
            let h = self.held.get(&rid).ok_or_else(|| OpErr::msg(EXPIRED))?;
            if h.idle_closed {
                return Err(OpErr::msg(IDLE_CLOSED));
            }
            (h.columns.clone(), h.rows.clone())
        };
        loop {
            let page = self.more(rid)?;
            rows.extend(page.rows);
            if !page.has_more {
                return Ok((columns, rows));
            }
        }
    }

    pub fn close(&mut self, rid: u64) {
        self.held.remove(&rid);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Albero e colonne
// ---------------------------------------------------------------------------------------------------------------

/// Proprietari di sistema esclusi dall'albero (oltre a `ORACLE_MAINTAINED = 'Y'` dove la colonna esiste).
pub const SYSTEM_OWNERS: [&str; 16] = [
    "SYS", "SYSTEM", "XDB", "MDSYS", "CTXSYS", "ORDSYS", "ORDDATA", "ORDPLUGINS", "OUTLN", "DBSNMP", "APPQOSSYS", "WMSYS", "GSMADMIN_INTERNAL", "LBACSYS", "OJVMSYS", "DVSYS",
];
const MORE_SYSTEM_OWNERS: [&str; 4] = ["AUDSYS", "SI_INFORMTN_SCHEMA", "DIP", "ANONYMOUS"];

/// Il testo delle query del catalogo per l'albero (solo costanti: i nomi dell'utente non entrano mai qui).
pub fn tree_sql(with_maintained: bool) -> String {
    let owners: Vec<String> = SYSTEM_OWNERS.iter().chain(MORE_SYSTEM_OWNERS.iter()).map(|o| format!("'{o}'")).collect();
    let maintained = if with_maintained { " AND owner NOT IN (SELECT username FROM all_users WHERE oracle_maintained = 'Y')" } else { "" };
    let filter = format!("owner NOT IN ({}){maintained}", owners.join(", "));
    format!(
        "SELECT owner, table_name, 'table' FROM all_tables WHERE {filter} AND nested = 'NO' AND iot_name IS NULL AND table_name NOT LIKE 'BIN$%' \
         UNION ALL SELECT owner, view_name, 'view' FROM all_views WHERE {filter} ORDER BY 1, 2"
    )
}

pub const COLUMNS_SQL: &str = "SELECT c.column_name, c.data_type, c.data_length, c.char_length, c.char_used, c.data_precision, c.data_scale, c.nullable, \
     CASE WHEN EXISTS (SELECT 1 FROM all_constraints k JOIN all_cons_columns cc ON cc.owner = k.owner AND cc.constraint_name = k.constraint_name \
     WHERE k.constraint_type = 'P' AND k.owner = c.owner AND k.table_name = c.table_name AND cc.column_name = c.column_name) THEN 1 ELSE 0 END \
     FROM all_tab_columns c WHERE c.owner = :1 AND c.table_name = :2 ORDER BY c.column_id";

pub const RESOLVE_SQL: &str = "SELECT owner, table_name FROM all_tables WHERE owner = NVL(:1, SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA')) AND table_name = :2";

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OraColumn {
    pub name: String,
    /// Tipo con lunghezza/precisione (`VARCHAR2(30)`, `NUMBER(10,2)`), da mostrare.
    #[serde(rename = "type")]
    pub type_name: String,
    /// `DATA_TYPE` del catalogo (`NUMBER`, `TIMESTAMP(6) WITH TIME ZONE`…): decide la conversione dei parametri.
    #[serde(skip)]
    pub data_type: String,
    pub nullable: bool,
    pub is_pk: bool,
    /// Si modifica in griglia (`ora_edit::bind_expr` ha una conversione per il tipo): lo stesso criterio decide
    /// l'interfaccia (celle in sola lettura) e il core (rifiuto delle modifiche).
    pub editable: bool,
}

/// Il tipo come lo scrive il catalogo, con lunghezza, precisione e scala.
pub fn display_type(data_type: &str, data_length: Option<i64>, char_length: Option<i64>, char_used: Option<&str>, precision: Option<i64>, scale: Option<i64>) -> String {
    match data_type {
        "VARCHAR2" | "NVARCHAR2" | "CHAR" | "NCHAR" => match char_length.filter(|n| *n > 0) {
            Some(n) if char_used == Some("C") && matches!(data_type, "VARCHAR2" | "CHAR") => format!("{data_type}({n} CHAR)"),
            Some(n) => format!("{data_type}({n})"),
            None => data_type.to_string(),
        },
        "RAW" => match data_length.filter(|n| *n > 0) {
            Some(n) => format!("RAW({n})"),
            None => "RAW".into(),
        },
        "NUMBER" => match (precision, scale) {
            (Some(p), Some(s)) if s != 0 => format!("NUMBER({p},{s})"),
            (Some(p), _) => format!("NUMBER({p})"),
            (None, Some(s)) if s != 0 => format!("NUMBER(*,{s})"),
            _ => "NUMBER".into(),
        },
        "FLOAT" => match precision {
            Some(p) => format!("FLOAT({p})"),
            None => "FLOAT".into(),
        },
        other => other.to_string(),
    }
}

/// Proprietari con le loro tabelle e viste (stessa forma di `PgTree`); i proprietari senza oggetti non compaiono.
pub fn build_tree(objects: Vec<(String, String, bool)>) -> PgTree {
    let mut schemas: Vec<PgSchema> = Vec::new();
    for (owner, name, view) in objects {
        if schemas.last().is_none_or(|s| s.name != owner) {
            schemas.push(PgSchema { name: owner, tables: Vec::new() });
        }
        if let Some(s) = schemas.last_mut() {
            s.tables.push(PgTable { name, kind: if view { "view" } else { "table" }.into() });
        }
    }
    PgTree { schemas }
}

// ---------------------------------------------------------------------------------------------------------------
// Lato chiamante (async): invia richieste all'attore
// ---------------------------------------------------------------------------------------------------------------

/// Esegue un testo SQL (una o più istruzioni, divise sui `;` di primo livello). `editableTable` come nella 4a.
pub async fn query(sessions: &OraSessions, id: &str, text: &str) -> Result<QueryResult, String> {
    let stmts = split_statements(text);
    if stmts.is_empty() {
        return Err("Scrivi un'istruzione da eseguire".into());
    }
    let actor = sessions.actor(id)?;
    let editable_table = match plain_table_read(text) {
        Some((owner, table)) => actor.request(|reply| Request::Resolve { owner, table, reply }).await.ok().flatten(),
        None => None,
    };
    let rid = sessions.new_result(id);
    let mut r = actor.request(|reply| Request::Query { rid, stmts, reply }).await?;
    if r.result_id.is_none() {
        sessions.forget_result(rid);
    }
    r.editable_table = editable_table;
    Ok(r)
}

pub async fn more(sessions: &OraSessions, result_id: u64) -> Result<QueryPage, String> {
    let actor = sessions.result_actor(result_id)?;
    actor.request(|reply| Request::More { rid: result_id, reply }).await
}

pub async fn close(sessions: &OraSessions, result_id: u64) -> Result<(), String> {
    let Ok(actor) = sessions.result_actor(result_id) else { return Ok(()) };
    sessions.forget_result(result_id);
    actor.request(|reply| Request::CloseResult { rid: result_id, reply }).await
}

pub async fn drain(sessions: &OraSessions, result_id: u64) -> Result<Drained, String> {
    let actor = sessions.result_actor(result_id)?;
    actor.request(|reply| Request::Drain { rid: result_id, reply }).await
}

pub async fn tree(sessions: &OraSessions, id: &str) -> Result<PgTree, String> {
    let actor = sessions.actor(id)?;
    actor.request(Request::Tree).await
}

pub async fn columns(sessions: &OraSessions, id: &str, owner: &str, table: &str) -> Result<Vec<OraColumn>, String> {
    let actor = sessions.actor(id)?;
    let (owner, table) = (owner.to_string(), table.to_string());
    actor.request(|reply| Request::Columns { owner, table, reply }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stmts(sql: &str) -> Vec<String> {
        split_statements(sql)
    }

    #[test]
    fn splits_on_top_level_semicolons_and_drops_them() {
        assert_eq!(stmts("select 1 from dual; select 2 from dual;"), ["select 1 from dual", "select 2 from dual"]);
        assert_eq!(stmts("  select 1 from dual  ;\n\n;; "), ["select 1 from dual"]);
        assert!(stmts("  -- solo un commento\n").is_empty());
        assert_eq!(stmts("select ';' from dual; select 2 from dual"), ["select ';' from dual", "select 2 from dual"]);
        assert_eq!(stmts("select (1;2) from dual"), ["select (1;2) from dual"]);
    }

    #[test]
    fn q_quotes_hide_semicolons_and_quotes() {
        let a = "select q'[it's; fine]' from dual; select 2 from dual";
        assert_eq!(stmts(a), ["select q'[it's; fine]' from dual", "select 2 from dual"]);
        for (open, close) in [("{", "}"), ("(", ")"), ("<", ">"), ("!", "!"), ("#", "#")] {
            let t = format!("select q'{open}a;'b{close}' from dual; select 2 from dual");
            assert_eq!(stmts(&t).len(), 2, "{open}");
        }
        // maiuscole, prefisso nazionale, non chiusa
        assert_eq!(stmts("select Q'<a;b>', nq'[c;d]', N'e;f' from dual; select 2 from dual").len(), 2);
        assert_eq!(stmts("select q'[a;b from dual; select 2").len(), 1);
        // `q` come identificatore, non come prefisso
        assert_eq!(stmts("select q from t; select 2 from dual").len(), 2);
        // una parola che finisce con q non è un prefisso
        assert_eq!(stmts("select faq'x;y' from dual").len(), 1);
    }

    #[test]
    fn comments_strings_and_quoted_identifiers_hide_semicolons() {
        assert_eq!(stmts("select 1 /* a; b */ from dual; select 2 from dual").len(), 2);
        assert_eq!(stmts("select 1 -- a; b\n from dual; select 2 from dual").len(), 2);
        assert_eq!(stmts("select \"a;b\" from dual; select 2 from dual").len(), 2);
        assert_eq!(stmts("select 'it''s;' from dual; select 2 from dual").len(), 2);
        // i commenti di Oracle non si annidano: il primo `*/` chiude
        assert_eq!(stmts("select 1 /* a /* b */ from dual; select 2 from dual").len(), 2);
        // i commenti ai bordi non entrano nel testo inviato; i suggerimenti dentro l'istruzione sì
        assert_eq!(stmts("/* c */ select /*+ FULL(t) */ 1 from t -- fine"), ["select /*+ FULL(t) */ 1 from t"]);
    }

    #[test]
    fn plsql_blocks_are_one_statement_up_to_a_lone_slash() {
        let anon = "begin\n  update t set a = 1;\n  delete from u where x = 2;\nend;\n/\nselect 1 from dual";
        assert_eq!(stmts(anon), ["begin\n  update t set a = 1;\n  delete from u where x = 2;\nend;", "select 1 from dual"]);
        // fino alla fine del testo, `;` finale conservato
        assert_eq!(stmts("declare x number; begin x := 1; end;"), ["declare x number; begin x := 1; end;"]);
        let proc = "create or replace procedure p is begin null; end;\n/\ncreate table x (a number);";
        assert_eq!(stmts(proc), ["create or replace procedure p is begin null; end;", "create table x (a number)"]);
        for kind in ["function f return number is begin return 1; end;", "package pk is procedure q; end;", "trigger tr before insert on t begin null; end;", "type ty as object (a number);", "editionable procedure p2 is begin null; end;"] {
            let t = format!("create or replace {kind}\n/\nselect 1 from dual");
            assert_eq!(stmts(&t).len(), 2, "{kind}");
        }
        // CREATE TABLE/VIEW non sono blocchi: si dividono sul `;`
        assert_eq!(stmts("create table a (x number); create view v as select 1 x from dual").len(), 2);
        // `/` senza essere sola sulla riga è una divisione, anche in un blocco
        assert_eq!(stmts("begin x := 4 / 2;\n y := 1; end;").len(), 1);
        assert_eq!(stmts("select 4 / 2 from dual; select 1 from dual").len(), 2);
        // `/` sola dopo SQL semplice senza `;` chiude l'istruzione (SQL*Plus)
        assert_eq!(stmts("select 1 from dual\n/\nselect 2 from dual\n/\n"), ["select 1 from dual", "select 2 from dual"]);
        // la `/` dentro commenti e stringhe non conta
        assert_eq!(stmts("begin\n/* c\n/\n*/ null; end;").len(), 1);
        assert_eq!(stmts("begin x := '\n/\n'; end;").len(), 1);
    }

    #[test]
    fn destructive_check_on_oracle_syntax() {
        let reasons = |s: &str| destructive(s).into_iter().map(|f| f.reason).collect::<Vec<_>>();
        assert_eq!(reasons("delete from t"), ["DELETE senza WHERE"]);
        assert_eq!(reasons("DELETE t"), ["DELETE senza WHERE"]);
        assert_eq!(reasons("update t set a = 1"), ["UPDATE senza WHERE"]);
        assert!(reasons("update t set a = 1 where id = 2").is_empty());
        assert!(reasons("delete from t where 1 = 1").is_empty());
        assert_eq!(reasons("drop table t purge"), ["DROP"]);
        assert_eq!(reasons("truncate table t"), ["TRUNCATE"]);
        assert_eq!(reasons("alter table t drop column c"), ["ALTER … DROP"]);
        assert!(reasons("alter table t add c number").is_empty());
        assert_eq!(reasons("purge recyclebin"), ["PURGE"]);
        assert_eq!(reasons("select 1 from dual; delete from t"), ["DELETE senza WHERE"]);
        // stringhe, q-quote, commenti, identificatori citati non contano
        assert!(reasons("select q'[delete from t; drop table x]' from dual").is_empty());
        assert!(reasons("select 'drop table x' from dual -- truncate table y").is_empty());
        assert!(reasons("select \"DROP\" from t").is_empty());
        assert!(reasons("select * from t for update").is_empty());
        assert!(reasons("select 1 from t where a in (select b from u for update)").is_empty());
        // WHERE di una sottoquery non salva il DELETE esterno
        assert_eq!(reasons("delete from t where exists (select 1 from u where u.a = 1) ").len(), 0);
        assert_eq!(reasons("delete from t"), ["DELETE senza WHERE"]);
        // blocchi anonimi: le istruzioni dentro si guardano, i corpi di oggetti creati no
        assert_eq!(reasons("begin delete from t; end;"), ["DELETE senza WHERE"]);
        assert!(reasons("begin delete from t where a = 1; update u set b = 2 where c = 3; end;").is_empty());
        assert_eq!(reasons("begin update u set b = 2; end;"), ["UPDATE senza WHERE"]);
        assert!(reasons("create or replace procedure p is begin delete from t; end;").is_empty());
        // FORALL: l'istruzione segue i limiti
        assert_eq!(reasons("begin forall i in 1..n delete from t; end;"), ["DELETE senza WHERE"]);
        assert_eq!(reasons("declare type a is table of number; v a; begin forall i in indices of v save exceptions update t set x = v(i); end;"), ["UPDATE senza WHERE"]);
        assert!(reasons("begin forall i in 1..n delete from t where id = v(i); end;").is_empty());
        assert!(reasons("begin forall i in 1..n delete from t where id = v(i); select 1 into x from dual for update; end;").is_empty());
        assert!(reasons("create trigger tr after delete on t begin null; end;").is_empty());
        let f = destructive("select 1 from dual; delete from t");
        assert_eq!(f[0].text, "delete from t");
    }

    #[test]
    fn editable_table_parsing() {
        let p = |s: &str| plain_table_read(s);
        assert_eq!(p("select * from emp"), Some((None, "EMP".into())));
        assert_eq!(p("SELECT * FROM scott.emp WHERE sal > 10 ORDER BY ename;"), Some((Some("SCOTT".into()), "EMP".into())));
        assert_eq!(p("select * from \"Scott\".\"Emp Tab\""), Some((Some("Scott".into()), "Emp Tab".into())));
        assert_eq!(p("select * from \"a\"\"b\""), Some((None, "a\"b".into())));
        assert_eq!(p("select * from t offset 10 rows fetch next 5 rows only"), Some((None, "T".into())));
        assert_eq!(p("select * from t fetch first 5 rows only"), Some((None, "T".into())));
        assert_eq!(p("select * from sales$1 where a = q'[x;y]'"), Some((None, "SALES$1".into())));
        for no in [
            "select a from t",
            "select * from t x",
            "select * from t, u",
            "select * from t join u on 1=1",
            "select * from t union select * from u",
            "select * from t minus select * from u",
            "select * from t for update",
            "select * from t where a = 1 group by a",
            "select * from t connect by prior a = b",
            "select * from t sample (10)",
            "select * from (select 1 from dual)",
            "select * from t; select * from u",
            "select * from",
            "update t set a = 1",
            "begin null; end;",
            "select * from \"unclosed",
        ] {
            assert_eq!(p(no), None, "{no}");
        }
        // una sottoquery nel WHERE non cambia la tabella
        assert_eq!(p("select * from t where a in (select b from u join v on 1=1)"), Some((None, "T".into())));
    }

    #[test]
    fn hex_and_truncation() {
        assert_eq!(hex_text(&[0, 0xAB, 0x0F]), ("00AB0F".into(), false));
        let (h, cut) = hex_text(&vec![0xFF; MAX_BIN + 1]);
        assert!(cut && h.len() == MAX_BIN * 2);
        let (h, cut) = hex_text(&vec![0x01; MAX_BIN]);
        assert!(!cut && h.len() == MAX_BIN * 2);
        let (_, cut) = hex_text(&[]);
        assert!(!cut);
    }

    #[test]
    fn limited_reads_and_clob_truncation() {
        let data = vec![b'a'; 20_000];
        let got = read_limited(&mut std::io::Cursor::new(&data), 10_000).unwrap();
        assert_eq!(got.len(), 10_001);
        let got = read_limited(&mut std::io::Cursor::new(&data), 30_000).unwrap();
        assert_eq!(got.len(), 20_000);
        assert!(read_limited(&mut std::io::Cursor::new(Vec::<u8>::new()), 5).unwrap().is_empty());
        // CLOB: oltre 64 KB si taglia (una lettura limitata ne dà `MAX_CELL + 1`)
        let (s, cut) = clob_text(vec![b'x'; MAX_CELL + 1]);
        assert!(cut && s.len() == MAX_CELL);
        let (s, cut) = clob_text(vec![b'x'; MAX_CELL]);
        assert!(!cut && s.len() == MAX_CELL);
        // il taglio non spezza un carattere: «è» = 2 byte, il secondo oltre il limite
        let mut v = vec![b'x'; MAX_CELL - 1];
        v.extend("è".as_bytes());
        v.push(b'y');
        let (s, cut) = clob_text(v);
        assert!(cut && s.len() == MAX_CELL - 1 && s.chars().all(|c| c == 'x'), "{}", s.len());
        // un carattere intero che finisce proprio al limite resta
        let mut v = vec![b'x'; MAX_CELL - 2];
        v.extend("è".as_bytes());
        v.push(b'y');
        let (s, cut) = clob_text(v);
        assert!(cut && s.ends_with('è'));
        assert_eq!(clob_text("àé日本".as_bytes().to_vec()), ("àé日本".into(), false));
    }

    #[test]
    fn number_normalization() {
        assert_eq!(normalize_number(".5"), "0.5");
        assert_eq!(normalize_number("-.5"), "-0.5");
        assert_eq!(normalize_number("12.50"), "12.50");
        assert_eq!(normalize_number("-3"), "-3");
        assert_eq!(normalize_number("1E+40"), "1E+40");
    }

    fn ts(nano: u32, tz_h: i32, tz_m: i32) -> Ts {
        Ts { year: 2024, month: 3, day: 9, hour: 7, minute: 5, second: 1, nano, tz_h, tz_m }
    }

    #[test]
    fn dates_and_timestamps_round_trip_the_session_formats() {
        assert_eq!(format_timestamp(&ts(0, 0, 0), ColKind::Date), "2024-03-09 07:05:01");
        assert_eq!(format_timestamp(&ts(123_456_000, 0, 0), ColKind::Timestamp(6, false)), "2024-03-09 07:05:01.123456");
        assert_eq!(format_timestamp(&ts(120_000_000, 0, 0), ColKind::Timestamp(3, false)), "2024-03-09 07:05:01.120");
        assert_eq!(format_timestamp(&ts(0, 0, 0), ColKind::Timestamp(0, false)), "2024-03-09 07:05:01.0");
        assert_eq!(format_timestamp(&ts(123_456_789, 0, 0), ColKind::Timestamp(9, false)), "2024-03-09 07:05:01.123456789");
        assert_eq!(format_timestamp(&ts(5_000, 2, 0), ColKind::Timestamp(6, true)), "2024-03-09 07:05:01.000005 +02:00");
        assert_eq!(format_timestamp(&ts(0, -5, -30), ColKind::Timestamp(6, true)), "2024-03-09 07:05:01.000000 -05:30");
        assert_eq!(format_timestamp(&ts(0, 0, -30), ColKind::Timestamp(6, true)), "2024-03-09 07:05:01.000000 -00:30");
    }

    #[test]
    fn column_kinds_follow_the_driver_types() {
        use oracle::sql_type::OracleType as T;
        assert_eq!(col_kind(&T::Number(10, 2)), ColKind::Number);
        assert_eq!(col_kind(&T::Float(126)), ColKind::Number);
        assert_eq!(col_kind(&T::Varchar2(10)), ColKind::Text);
        assert_eq!(col_kind(&T::Date), ColKind::Date);
        assert_eq!(col_kind(&T::Timestamp(6)), ColKind::Timestamp(6, false));
        assert_eq!(col_kind(&T::TimestampTZ(3)), ColKind::Timestamp(3, true));
        assert_eq!(col_kind(&T::TimestampLTZ(6)), ColKind::Timestamp(6, true));
        assert_eq!(col_kind(&T::CLOB), ColKind::Clob);
        assert_eq!(col_kind(&T::NCLOB), ColKind::Nclob);
        assert_eq!(col_kind(&T::BLOB), ColKind::Blob);
        assert_eq!(col_kind(&T::Raw(16)), ColKind::Raw);
        assert_eq!(col_kind(&T::LongRaw), ColKind::Raw);
        assert_eq!(col_kind(&T::Long), ColKind::Text);
        assert_eq!(col_kind(&T::Xml), ColKind::Unsupported);
        assert_eq!(col_kind(&T::Json), ColKind::Unsupported);
        assert_eq!(col_kind(&T::BFILE), ColKind::Unsupported);
        assert_eq!(col_kind(&T::Boolean), ColKind::Unsupported);
    }

    #[test]
    fn result_id_ranges_do_not_overlap() {
        assert!(!is_ora_result(1));
        assert!(!is_ora_result(ID_BASE - 1));
        assert!(is_ora_result(ID_BASE));
        assert!(is_ora_result(mongo::ID_BASE - 1));
        assert!(!is_ora_result(mongo::ID_BASE));
        assert!(!is_ora_result(u64::MAX));
        assert!(mongo::is_mongo_result(mongo::ID_BASE) && !mongo::is_mongo_result(ID_BASE));
    }

    #[test]
    fn tree_query_excludes_system_owners_with_constants_only() {
        for with in [false, true] {
            let s = tree_sql(with);
            for o in ["SYS", "SYSTEM", "XDB", "MDSYS", "CTXSYS", "ORDSYS", "OUTLN", "DBSNMP", "APPQOSSYS", "WMSYS", "GSMADMIN_INTERNAL", "LBACSYS", "OJVMSYS", "DVSYS", "AUDSYS"] {
                assert!(s.contains(&format!("'{o}'")), "{o}");
            }
            assert_eq!(s.contains("oracle_maintained = 'Y'"), with);
            assert!(s.contains("all_tables") && s.contains("all_views"));
            assert!(!s.contains(':'), "nessun parametro nel testo dell'albero");
        }
        assert!(COLUMNS_SQL.contains(":1") && COLUMNS_SQL.contains(":2") && RESOLVE_SQL.contains(":2"));
        assert!(RESOLVE_SQL.contains("'USERENV'"), "{RESOLVE_SQL}");
    }

    #[test]
    fn tree_groups_by_owner() {
        let t = build_tree(vec![("A".into(), "T1".into(), false), ("A".into(), "V1".into(), true), ("B".into(), "T2".into(), false)]);
        assert_eq!(t.schemas.len(), 2);
        assert_eq!(t.schemas[0].name, "A");
        assert_eq!(t.schemas[0].tables.len(), 2);
        assert_eq!((t.schemas[0].tables[1].name.as_str(), t.schemas[0].tables[1].kind.as_str()), ("V1", "view"));
        assert_eq!(t.schemas[1].tables[0].kind, "table");
        assert!(build_tree(vec![]).schemas.is_empty());
    }

    #[test]
    fn type_names() {
        let d = display_type;
        assert_eq!(d("VARCHAR2", Some(30), Some(30), Some("B"), None, None), "VARCHAR2(30)");
        assert_eq!(d("VARCHAR2", Some(120), Some(30), Some("C"), None, None), "VARCHAR2(30 CHAR)");
        assert_eq!(d("NVARCHAR2", Some(60), Some(30), Some("C"), None, None), "NVARCHAR2(30)");
        assert_eq!(d("CHAR", Some(5), Some(5), Some("B"), None, None), "CHAR(5)");
        assert_eq!(d("RAW", Some(16), Some(0), None, None, None), "RAW(16)");
        assert_eq!(d("NUMBER", Some(22), None, None, Some(10), Some(2)), "NUMBER(10,2)");
        assert_eq!(d("NUMBER", Some(22), None, None, Some(10), Some(0)), "NUMBER(10)");
        assert_eq!(d("NUMBER", Some(22), None, None, None, None), "NUMBER");
        assert_eq!(d("NUMBER", Some(22), None, None, None, Some(-2)), "NUMBER(*,-2)");
        assert_eq!(d("FLOAT", Some(22), None, None, Some(63), None), "FLOAT(63)");
        assert_eq!(d("DATE", Some(7), None, None, None, None), "DATE");
        assert_eq!(d("TIMESTAMP(6) WITH TIME ZONE", Some(13), None, None, None, None), "TIMESTAMP(6) WITH TIME ZONE");
        assert_eq!(d("CLOB", Some(4000), Some(0), None, None, None), "CLOB");
    }

    #[test]
    fn op_error_texts() {
        let ora = |c, m: &str| OpErr::Db(OraErr { ora: Some(c), dpi: None, message: m.into() });
        assert_eq!(ora(1013, "ORA-01013: user requested cancel").text(), CANCELLED);
        assert_eq!(ora(3113, "x").text(), super::super::pg::LOST);
        assert_eq!(ora(942, "ORA-00942: table or view does not exist\n").text(), "ORA-00942: table or view does not exist");
        assert_eq!(OpErr::msg("ciao").text(), "ciao");
        assert_eq!(ora(942, "ORA-00942").in_statement(1, 3).text(), "Istruzione 2 di 3: ORA-00942");
        assert_eq!(ora(942, "ORA-00942").in_statement(0, 1).text(), "ORA-00942");
        assert_eq!(ora(1013, "x").in_statement(1, 3).text(), CANCELLED);
    }
}
