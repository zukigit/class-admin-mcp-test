# ZeroClaw agent template

A reusable template for ZeroClaw agents, taken from the `minmin` agent in
`zeroclaw-data/`. It contains no credentials or runtime state.

| File | Purpose |
|------|---------|
| `config.base.toml` | Shared settings: tools, runtime/risk profiles, provider (no `api_key`) |
| `agent.toml` | The `[agents.<name>.*]` sections for one agent |
| `workspace/*.md` | Agent persona and memory files (`SOUL`, `IDENTITY`, `USER`, `AGENTS`, `TOOLS`, `HEARTBEAT`, `MEMORY`) |
| `new-agent.sh` | Renders the template into a data dir |

Placeholders: `{{AGENT_NAME}}`, `{{PROVIDER}}`, `{{PROVIDER_PROFILE}}`, `{{MODEL}}`.

## Create an agent

```bash
# into ./zeroclaw-data (adds to an existing config, or creates a new one)
templates/zeroclaw-agent/new-agent.sh tutor

# a different provider/model, into a separate data dir
PROVIDER=opencode PROVIDER_PROFILE=deepseek_flash MODEL=deepseek-v4.1-flash \
  templates/zeroclaw-agent/new-agent.sh tutor ./zeroclaw-data-tutor
```

Then:
1. Edit `IDENTITY.md`, `SOUL.md` and `USER.md` in `.zeroclaw/agents/<name>/workspace/`.
2. Provide the API key without committing it. Add this to `docker-compose.yml`:
   ```yaml
   - ZEROCLAW_providers__models__opencode__deepseek_flash__api_key=${PROVIDER_API_KEY:-}
   ```
   and put `PROVIDER_API_KEY=...` in `.env`. Or enter the key in the dashboard,
   where ZeroClaw stores it encrypted.
3. `docker compose restart zeroclaw`

## Left out on purpose

ZeroClaw creates these again on first run. Never copy them into the template:
`.secret_key`, `data/gateway-admin.token`, `gateway.paired_tokens`, encrypted
`api_key` values, `data/**/*.db*` (sessions, memory, devices), `state/`, lock files.
