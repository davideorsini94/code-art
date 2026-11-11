# Protocollo Claude Code CLI — esito spike (Task 4)

Verificato su `claude` **2.1.319** (macOS), 2026-11-11. Tutti gli esempi JSON sono righe reali catturate
(path redatti: home → `/Users/dev`, cartella di prova → `/Users/dev/proj`; firme/blob accorciati).
Le fixture complete sono in `src-tauri/tests/fixtures/stream/*.jsonl` (solo stdout, una riga JSON per riga).

---

## 0. Verdetto (Step 3): **motore CLI diretto**

Richieste di permesso, domande (`AskUserQuestion`), approvazione piano (`ExitPlanMode`) e interruzione
sono **tutte** realizzabili via stdin/stdout del processo `claude`, senza sidecar Node:

| Requisito | Come | Verificato |
|---|---|---|
| Richiesta di permesso | stdout `control_request` `subtype:"can_use_tool"` → stdin `control_response` `behavior:"allow"\|"deny"` | sì (consenti, nega, consenti+regola persistente) — `permission-request.jsonl` |
| Domanda all'utente | stesso `can_use_tool` con `tool_name:"AskUserQuestion"`; la risposta è `allow` con `updatedInput.answers` | sì — `ask-user-question.jsonl` |
| Approvazione piano | stesso `can_use_tool` con `tool_name:"ExitPlanMode"`; `allow` = approva | sì (approvazione) — `plan-mode.jsonl` |
| Interruzione | stdin `control_request` `{"subtype":"interrupt"}` | sì, anche con permesso pendente — `interrupt.jsonl`, `control-misc.jsonl` |
| Cambio modello / modalità a processo vivo | stdin `control_request` `set_model` / `set_permission_mode` | sì — `model-switch.jsonl` |

**Condizione critica:** serve il flag **non documentato in `--help`** `--permission-prompt-tool stdio`.
Con solo `--permission-prompts host` (il valore di default) la CLI **non** emette `can_use_tool`:
ogni azione che chiederebbe permesso viene negata automaticamente (`system/permission_denied`,
`permission_decision.source:"config"`), e i tool `AskUserQuestion`, `ExitPlanMode`, `EnterPlanMode`
spariscono dall'elenco strumenti. Con `--permission-prompt-tool stdio` compaiono e le richieste arrivano all'host.

### Comando da lanciare (Task 6–7)

```
claude -p \
  --input-format stream-json --output-format stream-json --verbose \
  --include-partial-messages \
  --permission-prompts host --permission-prompt-tool stdio \
  --permission-mode default \
  [--model <alias|id>] [--resume <sessionId> | --session-id <uuid>] [--add-dir ...]
```

- cwd del processo = cartella del progetto.
- Il processo resta vivo tra un turno e l'altro finché stdin è aperto; chiudere stdin = terminare dopo il turno in corso.
- `--permission-mode` va **sempre** passato esplicitamente: senza, si eredita il default dell'utente
  (in questa macchina era `auto`, visto nelle righe di sessione). `--help` elenca
  `acceptEdits, auto, bypassPermissions, manual, dontAsk, plan`; `default` è accettato e riportato come `"default"`.
- `--replay-user-messages` (opzionale) fa riemettere su stdout ogni messaggio utente con `isReplay:true` e il suo `uuid` (vedi §2.3).
- `--safe-mode` è stato usato nello spike solo per togliere hook/plugin/MCP dell'utente dalle catture; il prodotto NON deve usarlo.

---

## 1. Panoramica delle righe

Ogni riga stdout è un oggetto JSON con `type`. Righe osservate:

