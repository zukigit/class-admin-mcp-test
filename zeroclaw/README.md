# ZeroClaw kit

A portable folder for ZeroClaw agents and profiles. You can put it anywhere:
inside a project, in your home dir, or in its own git repo. It never assumes
where ZeroClaw's data lives. You pass that location with `--data` every time
you install.

```
<kit>/
  new.sh                          # create an item from its template
  install.sh                      # install item folders into a ZeroClaw config
  agents/<name>/                  # saved agents
    agent.toml                    #   [agent]         -> [agents.<name>]
    workspace/*.md                #   SOUL, IDENTITY, USER, AGENTS, TOOLS, HEARTBEAT, MEMORY
  risk_profiles/<name>/           # saved risk profiles
    risk_profile.toml             #   [risk_profile]  -> [risk_profiles.<name>]
  runtime_profiles/<name>/        # saved runtime profiles
    runtime_profile.toml          #   [runtime_profile] -> [runtime_profiles.<name>]
  templates/
    agent/  risk_profile/  runtime_profile/   # blank items, with ZeroClaw defaults
    config.base.toml              # tools/provider/gateway, used only if the data dir has no config yet
  lib/zeroclaw.sh                 # shared helpers
  docker-compose.example.yml      # ZeroClaw service to paste into a project's compose
```

Every item is a folder, and the folder name is the item's name. Nothing in
the kit holds credentials, so it is safe to commit or share.

## Create

```bash
KIT=/path/to/zeroclaw-kit

$KIT/new.sh agent   agent1          # -> $KIT/agents/agent1/
$KIT/new.sh risk    risk_profile1   # -> $KIT/risk_profiles/risk_profile1/
$KIT/new.sh runtime fast            # -> $KIT/runtime_profiles/fast/
```

Then edit the new folder. For an agent, that means `workspace/IDENTITY.md`,
`SOUL.md`, `USER.md`, and `agent.toml`. An agent picks its profiles in
`agent.toml`:

```toml
risk_profile = "risk_profile1"   # what it may do without asking: commands, paths, approvals
runtime_profile = "default"      # how it runs: limits, context compression, thinking
```

The kit ships a `default` risk profile and a `default` runtime profile, both with ZeroClaw's default values.

## Install

```bash
$KIT/install.sh --data ./zeroclaw-data $KIT/agents/agent1
$KIT/install.sh --data ./zeroclaw-data $KIT/risk_profiles/strict $KIT/runtime_profiles/fast
docker compose restart zeroclaw
```

- `--data` (or `-d`) is required. It's ZeroClaw's home dir, the folder
  mounted into the container, which holds `.zeroclaw/config.toml`, the
  secret key, sessions and memory. Keep it out of git. If it has no config
  yet, one is created from `templates/config.base.toml`.
- The type is detected from the folder (`agent.toml`, `risk_profile.toml` or
  `runtime_profile.toml`). You can pass several folders at once.
- An agent install also installs the risk and runtime profiles the agent
  names, if the config doesn't have them yet, and copies its workspace files.
- `--force` replaces items already in the config (a `config.toml.bak` is
  kept) and overwrites agent workspace files, including `MEMORY.md`. The
  memory DB and sessions are kept. Profiles pulled in by an agent are never
  replaced, because other agents may share them. Pass the profile folder
  itself with `--force` to update it.

You can install the same folders into as many data dirs or projects as you like.

## API keys

API keys are never stored in the kit. Pass them through the project's `docker-compose.yml` plus `.env`:

```yaml
- ZEROCLAW_providers__models__opencode__deepseek_flash__api_key=${PROVIDER_API_KEY:-}
```

Or enter the key in the dashboard, where ZeroClaw stores it encrypted with its own `.secret_key`.
