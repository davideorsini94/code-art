#!/bin/sh
# Finto `claude` per i test del motore (src-tauri/src/claude/engine.rs). Non usa la rete né ~/.claude.
#
# Variabili d'ambiente:
#   FAKE_CLAUDE_OUT         cartella dove scrivere <sessione>.args, .cwd, .stdin (obbligatoria)
#   FAKE_CLAUDE_FIXTURE     file .jsonl da ristampare dopo ogni messaggio utente ricevuto su stdin
#   FAKE_CLAUDE_CRASH_AFTER se impostata: dopo N righe della fixture scrive su stderr ed esce con 1
#   FAKE_CLAUDE_BARRIER     se impostata a N: prima di ristampare aspetta (max 5 s) che N processi
#                           abbiano ricevuto un messaggio; scrive "ok" o "timeout" in <sessione>.barrier
#
# Una riga `control_request` della fixture sospende la ristampa finché su stdin non arriva una
# `control_response` (come la CLI vera, che aspetta la decisione dell'host).

out="$FAKE_CLAUDE_OUT"
sid=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--session-id" ] || [ "$prev" = "--resume" ]; then sid="$a"; fi
  prev="$a"
done
[ -n "$sid" ] || sid="senza-sessione"

: > "$out/$sid.args"
for a in "$@"; do printf '%s\n' "$a" >> "$out/$sid.args"; done
pwd -P > "$out/$sid.cwd"
: > "$out/$sid.stdin"

wait_barrier() {
  [ -n "$FAKE_CLAUDE_BARRIER" ] || return 0
  : > "$out/$sid.ready"
  i=0
  while [ "$i" -lt 100 ]; do
    n=$(ls "$out" | grep -c '\.ready$')
    if [ "$n" -ge "$FAKE_CLAUDE_BARRIER" ]; then echo ok > "$out/$sid.barrier"; return 0; fi
    sleep 0.05
    i=$((i + 1))
  done
  echo timeout > "$out/$sid.barrier"
}

replay() {
  [ -n "$FAKE_CLAUDE_FIXTURE" ] || return 0
  wait_barrier
  count=0
  while IFS= read -r line <&3; do
    printf '%s\n' "$line"
    count=$((count + 1))
    if [ -n "$FAKE_CLAUDE_CRASH_AFTER" ] && [ "$count" -ge "$FAKE_CLAUDE_CRASH_AFTER" ]; then
      echo "errore finto: connessione persa" >&2
      exit 1
    fi
    case "$line" in
      *'"type":"control_request"'*)
        while IFS= read -r ans; do
          printf '%s\n' "$ans" >> "$out/$sid.stdin"
          case "$ans" in *'"type":"control_response"'*) break ;; esac
        done
        ;;
    esac
  done 3< "$FAKE_CLAUDE_FIXTURE"
}

while IFS= read -r msg; do
  printf '%s\n' "$msg" >> "$out/$sid.stdin"
  case "$msg" in
    *'"type":"user"'*) replay ;;
  esac
done
exit 0