| `type` | `subtype` / note | Uso |
|---|---|---|
| `system` | `init` | inizio di **ogni** turno (non all'avvio del processo!) |
| `system` | `status` | `status:"requesting"` (chiamata API in partenza), `"compacting"`, `null` + `compact_result`, `null` + `permissionMode` (cambio modalità) |
| `system` | `ui_invalidate`, `task_summary`, `post_turn_summary`, `thinking_tokens`, `rate_limit`… | informative, ignorabili |
| `system` | `permission_denied` | negazione automatica (senza prompt) |
| `system` | `task_started`, `task_progress`, `task_updated`, `task_notification`, `background_tasks_changed` | ciclo di vita subagent |
| `system` | `compact_boundary` | dopo `/compact` (o auto-compact) |
| `system` | `session_title_changed` | `--name` o `/rename` |
| `system` | `hook_started`, `hook_response` | hook utente (es. SessionStart) — solo senza `--safe-mode` |
| `stream_event` | `event.type` = eventi SSE dell'API Messages | testo/thinking/tool input parziali (solo con `--include-partial-messages`) |
| `assistant` | | messaggio assistente completo, **un content block per riga** |
| `user` | | risultati strumenti, marker di interruzione, output comandi locali, replay |
| `result` | `success` / `error_during_execution` / … | **fine turno** |
| `control_request` | `request.subtype:"can_use_tool"` | CLI → host: chiede decisione |
| `control_cancel_request` | | CLI → host: annulla una `control_request` pendente |
| `control_response` | | risposta della CLI a una `control_request` dell'host |
| `rate_limit_event` | | stato limiti abbonamento |

**Regola per il parser:** qualunque `type` o `subtype` sconosciuto va ignorato senza errore
(fixture `unknown.jsonl` contiene la riga inventata `{"type":"future-thing"}` subito dopo `init`).
Tutte le righe, tranne `control_*`, portano `session_id`; quasi tutte un `uuid`.

---

## 2. Avvio

### 2.1 Handshake opzionale `initialize` (consigliato)

La CLI non scrive nulla finché non riceve qualcosa su stdin. Per avere subito comandi slash, modelli e
modalità senza consumare un turno, l'host invia:

```json
{"type":"control_request","request_id":"host_req_1","request":{"subtype":"initialize","hooks":null}}
```

Risposta (array accorciati):

```json
{"type":"control_response","response":{"subtype":"success","request_id":"host_req_1","response":{
  "commands":[{"name":"design","description":"Make a new Design artifact from a brief","argumentHint":"[what to design]","builtin":true}, "..."],
  "agents":[{"name":"general-purpose","description":"..."}],
  "output_style":"default","available_output_styles":["default","Proactive","Concise","Explanatory","Learning"],
  "models":[{"value":"default","resolvedModel":"claude-fable-5-2","displayName":"Default (recommended)","description":"Fable 5.2","supportsEffort":true,"supportedEffortLevels":["low","medium","high","xhigh","max"],"supportsAdaptiveThinking":true,"supportsAutoMode":true,"supportsThinkingOff":false},
            {"value":"opus","resolvedModel":"claude-opus-5-5","displayName":"Opus 5.5","description":"Best for everyday, complex tasks", "...":"..."}],
  "account":{"email":"dev@example.com","organization":"Example Org","subscriptionType":"Claude Team","apiProvider":"firstParty"},
  "pid":46284,"claude_code_version":"2.1.319","current_permission_mode":"default",
  "session_state":"idle","fast_mode_state":"off","capabilities":["ui_surface_v1","mcp_call_result_not_for_model_v1","mcp_call_check_permissions_v1"]}}}
```

Nota: la risposta **non** contiene `session_id`; quello arriva con `system/init`.

### 2.2 Riga `system/init` (emessa all'inizio di ogni turno)

```json
{"type":"system","subtype":"init","cwd":"/Users/dev/proj","session_id":"4cd0c68d-c7ec-49b7-a475-d1fe8b87e01a",
 "tools":["Task","Artifact","AskUserQuestion","Bash","Edit","ExitPlanMode","Read","Write","..."],
 "mcp_servers":[{"name":"claude.ai Slack","status":"needs-auth","source":"claudeai"}],
 "model":"claude-haiku-5-5","permissionMode":"default",
 "slash_commands":["deep-research","design","compact","clear","model","..."],
 "terminal_slash_commands":["doctor","send-to-project","color","focus","reload-plugins"],
 "apiKeySource":"none","claude_code_version":"2.1.319","output_style":"default",
 "agents":["claude","claude-code-guide","Explore","general-purpose","Plan"],
 "skills":["deep-research","design","..."],
 "plugins":[{"name":"superpowers","path":"/Users/dev/.claude/plugins/cache/...","source":"superpowers@claude-plugins-official","version":"7.0.0"}],
 "capabilities":["interrupt_receipt_v1","interrupt_cancel_queued_v1","..."],
 "uuid":"7cd42f9b-...","scratchpad_path":"/tmp/claude-dev/-Users-dev-proj/<sid>/scratchpad","original_cwd":"/Users/dev/proj",
 "additional_directories":[],"fast_mode_state":"off","startup_timing":{"...":"..."}}
```

- `session_id` → id sessione (= nome del file `.jsonl` su disco). Resta lo stesso per tutta la vita del processo.
- `model` → id completo del modello corrente (dopo `set_model` il turno successivo riporta il nuovo).
- `permissionMode` → modalità corrente.
- `slash_commands` → elenco comandi slash (nomi senza `/`); descrizioni solo in `initialize.commands`.
- `tools` → strumenti caricati. Nota: il subagent appare come `"Task"` in `tools` ma i `tool_use` hanno `name:"Agent"`.
  Alcuni strumenti sono "differiti" (caricati dal modello via `ToolSearch`, es. `ExitPlanMode` in plan mode).
  `TaskCreate/TaskUpdate/TaskList/TaskGet` non sono nel set di default (vedi §6).

### 2.3 Messaggi utente in ingresso (stdin)

Testo:

```json
{"type":"user","session_id":"","parent_tool_use_id":null,"message":{"role":"user","content":"Reply with exactly: hello"}}
```

Testo + immagine base64 (content come array di blocchi API Messages):

```json
{"type":"user","session_id":"","parent_tool_use_id":null,"message":{"role":"user","content":[
  {"type":"text","text":"What single color is this image? One word."},
  {"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2..."}}]}}
```

- `session_id` può essere `""`; `parent_tool_use_id` `null`.
- Un comando slash si invia come testo normale: `"content":"/compact"`.
- Senza `--replay-user-messages` il messaggio utente NON viene riemesso; con il flag arriva:

```json
{"type":"user","message":{"role":"user","content":"Reply with exactly: hi"},"session_id":"701e017c-...","parent_tool_use_id":null,"uuid":"ca157b7d-6840-4028-b933-d41c4c97a4a7","timestamp":"2026-11-11T09:15:29.578Z","isReplay":true}
```

---

## 3. Streaming della risposta

Ordine tipico di un turno testuale (`simple-turn.jsonl`):
`system/init` → `system/status requesting` → `stream_event message_start` → `content_block_start` →
`content_block_delta`… → **`assistant`** → `content_block_stop` → `message_delta` → `message_stop` →
`rate_limit_event` → `system/post_turn_summary` → **`result`**.

### 3.1 Testo parziale

```json
{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"session_id":"...","parent_tool_use_id":null,"uuid":"...","api_message_id":"msg_011Cgvt42XLhwj4Q1gFpFf3K"}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}},"session_id":"...","parent_tool_use_id":null,"uuid":"...","api_message_id":"msg_011Cgvt42XLhwj4Q1gFpFf3K"}
```

Poi il blocco completo arriva come riga `assistant` (prima di `content_block_stop`):

```json
{"type":"assistant","message":{"model":"claude-haiku-5-5","id":"msg_011Cgvt42XLhwj4Q1gFpFf3K","type":"message","role":"assistant","content":[{"type":"text","text":"hello"}],"stop_reason":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":16585,"cache_read_input_tokens":24633,"output_tokens":4,"...":"..."}},"parent_tool_use_id":null,"session_id":"...","uuid":"69b2f6bb-...","timestamp":"2026-11-11T09:10:51.966Z","request_id":"req_011Cgvt4254NgdxEGT6Q3ZX9"}
```

Regole:
- **Una riga `assistant` per content block**: un messaggio API con thinking+testo o con 2 tool_use produce
  2 righe `assistant` con lo **stesso `message.id`**, ognuna con `content` di un solo elemento. Il reducer deve
  accumulare per `message.id` (o semplicemente appendere i blocchi in ordine).
- `message.stop_reason` nelle righe `assistant` è sempre `null`; lo stop reason vero è in
  `stream_event message_delta.delta.stop_reason` (`end_turn`, `tool_use`, …) e in `result.stop_reason`.
- I delta hanno `api_message_id` = `message.id` e `event.index` = indice del blocco.
- La riga `assistant` può sostituire il testo accumulato dai delta (è autorevole).

### 3.2 Thinking

```json
{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}},"...":"..."}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"","estimated_tokens":null}},"...":"..."}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"CAQSnAYKEAgTGAI4..."}},"...":"..."}
{"type":"assistant","message":{"id":"msg_011CgvtPbnNZLQQ99RNzanmQ","content":[{"type":"thinking","thinking":"","signature":"CAQSnAYKEAgTGAI4...TRIMMED"}],"...":"..."},"...":"..."}
{"type":"system","subtype":"thinking_tokens","estimated_tokens":300,"estimated_tokens_delta":250,"session_id":"...","uuid":"..."}
```

Osservato: in questa versione il testo del thinking arriva **vuoto** (`thinking:""`, `message_start` ha
`"thinking_display":"updates"`), anche con `--settings '{"showThinkingSummaries":true}'` (provato su haiku e sonnet).
La UI deve mostrare un indicatore "sta pensando" (eventualmente con `system/thinking_tokens.estimated_tokens`)
e mostrare il testo solo se non vuoto. `usage.output_tokens_details.thinking_tokens` conta i token di thinking.

### 3.3 Uso strumento (input parziale) e risultato

```json
{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01BSyVhT7F8ixLPnU6rqA9U8","name":"Read","input":{},"caller":{"type":"direct"}}},"...":"..."}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\""}},"...":"..."}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"fi"}},"...":"..."}
{"type":"assistant","message":{"id":"msg_011CgvtFa5TtZtohehBHZsfj","content":[{"type":"tool_use","id":"toolu_01BSyVhT7F8ixLPnU6rqA9U8","name":"Read","input":{"file_path":"/Users/dev/proj/a.txt"},"caller":{"type":"direct"}}],"...":"..."},"parent_tool_use_id":null,"wire_tool_inputs":{"toolu_01BSyVhT7F8ixLPnU6rqA9U8":{"file_path":"/Users/dev/proj/a.txt"}},"...":"..."}
```

Il risultato arriva come riga `user` con blocco `tool_result` (legato da `tool_use_id`) più
`tool_use_result` (oggetto strutturato, forma dipendente dallo strumento):

```json
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_01BSyVhT7F8ixLPnU6rqA9U8","type":"tool_result","content":"1\tcolor = red\n2\tsize = 3\n3\t"}]},"parent_tool_use_id":null,"session_id":"...","uuid":"73128772-...","timestamp":"2026-11-11T09:13:29.074Z",
 "tool_use_result":{"type":"text","file":{"filePath":"/Users/dev/proj/a.txt","content":"color = red\nsize = 3\n","numLines":3,"startLine":1,"totalLines":3}},
 "tool_result_meta":[{"id":"toolu_01BSyVhT7F8ixLPnU6rqA9U8","permission_decision":{"decision":"accept","source":"config","reason_type":"mode"}}]}
```

Edit (`tool-use-edit.jsonl`, `--permission-mode acceptEdits`):

```json
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_01VuZipZavrS85v9SSCERMpa","type":"tool_result","content":"The file /Users/dev/proj/a.txt has been updated successfully. ..."}]},"parent_tool_use_id":null,
 "tool_use_result":{"filePath":"/Users/dev/proj/a.txt","oldString":"color = red","newString":"color = blue","originalFile":"color = red\nsize = 3\n",
   "structuredPatch":[{"oldStart":1,"oldLines":2,"newStart":1,"newLines":2,"lines":["-color = red","+color = blue"," size = 3"]}],"userModified":false,"replaceAll":false},"...":"..."}
```

Errori: `tool_result.is_error:true` e `tool_use_result` è una stringa `"Error: ..."`.
`tool_result.content` può essere stringa o array di blocchi (`text`, `image`, `tool_reference`).

---

## 4. Subagent

Fixture `subagent.jsonl`. Il padre emette un `tool_use` con `name:"Agent"`:

```json
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01KMD62ujTvgGAKrRd1YaE9g","name":"Agent","input":{"description":"Read first line of a.txt","subagent_type":"general-purpose","prompt":"Read the file a.txt ..."}}],"...":"..."},"parent_tool_use_id":null,"...":"..."}
{"type":"system","subtype":"task_started","task_id":"af18de2de558b3e09","run_id":"0mwbwk4er-5ef0025b","tool_use_id":"toolu_01KMD62ujTvgGAKrRd1YaE9g","description":"Read first line of a.txt","subagent_type":"general-purpose","is_backgrounded":true,"spawn_depth":1,"task_type":"local_agent","prompt":"...","uuid":"...","session_id":"..."}
```

**Campo che lega allo strumento padre: `parent_tool_use_id`** = `id` del `tool_use` `Agent`.
Tutte le righe del subagent (`assistant`, `user`) lo portano, più `agent_id`, `subagent_type`, `task_description`:

```json
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01Uq8FRAeGHm9yPae6e29WyP","name":"Read","input":{"file_path":"/Users/dev/proj/a.txt"}}],"...":"..."},"parent_tool_use_id":"toolu_01KMD62ujTvgGAKrRd1YaE9g","agent_id":"af18de2de558b3e09","subagent_type":"general-purpose","task_description":"Read first line of a.txt","...":"..."}
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_01Uq8FRAeGHm9yPae6e29WyP","type":"tool_result","content":"1\tcolor = blue\n2\tsize = 3\n3\t"}]},"parent_tool_use_id":"toolu_01KMD62ujTvgGAKrRd1YaE9g","agent_id":"af18de2de558b3e09","...":"..."}
{"type":"assistant","message":{"content":[{"type":"text","text":"color = blue"}],"...":"..."},"parent_tool_use_id":"toolu_01KMD62ujTvgGAKrRd1YaE9g","agent_id":"af18de2de558b3e09","...":"..."}
{"type":"system","subtype":"task_progress","task_id":"af18de2de558b3e09","tool_use_id":"toolu_01KMD62ujTvgGAKrRd1YaE9g","description":"Reading a.txt","usage":{"total_tokens":23187,"tool_uses":1,"duration_ms":1230},"last_tool_name":"Read","...":"..."}
{"type":"system","subtype":"task_updated","task_id":"af18de2de558b3e09","patch":{"status":"completed","end_time":1794388461446},"...":"..."}
{"type":"system","subtype":"task_notification","task_id":"af18de2de558b3e09","tool_use_id":"toolu_01KMD62ujTvgGAKrRd1YaE9g","status":"completed","output_file":"/tmp/.../tasks/af18de2de558b3e09.output","summary":"color = blue","usage":{"total_tokens":24486,"tool_uses":1,"duration_ms":2108},"...":"..."}
```

- Gli `stream_event` parziali del subagent NON sono stati emessi (solo le righe complete); i delta osservati hanno tutti `parent_tool_use_id:null`.
  Esiste `--forward-subagent-text` per inoltrare testo/thinking del subagent (non necessario: il testo finale arrivava già).
- **Subagent in background (default osservato):** il `tool_result` del padre arriva subito
  (`tool_use_result.status:"async_launched"`), il turno finisce con un `result`, e quando il subagent termina la CLI
  **avvia da sola un nuovo turno** (`system/init` → … → secondo `result`) senza input dell'host.
  Quindi: il motore non deve assumere "un `result` per ogni messaggio inviato"; può arrivare un `result` non richiesto.
  `result.subagent_stats` riassume (spawned, completed, ...).

---

## 5. Permessi, domande, piano

### 5.1 Richiesta di permesso (CLI → host)

```json
{"type":"control_request","request_id":"c6b1b64f-8800-4372-b30a-5b4c4e3c465b","request":{"subtype":"can_use_tool","tool_name":"Write","display_name":"Write",
 "input":{"file_path":"/Users/dev/proj/note.txt","content":"hi"},"description":"note.txt",
 "permission_suggestions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}],
 "tool_use_id":"toolu_01JUeD5UKKsKXWrNF2veyNkt"}}
```

Bash:

```json
{"type":"control_request","request_id":"0477865f-c919-47c1-8f1e-8c069e660596","request":{"subtype":"can_use_tool","tool_name":"Bash","display_name":"Bash",
 "input":{"command":"touch denied.txt","description":"Create empty file denied.txt"},"description":"Create empty file denied.txt",
 "permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"touch denied.txt"}],"behavior":"allow","destination":"localSettings"},
                           {"type":"addDirectories","directories":["/Users/dev/proj"],"destination":"session"},
                           {"type":"setMode","mode":"acceptEdits","destination":"session"}],
 "blocked_path":"/Users/dev/proj/denied.txt","tool_use_id":"toolu_0184XSKzVR8BdXuQvMkh94qS"}}
```

`tool_use_id` lega la richiesta al blocco `tool_use` già emesso nella riga `assistant`.
Il turno resta fermo finché l'host non risponde (nessun timeout osservato).

### 5.2 Risposte (host → stdin)

Consenti (una volta):

```json
{"type":"control_response","response":{"subtype":"success","request_id":"c6b1b64f-8800-4372-b30a-5b4c4e3c465b","response":{"behavior":"allow","updatedInput":{"file_path":"/Users/dev/proj/note.txt","content":"hi"}}}}
```

Nega:

```json
{"type":"control_response","response":{"subtype":"success","request_id":"0477865f-c919-47c1-8f1e-8c069e660596","response":{"behavior":"deny","message":"The user denied this action."}}}
```

Consenti sempre (applica i suggerimenti ricevuti):

```json
{"type":"control_response","response":{"subtype":"success","request_id":"089660b2-...","response":{"behavior":"allow","updatedInput":{"command":"mkdir -p sub","description":"..."},
 "updatedPermissions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"mkdir -p sub"}],"behavior":"allow","destination":"localSettings"}, "..."]}}}
```

- `updatedInput` è obbligatorio con `allow` (passare `input` invariato o modificato).
- `updatedPermissions` con `destination:"localSettings"` **scrive su disco** `<cwd>/.claude/settings.local.json`
  (verificato: `{"permissions":{"allow":["Bash(mkdir -p sub)"]}}`); `destination:"session"` vale solo per il processo.
- Effetto a valle di un `deny`: riga `user` con `tool_result` `is_error:true` e testo = `message`; il modello continua il turno.
  Il `result` finale elenca `permission_denials:[{tool_name,tool_use_id,tool_input}]`.
- Approvazione registrata nel `tool_result_meta`: `"permission_decision":{"decision":"accept","source":"user_temporary"}`.

### 5.3 Domanda (`AskUserQuestion`)

```json
{"type":"control_request","request_id":"abdb04aa-2c5d-470e-a621-8479e649e260","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","display_name":"AskUserQuestion",
 "input":{"questions":[{"question":"Which color do you prefer?","header":"Color","options":[{"label":"Red","description":"Choose red as your preferred color."},{"label":"Blue","description":"Choose blue as your preferred color."}],"multiSelect":false}]},
 "tool_use_id":"toolu_011AM33evTo74WPf1WReZHDf","requires_user_interaction":true}}
```

Risposta: `allow` con `updatedInput` = input originale + `answers` (mappa testo-domanda → label scelta):

```json
{"type":"control_response","response":{"subtype":"success","request_id":"abdb04aa-2c5d-470e-a621-8479e649e260","response":{"behavior":"allow","updatedInput":{
 "questions":[{"question":"Which color do you prefer?","header":"Color","options":[{"label":"Red","description":"..."},{"label":"Blue","description":"..."}],"multiSelect":false}],
 "answers":{"Which color do you prefer?":"Blue"}}}}}
```

Esito: `tool_result` `"Your questions have been answered: \"Which color do you prefer?\"=\"Blue\". ..."` e
`tool_use_result:{"questions":[...],"answers":{"Which color do you prefer?":"Blue"}}`.

UNVERIFIED: formato per `multiSelect:true` (dalla documentazione SDK: label multiple unite da `", "` nella stessa stringa)
e risposta libera "Other" (stringa arbitraria al posto della label). Annullare la domanda = `deny` con `message`.

### 5.4 Plan mode e approvazione piano (`ExitPlanMode`)

Avvio con `--permission-mode plan` (o `set_permission_mode` `"plan"`). Il modello scrive il piano in
`~/.claude/plans/<slug>.md` (lo fa la CLI, consentito in plan mode) e poi chiama `ExitPlanMode`:

```json
{"type":"control_request","request_id":"d60aae2d-a563-4e2e-b77b-066109758496","request":{"subtype":"can_use_tool","tool_name":"ExitPlanMode","display_name":"ExitPlanMode",
 "input":{"plan":"# Plan: create hello.py\n\n- Create `hello.py` ...\n- Verify by running `python3 hello.py` ...\n","planFilePath":"/Users/dev/.claude/plans/make-a-very-short-goofy-scott.md"},
 "tool_use_id":"toolu_01Aq5xRoBmsmKgwRrRAgtedS","requires_user_interaction":true}}
```

Approva = `allow` con `updatedInput` = input. Subito dopo la CLI emette il cambio modalità e il tool_result:

```json
{"type":"system","subtype":"status","status":null,"permissionMode":"default","uuid":"...","session_id":"..."}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"User has approved your plan. You can now start coding. ...","tool_use_id":"toolu_01Aq5xRoBmsmKgwRrRAgtedS"}]},"tool_use_result":{"plan":"# Plan: ...","isAgent":false,"filePath":"/Users/dev/.claude/plans/make-a-very-short-goofy-scott.md"},"...":"..."}
```

Dopo l'approvazione la modalità torna a `default` (quindi le scritture successive chiedono di nuovo permesso —
visto nella fixture). Per approvare "e accetta modifiche" aggiungere
`"updatedPermissions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}]` (UNVERIFIED, forma dai suggerimenti osservati).
Rifiuto del piano = `deny` con `message` (feedback per il modello): UNVERIFIED in questa versione, ma è lo stesso
meccanismo `deny` verificato per gli altri strumenti; ci si aspetta che il modello resti in plan mode.

