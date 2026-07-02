# KetherServerDaemon

Daemon for managing Left 4 Dead 2 workshop and custom maps: installs VPKs, keeps a JSON registry, watches the addons folder, syncs with a remote backend, and exposes a local HTTP API plus interactive REPL.

## Requirements

- Rust stable (see `rust-toolchain.toml`)
- Network access for Steam Workshop downloads and optional backend sync

## Build

```bash
cargo build --release
```

Binary: `target/release/KetherServerDaemon`

## Configuration

Configuration is loaded from `config.toml` (or path in `KETHER_CONFIG`). Environment variables override file values:

| Variable | Description |
|----------|-------------|
| `KETHER_CONFIG` | Path to config TOML |
| `KETHER_L4D2_SERVER_DIR` | L4D2 server root (addons at `{dir}/left4dead2/addons`) |
| `KETHER_REGISTRY_PATH` | JSON map registry file |
| `KETHER_BACKEND_API_URL` | Remote sync API base URL (website-server: `http://127.0.0.1:3001/api`) |
| `KETHER_BACKEND_API_KEY` | Bearer token for backend API (must match website-server `[server_daemon].sync_api_key`) |
| `KETHER_LOCAL_API_BIND` | Local HTTP API bind address (default `127.0.0.1:8080`) |
| `KETHER_SYNC_INTERVAL_SECS` | Backend sync interval |
| `KETHER_LOG_LEVEL` | `trace`, `debug`, `info`, `warn`, `error` |
| `KETHER_MAX_DOWNLOAD_SIZE_BYTES` | Max download size (default 1GB) |
| `KETHER_MAX_EXTRACTION_SIZE_BYTES` | Max ZIP extraction size |
| `KETHER_MAX_EXTRACTION_FILE_COUNT` | Max files per archive |

## REPL commands

| Command | Description |
|---------|-------------|
| `ls` / `maps` | List installed maps |
| `i <url\|workshop_id> [name]` | Install map |
| `rm <id>` | Uninstall map |
| `u [id] [--check] [--force]` | Check or update workshop maps |
| `d [u\|U]` | Discover local VPKs (`u` = refresh metadata) |
| `compact` | Prune orphaned registry entries and reindex IDs |
| `info <id>` | Show map details |
| `modify <id> <field> <value>` | Edit registry field |
| `S` / `stop` | Stop daemon |

## HTTP API

Default bind: `http://127.0.0.1:8080`

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Health check |
| GET | `/api/maps` | List maps |
| GET | `/api/maps/{id}` | Get map |
| PATCH | `/api/maps/{id}` | Modify map field |
| POST | `/api/maps/install` | Install from URL or workshop ID |
| POST | `/api/maps/uninstall/{id}` | Uninstall map |
| POST | `/api/maps/workshop/update` | Update workshop maps (`check_only`, `force`) |
| POST | `/api/maps/discover` | Scan addons directory |
| POST | `/api/maps/compact` | Compact registry |

Responses use `{ "success": true, "data": ... }` or `{ "success": false, "error": "..." }`.

## Backend sync (website-server)

The daemon pushes its map registry to the website backend and polls for pending updates:

| Method | Path (relative to `backend_api_url`) | Auth |
|--------|--------------------------------------|------|
| POST | `/registry/sync` | `Authorization: Bearer <backend_api_key>` |
| GET | `/registry/updates` | same Bearer token |

Example `config.toml` when website-server runs on port **3001**:

```toml
backend_api_url = "http://127.0.0.1:3001/api"
backend_api_key = "your-shared-secret"
```

The public website reads installed maps from `GET http://127.0.0.1:3001/api/maps` (no auth).

## Systemd

Example user unit: `.config/systemd/user/ksd.service`

## Tests

```bash
cargo test
cargo clippy
cargo fmt --check
```
