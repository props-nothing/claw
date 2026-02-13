---
name: 1Password Credential Management
description: This skill should be used when the user asks to "get a password", "look up credentials", "retrieve a secret", "use 1Password", "log into a website", "access a server", or when any task requires authentication credentials like passwords, API keys, tokens, or SSH keys. Activate automatically whenever the <credentials> block indicates provider is "1password".
version: 2.0.0
tags: [credentials, 1password, secrets, security, passwords]
author: Claw Team
---

# 1Password Credential Management

## Overview

Retrieve credentials from 1Password using the `op` CLI. Secrets are fetched on-demand and never stored in memory — only the item-name-to-service mapping is remembered for future sessions.

## Two Operating Modes

### Mode 1: Service Account (headless / server — recommended for automation)

When `OP_SERVICE_ACCOUNT_TOKEN` is set (via `claw.toml` or environment variable), the `op` CLI works **without** the 1Password desktop app and **without** any biometric prompts. This is the best mode for:
- Running Claw on a headless Linux server
- Docker / containerized deployments
- Unattended automation (no Touch ID interruptions)
- CI/CD pipelines

Setup:
1. Create a service account at https://my.1password.com → Developer → Service Accounts
2. Grant it access to the vaults Claw needs
3. Add the token to your config:

```toml
[credentials]
provider = "1password"
service_account_token = "ops_..."
default_vault = "Servers"
```

Or set via environment variable: `export OP_SERVICE_ACCOUNT_TOKEN=ops_...`

In this mode, every `op` command works directly — no signing in, no biometric, no desktop app required.

### Mode 2: Desktop App Integration (macOS / biometric)

When no service account token is set, `op` authenticates through the 1Password desktop app using biometric (Touch ID on macOS). This is convenient for interactive use but causes **repeated Touch ID prompts** when the agent makes many `op` calls.

**To minimize Touch ID prompts**, batch credential lookups with `op run`:

```bash
# BAD: triggers Touch ID on EVERY call
op read "op://Servers/Plesk/username"    # ← Touch ID prompt
op read "op://Servers/Plesk/password"    # ← another Touch ID prompt

# GOOD: triggers Touch ID ONCE for the entire batch
export PLESK_USER="op://Servers/Plesk/username"
export PLESK_PASS="op://Servers/Plesk/password"
op run -- sh -c 'echo "user=$PLESK_USER pass=$PLESK_PASS"'
```

`op run` resolves all `op://` references in exported env vars in a single biometric session.

For a **single** lookup, `op read "op://Vault/Item/field"` is fine (one prompt).

**If Touch ID keeps interrupting the agent**, switch to a service account token.

## Credential Retrieval Flow

### Step 1: Check memory for existing mapping

```
memory_search("credentials for <service name>")
```

If a mapping exists (e.g. "Plesk credentials → 1Password item 'Plesk Admin' in vault 'Servers'"), use it directly in step 2.

### Step 2: Retrieve from 1Password

```bash
# Preferred: read a single field by URI (works in both modes)
op read "op://Vault/Item/field"

# Get username and password together
op item get "Item Name" --fields label=username,label=password

# Get from a specific vault
op item get "Item Name" --vault "Servers" --fields label=username,label=password

# Get OTP/TOTP code
op item get "Item Name" --otp
```

### Step 3: Store the mapping (first time only)

After first successful retrieval, store the mapping so future sessions find it automatically:

```
memory_store(
  category: "credentials",
  key: "plesk_admin",
  value: "Plesk login credentials are in 1Password item 'Plesk Admin' in vault 'Servers'. URL: https://server.example.com:8443"
)
```

**Never store the actual password, token, or secret in memory.**

## Finding Items

When no mapping exists in memory, search 1Password:

```bash
# Search by name (partial match)
op item list --format=json | jq '.[] | select(.title | test("search term"; "i")) | {id, title, vault}'

# List items in the default vault
op item list --vault "Servers" --format=json

# List all vaults
op vault list
```

Then store the mapping once found.

## Environment Variable Injection (batch pattern)

For tasks needing multiple secrets at once, use `op run` to inject all of them with a single auth:

```bash
# Set references as env vars
export DB_HOST="op://Servers/Database/host"
export DB_USER="op://Servers/Database/username"
export DB_PASS="op://Servers/Database/password"

# Run the command — op resolves all references at once
op run -- psql "host=$DB_HOST user=$DB_USER password=$DB_PASS dbname=mydb"
```

## SSH Keys

```bash
# 1Password SSH agent handles this automatically if configured
ssh -o IdentityAgent=~/Library/Group\ Containers/2BUA8C4S2C.com.1password/t/agent.sock user@host
```

## Troubleshooting

```bash
# Check if op is available
which op

# Check if signed in
op account list

# Check current session
op whoami

# Desktop app mode — session expired
# → User needs to unlock 1Password, then retry

# Service account mode — verify token
# → Check OP_SERVICE_ACCOUNT_TOKEN is set: printenv OP_SERVICE_ACCOUNT_TOKEN | head -c 10

# Touch ID keeps prompting (desktop app mode)
# → Switch to a service account token in claw.toml:
#   [credentials]
#   service_account_token = "ops_..."
# → Or batch lookups with `op run` (see above)

# Item not found — broader search
op item list --format=json | jq '.[].title'
```

## Security Rules

- **Never echo or print passwords** to terminal output — pipe them directly into commands
- **Never store secrets in memory** — only store item names, vault paths, and service mappings
- **Never write secrets to files** — use `op read` or `op run` environment variable injection
- **Credentials are ephemeral** — retrieve again each session rather than caching
- **The operator authorized this** — the `[credentials]` config block is explicit consent