### 5.5 Annullamento lato CLI

Se il turno viene interrotto mentre una richiesta è pendente, la CLI invia:

```json
{"type":"control_cancel_request","request_id":"8cb99aec-2f6c-4455-8e23-32e3cdc26490"}
```

L'host deve chiudere il dialogo corrispondente e **non** rispondere più a quel `request_id` (`control-misc.jsonl`).

---

## 6. Todo (Task tools)

In 2.1.319 **non esiste `TodoWrite`**. La lista attività usa `TaskCreate`/`TaskUpdate`/`TaskList`/`TaskGet`, che
**non sono nel set di strumenti di default** in modalità `-p` (non compaiono in `init.tools`, neanche come differiti
verificati). Nella fixture sono stati abilitati con `--tools "TaskCreate,TaskUpdate,TaskList,TaskGet,Read"`.
Non c'è un evento aggregato "lista todo": la UI ricostruisce lo stato dai tool_use/tool_result:

```json
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01KkQVG9hSqnCpPK1sXjEgta","name":"TaskCreate","input":{"subject":"Write intro","description":"Write intro"}}],"...":"..."},"...":"..."}
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_01KkQVG9hSqnCpPK1sXjEgta","type":"tool_result","content":"Task #1 created successfully: Write intro"}]},"tool_use_result":{"task":{"id":"1","subject":"Write intro","description":"Write intro"}},"...":"..."}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01KLWjiJeovz5XH2p4TxurTK","name":"TaskUpdate","input":{"taskId":"1","status":"in_progress"}}],"...":"..."},"...":"..."}
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_01KLWjiJeovz5XH2p4TxurTK","type":"tool_result","content":"Updated task #1 status"}]},"tool_use_result":{"success":true,"taskId":"1","updatedFields":["status"],"statusChange":{"from":"pending","to":"in_progress"}},"...":"..."}
```

