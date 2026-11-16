//! Cornice dei messaggi LSP su stdio: `Content-Length: N\r\n\r\n` seguito da N byte di JSON.
//! Il decoder rifiuta header oltre 8 KB e contenuti oltre 64 MB: dopo un errore il server va chiuso.

/// Header più lunghi (fino a `\r\n\r\n` compreso) sono un errore.
pub const MAX_HEADER: usize = 8 * 1024;
/// Contenuti più lunghi sono un errore (anche in uscita: `lsp_send` non li accetta).
pub const MAX_CONTENT: usize = 64 * 1024 * 1024;

pub fn encode(json: &[u8]) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", json.len()).into_bytes();
    out.extend_from_slice(json);
    out
}

/// Spezza un flusso di byte in messaggi completi. Dopo un errore resta chiuso (ogni `push` fallisce).
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    /// Lunghezza del contenuto atteso, una volta letto l'header.
    want: Option<usize>,
    failed: bool,
}

impl Decoder {
    /// Aggiunge `bytes` e restituisce i messaggi completati (anche nessuno).
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        if self.failed {
            return Err("flusso LSP già interrotto".into());
        }
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let want = match self.want {
                Some(n) => n,
                None => match self.header() {
                    Ok(Some(n)) => n,
                    Ok(None) => break,
                    Err(e) => {
                        self.failed = true;
                        self.buf = Vec::new();
                        return Err(e);
                    }
                },
            };
            if self.buf.len() < want {
                self.want = Some(want);
                break;
            }
            self.want = None;
            let rest = self.buf.split_off(want);
            out.push(std::mem::replace(&mut self.buf, rest));
        }
        Ok(out)
    }

    /// Legge e toglie dal buffer un header completo: `Ok(None)` se non è ancora arrivato tutto.
    fn header(&mut self) -> Result<Option<usize>, String> {
        let scan = &self.buf[..self.buf.len().min(MAX_HEADER)];
        let Some(end) = scan.windows(4).position(|w| w == b"\r\n\r\n") else {
            return if self.buf.len() >= MAX_HEADER { Err("header LSP oltre 8 KB".into()) } else { Ok(None) };
        };
        let text = std::str::from_utf8(&self.buf[..end]).map_err(|_| "header LSP non ASCII".to_string())?;
        let mut len = None;
        for line in text.split("\r\n") {
            let (name, value) = line.split_once(':').ok_or_else(|| "header LSP malformato".to_string())?;
            if name.trim().eq_ignore_ascii_case("content-length") {
                let v = value.trim();
                // solo cifre: `usize::from_str` accetterebbe anche `+`
                let n = Some(v).filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())).and_then(|v| v.parse::<usize>().ok());
                let n = n.ok_or_else(|| "Content-Length non valido".to_string())?;
                if len.replace(n).is_some() {
                    return Err("Content-Length ripetuto".into());
                }
            }
        }
        let n = len.ok_or_else(|| "Content-Length mancante".to_string())?;
        if n > MAX_CONTENT {
            return Err("messaggio LSP oltre 64 MB".into());
        }
        self.buf.drain(..end + 4);
        Ok(Some(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn encode_adds_content_length() {
        assert_eq!(encode(br#"{"a":1}"#), b"Content-Length: 7\r\n\r\n{\"a\":1}".to_vec());
        // la lunghezza è in byte, non in caratteri
        assert_eq!(&encode("\"è\"".as_bytes())[..18], b"Content-Length: 4\r");
    }

    #[test]
    fn split_message_arrives_when_complete() {
        let bytes = frame(r#"{"jsonrpc":"2.0","method":"x"}"#);
        let mut d = Decoder::default();
        // un byte per volta, header compreso
        let mut got = Vec::new();
        for b in &bytes {
            got.extend(d.push(std::slice::from_ref(b)).unwrap());
        }
        assert_eq!(got, vec![br#"{"jsonrpc":"2.0","method":"x"}"#.to_vec()]);
    }

    #[test]
    fn two_messages_in_one_packet() {
        let mut bytes = frame(r#"{"a":1}"#);
        bytes.extend(b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-length: 7\r\n\r\n{\"b\":2}");
        bytes.extend(&frame(r#"{"c":3}"#)[..10]);
        let mut d = Decoder::default();
        assert_eq!(d.push(&bytes).unwrap(), vec![br#"{"a":1}"#.to_vec(), br#"{"b":2}"#.to_vec()]);
        let rest = frame(r#"{"c":3}"#);
        assert_eq!(d.push(&rest[10..]).unwrap(), vec![br#"{"c":3}"#.to_vec()]);
        assert!(d.push(b"").unwrap().is_empty());
    }

    #[test]
    fn malformed_header_is_an_error() {
        for bad in [
            &b"Senza-Due-Punti\r\n\r\n{}"[..],
            b"Content-Length: dieci\r\n\r\n{}",
            b"Content-Length: -1\r\n\r\n{}",
            b"Content-Type: text/plain\r\n\r\n{}",
            b"Content-Length: 2\r\nContent-Length: 3\r\n\r\n{}",
        ] {
            let mut d = Decoder::default();
            assert!(d.push(bad).is_err(), "{}", String::from_utf8_lossy(bad));
            // dopo l'errore il decoder resta chiuso
            assert!(d.push(&frame("{}")).is_err());
        }
    }

    #[test]
    fn oversized_header_or_content_is_an_error() {
        let mut d = Decoder::default();
        let long = format!("X-Lungo: {}", "a".repeat(MAX_HEADER));
        assert!(d.push(long.as_bytes()).is_err(), "header senza fine oltre 8 KB");
        let mut d = Decoder::default();
        assert!(d.push(format!("Content-Length: {}\r\n\r\n", MAX_CONTENT + 1).as_bytes()).is_err());
        let mut d = Decoder::default();
        assert!(d.push(format!("Content-Length: {MAX_CONTENT}\r\n\r\n").as_bytes()).unwrap().is_empty(), "64 MB esatti ammessi");
    }
}
