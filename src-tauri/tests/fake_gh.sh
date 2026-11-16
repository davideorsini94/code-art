#!/bin/sh
# Finto `gh` per i test di src-tauri/src/github.rs. Non usa la rete né la configurazione vera di gh.
# Dati tutti fittizi (account `octo-user` attivo, `octo-work` non attivo).
#
# Variabili d'ambiente:
#   FAKE_GH_LOG    file in cui aggiungere, per ogni chiamata, gli argomenti (uno per riga), la riga
#                  `GH_TOKEN=<valore>` oppure `GH_TOKEN-` se assente, e una riga `--` finale (obbligatoria)
#   FAKE_GH_MODE   bad_json: risposte JSON malformate · fail: errore su stderr (con token e HOME) e uscita 1, salvo i comandi `auth` ·
#                  text_auth: `auth status --json` non supportato (vecchie versioni), stato solo come testo
#   FAKE_GH_STDIN  se impostata, file in cui copiare lo stdin di `pr create`

log="$FAKE_GH_LOG"
for a in "$@"; do printf '%s\n' "$a" >> "$log"; done
if [ -n "${GH_TOKEN+x}" ]; then printf 'GH_TOKEN=%s\n' "$GH_TOKEN" >> "$log"; else printf 'GH_TOKEN-\n' >> "$log"; fi
printf -- '--\n' >> "$log"

if [ "$FAKE_GH_MODE" = "fail" ] && [ "$1" != "auth" ]; then
  echo "HTTP 401: Bad credentials for token ${GH_TOKEN:-nessuno} (config in $HOME/.config/gh/hosts.yml)" >&2
  exit 1
fi

bad() { [ "$FAKE_GH_MODE" = "bad_json" ]; }

case "$1 $2" in
  "auth status")
    case " $* " in
      *" --json "*)
        if [ "$FAKE_GH_MODE" = "text_auth" ]; then echo "unknown flag: --json" >&2; exit 1; fi
        if bad; then echo '{"hosts": [oops'; exit 0; fi
        echo '{"hosts":{"github.com":[{"state":"success","active":true,"host":"github.com","login":"octo-user","tokenSource":"keyring","gitProtocol":"https"},{"state":"success","active":false,"host":"github.com","login":"octo-work","tokenSource":"keyring","gitProtocol":"https"}]}}'
        ;;
      *)
        cat >&2 <<'EOF'
github.com
  ✓ Logged in to github.com account octo-user (keyring)
  - Active account: true
  - Git operations protocol: https

  ✓ Logged in to github.com account octo-work (keyring)
  - Active account: false
  - Git operations protocol: https
EOF
        ;;
    esac
    ;;
  "auth token")
    echo "gho_FAKEtoken0000000000000000000000000"
    ;;
  "repo list")
    if bad; then echo 'not json'; exit 0; fi
    echo '[{"nameWithOwner":"octo-user/demo","description":"Progetto di prova","isPrivate":true,"updatedAt":"2026-11-01T10:00:00Z"},{"nameWithOwner":"octo-user/altro","description":null,"isPrivate":false,"updatedAt":"2026-10-01T10:00:00Z"}]'
    ;;
  "repo create")
    echo "https://github.com/octo-user/demo"
    ;;
  "repo clone")
    mkdir -p "$4"
    ;;
  "pr list")
    if bad; then echo '[{"number": "sette"'; exit 0; fi
    cat <<'EOF'
[{"number":7,"title":"Aggiunge il login","author":{"login":"octo-user","name":"Octo"},"headRefName":"feature/login","baseRefName":"main","state":"OPEN","isDraft":false,"url":"https://github.com/octo-org/demo/pull/7","statusCheckRollup":[{"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://github.com/octo-org/demo/actions/runs/1"},{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"https://github.com/octo-org/demo/actions/runs/2"},{"__typename":"CheckRun","name":"lint","status":"IN_PROGRESS","conclusion":"","detailsUrl":"https://github.com/octo-org/demo/actions/runs/3"},{"__typename":"StatusContext","context":"ci/legacy","state":"SUCCESS","targetUrl":"https://ci.example.invalid/1"}]},{"number":8,"title":"Bozza","author":{"login":"octo-work"},"headRefName":"wip","baseRefName":"main","state":"OPEN","isDraft":true,"url":"https://github.com/octo-org/demo/pull/8","statusCheckRollup":[]}]
EOF
    ;;
  "pr view")
    if bad; then echo '{'; exit 0; fi
    echo '{"number":9,"title":"Nuova funzione","author":{"login":"octo-user"},"headRefName":"feature/x","baseRefName":"main","state":"OPEN","isDraft":true,"url":"https://github.com/octo-org/demo/pull/9","statusCheckRollup":null}'
    ;;
  "pr checks")
    echo '[{"name":"build","state":"SUCCESS","bucket":"pass","link":"https://github.com/octo-org/demo/actions/runs/1"},{"name":"lint","state":"IN_PROGRESS","bucket":"pending","link":"https://github.com/octo-org/demo/actions/runs/3"},{"name":"test","state":"FAILURE","bucket":"fail","link":"https://github.com/octo-org/demo/actions/runs/2"}]'
    # come gh vero: controlli in corso => uscita 8
    exit 8
    ;;
  "pr create")
    if [ -n "$FAKE_GH_STDIN" ]; then cat > "$FAKE_GH_STDIN"; else cat > /dev/null; fi
    echo "https://github.com/octo-org/demo/pull/9"
    ;;
  "pr merge")
    ;;
  *)
    echo "comando finto sconosciuto: $*" >&2
    exit 2
    ;;
esac
exit 0