Stati osservati: `pending` → `in_progress` → `completed`. Decisione per il Task 6: se si vuole la todo list,
`--tools` deve elencare **esplicitamente tutti** gli strumenti voluti più i Task tools. Verificato: `--tools "default,TaskCreate,TaskUpdate"`
NON somma al set di default, produce `tools:["TaskCreate","TaskUpdate"]` soltanto.

---

## 7. Interruzione

Host → stdin:

```json
{"type":"control_request","request_id":"host_req_1","request":{"subtype":"interrupt"}}
```

Sequenza osservata (`interrupt.jsonl`):

```json
{"type":"control_response","response":{"subtype":"success","request_id":"host_req_1","response":{"still_queued":[]}}}
{"type":"assistant","message":{"id":"msg_011CgvtLMTpcKMUa7USP8MvP","content":[{"type":"text","text":"1\n2\n...\n36"}],"...":"..."},"aborted":true,"...":"..."}
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]},"parent_tool_use_id":null,"uuid":"5cfd5d92-...","timestamp":"2026-11-11T09:14:33.804Z"}
{"type":"stream_event","event":{"type":"content_block_stop","index":0},"...":"..."}
{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":null,"...":"..."},"...":"..."},"...":"..."}
{"type":"stream_event","event":{"type":"message_stop"},"...":"..."}
{"type":"result","subtype":"error_during_execution","is_error":true,"result":null,"stop_reason":null,"terminal_reason":"aborted_streaming","num_turns":2,"errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=null"],"...":"..."}
```

