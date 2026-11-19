# ZeroClaw kit

A portable folder for ZeroClaw agents. You can put it anywhere: inside a
project, in your home dir, or in its own git repo. It never assumes where
ZeroClaw's data lives. You pass that location with `--data` every time you
install an agent.

```
<kit>/
  agents/<name>/              # saved agents, one folder each (folder name = agent name)
    agent.toml                #   settings: [agent] -> [agents.<name>] when installed
    workspace/*.md            #   SOUL, IDENTITY, USER, AGENTS, TOOLS, HEARTBEAT, MEMORY
  templates/
    agent/                    # blank agent used by new-agent.sh
    config.base.toml          # shared settings, used only if the data dir has no config yet
  scripts/
    new-agent.sh              # template -> <kit>/agents/<name>/
    install-agent.sh          # <kit>/agents/<name>/ -> <data>/.zeroclaw
  docker-compose.example.yml  # ZeroClaw service to paste into a project's compose
```

Nothing here holds credentials, so the whole kit is safe to commit or share.

## Data dir (`--data`)

The data dir is ZeroClaw's home, the folder mounted into the container
(`HOME=/zeroclaw-data`). It contains `.zeroclaw/config.toml`, the secret key,
sessions and memory, so keep it out of git. `install-agent.sh` requires it:

```bash
install-agent.sh --data /path/to/zeroclaw-data agent1   # or -d, or --data=...
```

If it has no `.zeroclaw/config.toml` yet, one is created from
`templates/config.base.toml`.

## Create an agent

```bash
KIT=/path/to/zeroclaw-kit

$KIT/scripts/new-agent.sh agent1                     # -> $KIT/agents/agent1
# edit $KIT/agents/agent1/workspace/IDENTITY.md, SOUL.md, USER.md, agent.toml
$KIT/scripts/install-agent.sh --data ./zeroclaw-data agent1
docker compose restart zeroclaw
```

You can install the same saved agent into as many data dirs or projects as you like.
`install-agent.sh` also takes an agent folder path instead of a name, and
`--force` reinstalls over an existing agent. With `--force` the old config
sections are replaced (a `config.toml.bak` is kept), the workspace files are
overwritten, and the memory DB and sessions are kept.

## API keys

API keys are never stored in the kit. Pass them through the project's `docker-compose.yml` plus `.env`:

```yaml
- ZEROCLAW_providers__models__opencode__deepseek_flash__api_key=${PROVIDER_API_KEY:-}
```

Or enter the key in the dashboard, where ZeroClaw stores it encrypted with its own `.secret_key`.
