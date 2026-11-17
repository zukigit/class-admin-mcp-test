# class-admin-mcp

An MCP server for a school's classes, written in Rust with SQLite.

- **Public (no login):** browse published courses and upcoming classes (times, fee in USD,
  spots left), book a seat with name + email, check or cancel your own booking.
- **Admins (token):** manage courses, classes, weekly times, fees and capacity; approve,
  reject or cancel bookings.

New bookings are **pending** and hold a seat until an admin approves them. Pending bookings
expire after 72 hours or when the class starts. The full design is in
[docs/class-admin-mcp-server-plan.md](docs/class-admin-mcp-server-plan.md).

## Quick start

```bash
cargo build --release
BIN=./target/release/class-admin

# Signing secret for admin tokens. Keep it on the server only.
export CLASS_ADMIN_JWT_SECRET=$($BIN secret generate)

# Optional: sample courses and classes
$BIN seed

# Start the server (default 127.0.0.1:8080, database ./data/class_admin.db)
$BIN serve
```

Endpoints:

| Path | Who |
|---|---|
| `POST /mcp` | Everyone. No token needed; admin tokens also work here. |
| `POST /mcp/admin` | Admins only (`Authorization: Bearer <token>`); `401` without a token. |
| `GET /healthz` | Health check. |

## Admin tokens

```bash
$BIN token issue --user alice            # prints the token (default 30 days, max 90)
$BIN token inspect <token>               # valid / expired / revoked, claims
$BIN token revoke --jti <id>             # revoke one token (jti is printed by `issue`)
$BIN token revoke --token <token>        # same, using the token itself
$BIN token revoke --user alice           # revoke all of alice's tokens issued up to now
$BIN revoke list                         # show revocations
```

Revocation takes effect on the next request. To cancel **every** token at once, change
`CLASS_ADMIN_JWT_SECRET` and restart.

Token and revoke commands use the same `CLASS_ADMIN_DB` and `CLASS_ADMIN_JWT_SECRET` as the server,
so run them on the server host.

## Connecting an MCP client

Public users (Claude Code `.mcp.json`; other clients are similar):

```json
{ "mcpServers": { "school": { "type": "http", "url": "https://classes.example.com/mcp" } } }
```

Admins:

```json
{
  "mcpServers": {
    "school-admin": {
      "type": "http",
      "url": "https://classes.example.com/mcp/admin",
      "headers": { "Authorization": "Bearer ${CLASS_ADMIN_TOKEN}" }
    }
  }
}
```

Set `CLASS_ADMIN_TOKEN` in the admin's own environment; never commit it.

## Tools

| Public | Admin only |
|---|---|
| `list_courses`, `get_course` | `create_course`, `update_course` |
| `list_upcoming_classes`, `get_class` | `list_all_classes`, `create_class`, `update_class` |
| `book_class` | `set_class_schedule`, `set_class_fee` |
| `get_booking`, `cancel_booking` | `cancel_class`, `delete_class` |
| `whoami` | `list_bookings`, `approve_booking`, `reject_booking`, `admin_cancel_booking` |

Admin tools are hidden from public callers and refuse to run for them.

## Putting it on the internet

1. Run it behind an HTTPS proxy, for example [Caddy](https://caddyserver.com/):
   ```
   classes.example.com {
       reverse_proxy 127.0.0.1:8080
   }
   ```
2. Set `CLASS_ADMIN_PUBLIC_URL=https://classes.example.com`. Requests whose `Host` header is
   not this host (or localhost) are rejected, which protects against DNS rebinding.
3. Run only **one** copy of the server at a time (SQLite).

Requests that carry a browser `Origin` header are rejected unless the origin is listed in
`CLASS_ADMIN_ALLOWED_ORIGINS`. AI agents and CLI clients don't send one. For browser-based
tools such as MCP Inspector, add their origin, e.g. `CLASS_ADMIN_ALLOWED_ORIGINS=http://localhost:6274`.

## Configuration

All settings are environment variables (or the matching `--flag`; see `class-admin serve --help`).

| Variable | Default | Purpose |
|---|---|---|
| `CLASS_ADMIN_DB` | `./data/class_admin.db` | SQLite file |
| `CLASS_ADMIN_BIND` | `127.0.0.1:8080` | Listen address |
| `CLASS_ADMIN_PUBLIC_URL` | — | Public URL; its host is allowed in the `Host` header |
| `CLASS_ADMIN_ALLOWED_ORIGINS` | empty | Allowed browser origins, comma separated |
| `CLASS_ADMIN_TRUSTED_PROXY` | `127.0.0.1` | Only this proxy's `X-Forwarded-For` is trusted |
| `CLASS_ADMIN_JWT_SECRET` | — | Admin token secret (≥ 32 chars). Without it, admin access is off |
| `CLASS_ADMIN_RATE_LIMIT_PUBLIC` | `60/min` | Per-IP limit for public calls |
| `CLASS_ADMIN_RATE_LIMIT_BOOKING` | `5/hour` | Per-IP limit for `book_class` |
| `CLASS_ADMIN_RATE_LIMIT_ADMIN` | `300/min` | Per-token limit for admins |
| `CLASS_ADMIN_RATE_LIMIT_AUTH_FAIL` | `10/min` | Per-IP limit for failed token checks |
| `CLASS_ADMIN_MAX_PENDING_PER_EMAIL` | `3` | Max pending bookings per email |
| `CLASS_ADMIN_PENDING_TTL_HOURS` | `72` | Pending bookings expire after this |
| `CLASS_ADMIN_BOOKING_RETENTION_DAYS` | `365` | Booker name/email erased this long after the class ends |
| `RUST_LOG` | `info,rmcp=warn` | Log level |

## Maintenance

The server runs these on its own (every minute / hour). They can also be run by hand:

```bash
$BIN bookings expire               # expire overdue pending bookings
$BIN bookings purge-personal-data  # erase old booker names and emails
$BIN revoke prune                  # drop revocation rows that are no longer needed
```

The database file contains bookers' personal data, so keep it and any copies private.

## Development

```bash
cargo test                                  # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt
```