- Il processo resta vivo: il messaggio successivo ("still alive") è stato gestito normalmente.
- Interruzione durante un permesso pendente: `control_cancel_request`, poi `tool_result` di rifiuto
  (`"The user doesn't want to proceed with this tool use..."`), `user` `"[Request interrupted by user for tool use]"`,
  `result` `error_during_execution` con `terminal_reason:"aborted_tools"`.
- `still_queued` elenca eventuali messaggi utente in coda non ancora presi.
- La riga `assistant` parziale ha `"aborted":true`.

---

## 8. Cambio modello e modalità a processo vivo (messaggi di controllo — niente riavvio)

```json
{"type":"control_request","request_id":"host_req_1","request":{"subtype":"set_model","model":"sonnet"}}
```

Risposte (`model-switch.jsonl`):

```json
{"type":"user","message":{"role":"user","content":"<local-command-stdout>Set model to `sonnet (claude-sonnet-5-5)`</local-command-stdout>"},"session_id":"...","parent_tool_use_id":null,"uuid":"...","timestamp":"...","isReplay":true}
{"type":"control_response","response":{"subtype":"success","request_id":"host_req_1","response":{"model":"claude-sonnet-5-5"}}}
```

```json
{"type":"control_request","request_id":"host_req_2","request":{"subtype":"set_permission_mode","mode":"acceptEdits"}}
```

