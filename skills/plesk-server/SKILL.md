---
name: Plesk Server Management (SSH-first)
description: Manage Plesk servers primarily via SSH + Plesk CLI tools (plesk bin, plesk ext, plesk db). Use this when the operator asks to manage Plesk (domains, subdomains, DNS, databases, SSL/Let's Encrypt, mail) and you have server SSH access.
version: 2.0.0
tags: [hosting, plesk, devops, server, ssh]
author: Claw Team
---

# Plesk Server Management (SSH-first)

## Overview

This skill manages Plesk **without browser automation**. Prefer:

- SSH into the server
- Use Plesk CLI utilities:
  - `plesk bin ...` (core management)
  - `plesk ext ...` (extensions, incl. Let's Encrypt)
  - `plesk db` (direct DB access to psa/admin DB when needed)

Only use a browser if explicitly requested by the operator or if a task is impossible via CLI.

## Prerequisites

- Server hostname/IP and SSH method (password or key)
- Sudo/root access (many Plesk operations require root)
- Plesk installed on the target server

### Credential handling (1Password)

If credentials are in 1Password:

1. `memory_search("plesk ssh")` / `memory_search("<server> plesk")` to find an item mapping.
2. Retrieve with 1Password CLI, e.g.
   - `op item get "Item Name" --fields label=username,label=password,label=host`
   - or `op item get "Item Name" --format json` and extract fields.
3. **Do not store secrets** in memory. Only store the mapping (item name + vault).

After first successful retrieval, store mapping:

- `memory_store(category: "learned_lessons", key: "plesk_<server>_ssh_1password_item", value: "SSH/Plesk creds are in 1Password item '...' (vault '...'). Host: ...")`

## Connect to the server (SSH)

Validate SSH availability locally:

- `which ssh`

Connect (examples):

```bash
ssh root@HOST
# or
ssh -i /path/to/key user@HOST
```

If you need non-interactive execution, use:

```bash
ssh -o BatchMode=yes user@HOST 'command'
```

## Verify Plesk & discover environment

Run these first after login:

```bash
plesk version || cat /usr/local/psa/version
uname -a
whoami
```

Useful inventory:

```bash
plesk bin subscription --list
plesk bin domain --list
plesk bin ipmanage --list
plesk bin service --list
```

## Common tasks (CLI)

### 1) Create a domain / subscription

Plesk usually works with **subscriptions** (hosting container) and **domains**.

Create a subscription:

```bash
plesk bin subscription --create example.com \
  -owner admin \
  -service-plan "Default Domain" \
  -ip "SERVER_IP"
```

If you only need a domain under an existing subscription, list subscriptions first and decide.

### 2) Create a subdomain

```bash
plesk bin subdomain --create sub -domain example.com
```

Verify:

```bash
plesk bin subdomain --list -domain example.com
```

### 3) DNS management

#### List DNS records

```bash
plesk bin dns --info example.com
```

#### Add a DNS record

Examples:

A record:

```bash
plesk bin dns --add example.com -a sub -ip 203.0.113.10
```

CNAME:

```bash
plesk bin dns --add example.com -cname www -canonical example.com.
```

TXT:

```bash
plesk bin dns --add example.com -txt _acme-challenge -value "txt-value"
```

#### Update / remove records

Plesk DNS CLI varies slightly by version; safest workflow:

1. `plesk bin dns --info example.com` (identify the exact record)
2. Remove the old record
3. Add the new record

Remove examples:

```bash
# Remove A record
plesk bin dns --del example.com -a sub -ip 203.0.113.10

# Remove TXT record
plesk bin dns --del example.com -txt _acme-challenge -value "txt-value"
```

Apply DNS changes (if required by your setup):

```bash
plesk bin dns --update example.com
```

### 4) Create a database + user

List DB servers:

```bash
plesk bin dbserver --list
```

Create DB user:

```bash
plesk bin database-user --create dbuser \
  -passwd 'STRONG_PASSWORD'
```

Create DB:

```bash
plesk bin database --create dbname \
  -domain example.com \
  -type mysql \
  -add-user dbuser
```

List:

```bash
plesk bin database --list -domain example.com
plesk bin database-user --list
```

### 5) SSL / Let's Encrypt (extension)

Check extensions:

```bash
plesk ext --list
```

Let's Encrypt extension name is commonly `letsencrypt`.

Issue/renew a certificate (typical):

```bash
plesk ext letsencrypt --help
```

Then use the command supported by that server/version. Common patterns include providing:

- domain name(s)
- email
- webroot / subscription context

If the extension CLI differs, fall back to:

- `plesk bin certificate --help`
- or inspect extension commands: `plesk ext letsencrypt --help`

### 6) Mailboxes

Enable mail for a domain (if disabled):

```bash
plesk bin domain --update example.com -mail_service true
```

Create mailbox:

```bash
plesk bin mail --create user@example.com -passwd 'STRONG_PASSWORD' -mailbox true
```

List mailnames:

```bash
plesk bin mail --info user@example.com
plesk bin mail --list example.com
```

### 7) Restart services / check health

Service list:

```bash
plesk bin service --list
```

Restart Plesk panel service (varies by OS):

```bash
# Often works on systemd hosts
systemctl restart sw-cp-server || true
systemctl restart psa || true
```

Check Plesk repair tools:

```bash
plesk repair --help
plesk repair all -y
```

## Evidence / “screenshots” replacement (SSH-first)

When the operator asks for proof, provide **command output** instead of screenshots:

- `plesk bin ... --list/--info`
- `dig`/`host` for DNS verification
- `openssl s_client` for TLS verification

Examples:

```bash
plesk bin subdomain --list -domain example.com
plesk bin dns --info example.com
openssl s_client -connect example.com:443 -servername example.com </dev/null 2>/dev/null | openssl x509 -noout -subject -issuer -dates
```

If a visual screenshot is explicitly required, ask for approval to use browser automation.

## Notes / troubleshooting

- If `plesk` command isnt in PATH, try:
  - `/usr/sbin/plesk`
  - `/usr/local/psa/bin/plesk`
- For deep troubleshooting, Plesk logs are typically in:
  - `/var/log/plesk/panel.log`
  - `/var/log/sw-cp-server/error_log`
  - `/var/log/plesk/plesklog`
- Prefer `plesk bin ... --help` for version-specific flags.
