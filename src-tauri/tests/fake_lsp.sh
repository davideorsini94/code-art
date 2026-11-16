#!/bin/sh
# Finto language server per i test di src-tauri/src/lsp: legge messaggi `Content-Length` da stdin, risponde a
# `initialize` con un JSON fisso (id 1) e ripete su stdout le notifiche ricevute. Esce con `exit` o a fine input.
#
# Variabili d'ambiente:
#   FAKE_LSP_PIDS  file in cui scrivere il proprio pid e quello di un figlio `sleep` (stesso gruppo di processi)
#   FAKE_LSP_MODE  die: scrive su stderr ed esce subito con 7 · crash: esce con 3 al primo messaggio ·
#                  garbage: risponde al primo messaggio con un header malformato

if [ "$FAKE_LSP_MODE" = "die" ]; then
  echo "avvio non riuscito: configurazione mancante" >&2
  exit 7
fi
if [ -n "$FAKE_LSP_PIDS" ]; then
  sleep 600 &
  printf '%s\n%s\n' "$$" "$!" > "$FAKE_LSP_PIDS"
fi

send() {
  printf 'Content-Length: %s\r\n\r\n%s' "$(printf '%s' "$1" | wc -c | tr -d ' ')" "$1"
}

while :; do
  len=
  while IFS= read -r line; do
    line=$(printf '%s' "$line" | tr -d '\r')
    [ -z "$line" ] && break
    case "$line" in
      Content-Length:*) len=$(echo ${line#Content-Length:}) ;;
    esac
  done
  [ -z "$len" ] && exit 0
  # `dd` a un byte per volta: non consuma l'inizio del messaggio successivo
  body=$(dd bs=1 count="$len" 2>/dev/null)
  case "$FAKE_LSP_MODE" in
    crash) exit 3 ;;
    garbage) printf 'Senza-Due-Punti\r\n\r\n'; continue ;;
  esac
  case "$body" in
    *'"method":"initialize"'*) send '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"textDocumentSync":1},"serverInfo":{"name":"fake-lsp"}}}' ;;
    *'"method":"exit"'*) exit 0 ;;
    *'"method":'*) send "$body" ;;
  esac
done