```json
{"type":"control_response","response":{"subtype":"success","request_id":"host_req_2","response":{"mode":"acceptEdits"}}}
{"type":"system","subtype":"status","status":null,"permissionMode":"acceptEdits","uuid":"...","session_id":"..."}
```

Il turno successivo riporta `init.model:"claude-sonnet-5-5"`, `init.permissionMode:"acceptEdits"`.
Inviati a turno fermo (tra due `result`). `model` accetta alias (`sonnet`, `opus`, `haiku`, `default`) o id completo.
Alternativa (non necessaria): riavvio con `--resume <sessionId> --model ...`.

Altri messaggi di controllo verificati:

| request | risposta |
|---|---|
| `{"subtype":"get_context_usage"}` | `{"categories":[{"id":"system_prompt","name":"System prompt","tokens":2666,...},{"id":"messages",...},{"id":"free_space",...}],"totalTokens":32937,"maxTokens":1000000,"rawMaxTokens":1000000,"percentage":3,"gridRows":[...],...}` |
| `{"subtype":"mcp_status"}` | `{"mcpServers":[]}` |
| subtype sconosciuto | `{"subtype":"error","request_id":"host_req_3","error":"Unsupported control request subtype: no_such_thing"}` |

`request_id` lato host: qualunque stringa unica (lo spike usava `host_req_N`); la CLI usa UUID per le sue.

---

## 9. Fine turno e uso del contesto

```json
{"type":"result","subtype":"success","is_error":false,"result":"hello","stop_reason":"end_turn","terminal_reason":"completed","num_turns":1,
 "session_id":"ba875580-1f68-4292-ab0f-757b0e1ec9a4","duration_ms":2076,"duration_api_ms":919,"total_cost_usd":0.00356553,
 "usage":{"input_tokens":2,"cache_creation_input_tokens":16585,"cache_read_input_tokens":24633,"output_tokens":4,"output_tokens_details":{"thinking_tokens":0},
   "iterations":[{"input_tokens":2,"output_tokens":4,"cache_read_input_tokens":24633,"cache_creation_input_tokens":16585,"type":"message"}],"...":"..."},
 "modelUsage":{"claude-haiku-5-5":{"inputTokens":2,"outputTokens":4,"cacheReadInputTokens":24633,"cacheCreationInputTokens":16585,"costUSD":0.00356553,"contextWindow":1000000,"maxOutputTokens":128000,"...":"..."}},
 "permission_denials":[],"subagent_stats":{"spawned":0,"...":"..."},"uuid":"10376a88-...","...":"molti campi di timing"}
```

- **Fine turno = riga `type:"result"`** (qualsiasi subtype). `is_error` distingue errore/interruzione.
  Subtype visti: `success`, `error_during_execution` (interrupt). Dall'SDK sono attesi anche `error_max_turns`,
  `error_max_budget_usd` (UNVERIFIED).
- `result.usage` è **cumulativo sui round API del turno**; `usage.iterations` contiene solo l'**ultimo** round.
- **Contesto usato** ≈ `iterations[-1].input_tokens + cache_read_input_tokens + cache_creation_input_tokens` (+ `output_tokens`)
  dell'ultima iterazione; **finestra** = `modelUsage[<model>].contextWindow`. Esempio: 2+24633+16585 = 41 220 / 1 000 000.
  In tempo reale lo stesso calcolo si fa su ogni `stream_event message_start.message.usage`.
  Valore esatto (come `/context`): control request `get_context_usage` → `totalTokens`, `maxTokens`, `percentage`.
- `modelUsage` è cumulativo per la vita del processo (dopo `set_model` contiene due modelli).

---

## 10. Comandi slash (`/compact`)

Inviato come messaggio utente `"/compact"`. Righe (`slash-command.jsonl`):

```json
{"type":"system","subtype":"status","status":"compacting","session_id":"...","uuid":"..."}
{"type":"system","subtype":"status","status":null,"compact_result":"success","session_id":"...","uuid":"..."}
{"type":"system","subtype":"init","...":"..."}
{"type":"system","subtype":"compact_boundary","session_id":"...","uuid":"107b1e22-...","compact_metadata":{"trigger":"manual","pre_tokens":32928,"post_tokens":4041,"cumulative_dropped_tokens":28887,"duration_ms":5781},"logical_parent_uuid":"7eece411-..."}
{"type":"user","message":{"role":"user","content":"This session is being continued from a previous conversation ... Summary: ..."},"...":"..."}
{"type":"user","message":{"role":"user","content":"<local-command-stdout>Compacted </local-command-stdout>"},"session_id":"...","parent_tool_use_id":null,"uuid":"...","timestamp":"...","isReplay":true}
{"type":"result","subtype":"success","is_error":false,"result":"","stop_reason":null,"terminal_reason":null,"num_turns":0,"...":"..."}
```

Regola generale: i comandi locali producono una riga `user` con contenuto `<local-command-stdout>…</local-command-stdout>`
e `isReplay:true`, poi un `result` con `num_turns:0`. Il riassunto della compattazione è un `user` (da non mostrare come prompt umano).
`/rename X` emette `{"type":"system","subtype":"session_title_changed","title":"X",...}`.

---

## 11. Formato sessione su disco (per il lettore, Task 10–11)

### 11.1 Percorso e regola di codifica della cartella

`~/.claude/projects/<encodeCwd(cwd)>/<sessionId>.jsonl` (+ eventuale `<sessionId>/subagents/agent-<agentId>.jsonl`).

Regola **verificata**:

1. `cwd` = percorso reale (symlink risolti: `/tmp/...` → `/private/tmp/...`) in **Unicode NFC**
   (una cartella creata in NFD — `a`+U+0300 — viene registrata come `à` NFC).
2. Ogni **unità UTF-16** che non è `[A-Za-z0-9]` diventa `-` (spazio, `.`, `_`, `/`, `-`, lettere accentate; un'emoji = 2 unità → `--`).
3. Se il risultato supera **200** caratteri: primi 200 caratteri + `-` + hash, dove hash = `abs(javaHashCode(cwd))` in base 36
   (hashCode a 32 bit, `h = h*31 + unità UTF-16`, sul cwd originale non codificato).

Esempi reali (prefisso `/private/tmp/.../spike/enc/` codificato come `-private-tmp-...-spike-enc-`):

| nome cartella | suffisso codificato |
|---|---|
| `a b` | `-a-b` |
| `a.b` | `-a-b` |
| `a_b` | `-a-b` |
| `città` | `-citt-` |
| `Mix-è v1.2_x y` | `-Mix---v1-2-x-y` |
| `emoji😀x` | `-emoji--x` |
| `nfdà` (creata NFD) | `-nfd-` |
| `L`×230 | primi 200 car. + `-z0drq2` (lunghezza 207) |

La codifica **non è invertibile** (`a b`, `a.b`, `a_b` collidono nella stessa cartella!). Il lettore deve quindi:
per elencare le sessioni di un progetto, codificare il cwd con la regola sopra e poi **filtrare per il campo `cwd`**
delle righe (le righe `user`/`assistant` lo portano) per scartare i progetti omonimi dopo la codifica.

Riferimento Rust:

```rust
fn encode_project_dir(cwd_nfc_realpath: &str) -> String {
    let units: Vec<u16> = cwd_nfc_realpath.encode_utf16().collect();
    let s: String = units.iter().map(|&u| if (u as u32) < 128 && (u as u8 as char).is_ascii_alphanumeric() { u as u8 as char } else { '-' }).collect();
    if s.len() <= 200 { return s; }
    let mut h: i32 = 0;
    for &u in &units { h = h.wrapping_mul(31).wrapping_add(u as i32); }
    format!("{}-{}", &s[..200], to_base36((h as i64).unsigned_abs())) // to_base36: cifre 0-9a-z, minuscole
}
```

(Hash verificato: cwd lungo 230+ caratteri → suffisso `z0drq2` ricalcolato identico con questo algoritmo.)

### 11.2 Tipi di riga

Ogni riga ha `type`. Osservati nei file prodotti dallo spike:

| `type` | contenuto chat? | note |
|---|---|---|
| `user` | **sì** (vedi 11.3) | `message.content` stringa o array |
| `assistant` | **sì** | un content block per riga (come in stream), `message.id` condiviso |
| `system` | parziale | `subtype:"compact_boundary"` (`content:"Conversation compacted"`), `subtype:"local_command"` (`content:"<local-command-stdout>…"`) |
| `custom-title` | titolo | `{"type":"custom-title","customTitle":"My Spike Title","sessionId":"…"}` — **vince l'ultima** (ripetuta più volte; dopo `/rename` cambia) |
| `agent-name` | — | duplica il titolo |
| `last-prompt` | anteprima | `{"type":"last-prompt","lastPrompt":"Reply with exactly: titled","leafUuid":"…"}` |
| `queue-operation` | — | `enqueue`/`dequeue` con `content` del prompt e `timestamp` |
| `attachment` | no | contesto iniettato (hook, reminder, file); ha `uuid`/`parentUuid` e partecipa alla catena |
| `file-history-snapshot`, `file-history-delta` | no | backup file per rewind |
| `api-request`, `api-request-shape`, `api-request-blob` | no | cache richieste, voluminose |
| `session-owner` | no | contiene `accountUuid`/`organizationUuid` (dati personali, non mostrare/loggare) |
| `cost-state` | no | costi cumulativi (`totalCostUSD`, `modelUsage`) |
| `atis-latch`, `mode`, `tools-used` | no | interni |

Il lettore deve ignorare i tipi sconosciuti. Non esiste un tipo `summary` nei file creati in 2.1.319.

### 11.3 Righe `user`/`assistant` e come distinguere

Campi comuni: `uuid`, `parentUuid`, `isSidechain`, `sessionId`, `cwd`, `timestamp` (ISO-8601 UTC con ms, es. `"2026-11-11T09:13:27.491Z"`),
`version`, `gitBranch`, `userType`, `entrypoint`, `message`.

**Prompt umano** (mostrare come messaggio utente):

```json
{"parentUuid":null,"isSidechain":false,"promptId":"475577dd-...","type":"user","message":{"role":"user","content":"Read a.txt, then use the Edit tool to change red to blue. Be brief."},
 "uuid":"97e9b574-...","timestamp":"2026-11-11T09:13:27.491Z","permissionMode":"acceptEdits","promptSource":"sdk","turnOrigin":"sdk","turnPosition":{"promptIndex":0,"turnIndex":1},
 "userType":"external","entrypoint":"claude-desktop","cwd":"/Users/dev/proj","sessionId":"c7f1c776-...","version":"2.1.319","gitBranch":"HEAD"}
```

Con immagine: `content` è array `[{"type":"text",...},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"…"}}]` (base64 salvato inline) e `imagePasteIds:[1]`.

**Risultato strumento** (non è un prompt): `content` è array con blocchi `tool_result`, e la riga ha
`toolUseResult` (camelCase su disco; `tool_use_result` in stream) e `sourceToolAssistantUUID`:

```json
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_01BSyVhT7F8ixLPnU6rqA9U8","type":"tool_result","content":"1\tcolor = red\n..."}]},
 "toolUseResult":{"type":"text","file":{"filePath":"/Users/dev/proj/a.txt","...":"..."}},"sourceToolAssistantUUID":"31029fb9-...","permissionDecision":{"decision":"accept","source":"config","reasonType":"mode"},"isSidechain":false,"...":"..."}
```

**Righe meta / sintetiche** (non mostrare come prompt umano):
- `isMeta:true` (es. `<local-command-caveat>…`, `<system-reminder>…`).
- `isCompactSummary:true` + `isVisibleInTranscriptOnly:true` (riassunto dopo `/compact`).
- contenuto che inizia con `<command-name>/compact</command-name>` → invocazione di comando slash (mostrabile come "/compact").
- contenuto `<local-command-stdout>…` → output di comando locale.
- contenuto `<task-notification>…` con `origin.kind:"task-notification"`, `promptSource:"system"`, `turnOrigin:"task_notification"` → notifica subagent.
- `[{"type":"text","text":"[Request interrupted by user]"}]` (o `"... for tool use]"`) → marker di interruzione.

Regola pratica per "prompt umano": `type=="user"` && `!isMeta` && `!isCompactSummary` && nessun blocco `tool_result`
&& `promptSource` in (`"sdk"`, o assente per vecchie versioni) && il testo non inizia con `<local-command-` / `<command-name>` / `<task-notification>`.

**Assistente**: `message` completo dell'API con `model`, `id`, `content` (un blocco), `usage`, `stop_reason` (su disco valorizzato,
es. `"end_turn"`); extra `requestId`, `requestedModel`, `effort`, `thinkingDisplay` per i thinking.
Su disco i blocchi `thinking` hanno `thinking:""` (come in stream). Righe con `message.model:"<synthetic>"` sono sintetiche (comandi locali).

### 11.4 `isSidechain` e subagent

- Nel file principale tutte le righe hanno `isSidechain:false`.
- Il subagent è in un **file separato** `<sessionId>/subagents/agent-<agentId>.jsonl` con `isSidechain:true` e `agentId` su ogni riga
  (prima riga `user` = prompt del subagent, `parentUuid:null`). Accanto: `agent-<agentId>.meta.json`
  (`{"agentType":"general-purpose","description":"…","toolUseId":"toolu_01KMD62ujTvgGAKrRd1YaE9g","spawnDepth":1,"finished":true,...}`)
  — `toolUseId` lega al `tool_use` `Agent` del file principale — e `agent-<agentId>.prefix.json`.
- Il lettore della chat principale può ignorare `isSidechain:true`; per mostrare il dettaglio del subagent legge il file `subagents/`.

### 11.5 Ordine e catena

Le righe sono in ordine di scrittura (append). `parentUuid` forma una catena che passa anche per righe `attachment`
e non è utile per l'ordinamento della chat: **leggere in ordine di file**. Dopo `compact_boundary` la catena riparte
(`parentUuid:null`, `logicalParentUuid` punta al vecchio ramo). Un `--resume` aggiunge righe allo stesso file.

---

## 12. Elenco fixture

| file | scenario | note |
|---|---|---|
| `simple-turn.jsonl` | testo semplice | |
| `tool-use-edit.jsonl` | Read + Edit | `--permission-mode acceptEdits` (nessun prompt) |
| `permission-request.jsonl` | `initialize`; Write consentito; Bash negato; Bash consentito con `updatedPermissions` | 3 turni |
| `ask-user-question.jsonl` | `AskUserQuestion` risposta "Blue" | |
| `plan-mode.jsonl` | plan → `ExitPlanMode` approvato → Write consentito | `--permission-mode plan` |
| `subagent.jsonl` | `Agent` in background + turno automatico di notifica | 2 `result` |
| `todo-update.jsonl` | `TaskCreate`×2, `TaskUpdate` in_progress/completed | `--tools TaskCreate,TaskUpdate,TaskList,TaskGet,Read` |
| `interrupt.jsonl` | interrupt durante lo streaming + turno successivo | |
| `slash-command.jsonl` | turno, `/compact`, turno | |
| `image-input.jsonl` | immagine PNG base64 (16×16 rossa) | l'input è solo su stdin (§2.3) |
| `model-switch.jsonl` | `set_model sonnet` + `set_permission_mode acceptEdits` tra due turni | |
| `unknown.jsonl` | `simple-turn` + riga `{"type":"future-thing"}` aggiunta a mano | |
| `control-misc.jsonl` | `get_context_usage`, `mcp_status`, subtype sconosciuto, interrupt con permesso pendente (`control_cancel_request`) | extra |

Le fixture contengono solo stdout. Le righe stdin inviate sono quelle mostrate nei §2.3, 5.2, 5.3, 5.4, 7, 8.
Le catture sono state fatte con `--safe-mode` (niente hook/MCP utente): senza, compaiono in testa righe
`{"type":"system","subtype":"hook_started","hook_id":"…","hook_name":"SessionStart:startup","hook_event":"SessionStart",...}` e
`{"type":"system","subtype":"hook_response",...,"output":"…","stdout":"…","exit_code":0,"outcome":"success"}` e `mcp_servers` popolato.

## 13. Riepilogo UNVERIFIED

- `AskUserQuestion` con `multiSelect:true` e risposta libera ("Other").
- Rifiuto del piano (`deny` su `ExitPlanMode`) e approvazione con passaggio a `acceptEdits` via `updatedPermissions`.
- Subtype di `result` diversi da `success`/`error_during_execution`.
- Testo del thinking non vuoto (mai osservato in 2.1.319).
- Se un elenco `--tools` esplicito (strumenti di default + Task tools) mantenga anche gli strumenti differiti/MCP come il set di default.
