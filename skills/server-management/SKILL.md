---
name: Server Management
description: This skill should be used when the user asks to "manage a server", "deploy to a server", "SSH into a server", "check server health", "restart a service", "view server logs", or needs to manage, configure, deploy to, or troubleshoot a remote server via SSH.
version: 1.0.0
tags: [ssh, servers, devops, linux, sysadmin]
author: Claw Team
---

# Server Management

## Overview

Procedural guide for managing remote servers via SSH — connectivity testing, health checks, deployment, service management, log analysis, and security auditing.

## Prerequisites

- SSH access to the target server (credentials or key-based auth)
- To retrieve SSH credentials: run `memory_search("credentials ssh <hostname>")` — if a 1Password mapping exists, use `op` to fetch the key or password
- Test connectivity first: `ssh -o ConnectTimeout=5 -o BatchMode=yes user@host echo ok`
- 1Password SSH agent can provide keys automatically if configured

## Connecting

### Test connectivity

```bash
ssh -o ConnectTimeout=5 user@host echo "Connection OK"
```

### Running remote commands

Use `shell_exec` with SSH:

```bash
ssh user@host "command to run"
```

For multiple commands:

```bash
ssh user@host 'command1 && command2 && command3'
```

## Server Health Check

Run these checks to assess server status:

```bash
# System info
ssh user@host 'uname -a'

# Uptime and load
ssh user@host 'uptime'

# Disk usage
ssh user@host 'df -h'

# Memory usage
ssh user@host 'free -h'

# Top processes by CPU
ssh user@host 'ps aux --sort=-%cpu | head -10'

# Top processes by memory
ssh user@host 'ps aux --sort=-%mem | head -10'

# Check for failed systemd services
ssh user@host 'systemctl --failed'

# Recent error logs
ssh user@host 'journalctl -p err --since "1 hour ago" --no-pager | tail -20'
```

## Deployment

### Deploy with rsync

```bash
# Sync local directory to remote
rsync -avz --delete ./dist/ user@host:/var/www/site/

# Dry run first
rsync -avzn --delete ./dist/ user@host:/var/www/site/
```

### Deploy with git

```bash
ssh user@host 'cd /var/www/app && git pull origin main && npm install && pm2 restart app'
```

### Deploy with Docker

```bash
ssh user@host 'cd /opt/app && docker compose pull && docker compose up -d'
```

## Service Management

```bash
# systemd services
ssh user@host 'sudo systemctl status nginx'
ssh user@host 'sudo systemctl restart nginx'
ssh user@host 'sudo systemctl enable --now nginx'

# Check if a port is listening
ssh user@host 'ss -tlnp | grep :80'

# Verify config before restart
ssh user@host 'sudo nginx -t'
ssh user@host 'sudo apachectl configtest'
```

## File Operations

```bash
# Read a remote file
ssh user@host 'cat /etc/nginx/nginx.conf'

# Write a remote file via stdin
ssh user@host 'cat > /tmp/config.txt << "EOF"
file content here
EOF'

# Copy files to server
scp local-file.txt user@host:/remote/path/

# Copy files from server
scp user@host:/remote/path/file.txt ./local-path/
```

## Log Analysis

```bash
# Recent nginx access logs
ssh user@host 'tail -100 /var/log/nginx/access.log'

# Error logs
ssh user@host 'tail -100 /var/log/nginx/error.log'

# Search logs for errors
ssh user@host 'grep -i error /var/log/syslog | tail -20'

# Application logs (PM2)
ssh user@host 'pm2 logs --lines 50'
```

## Security

```bash
# Check open ports
ssh user@host 'ss -tlnp'

# Check firewall rules (ufw)
ssh user@host 'sudo ufw status verbose'

# Recent SSH logins
ssh user@host 'last -20'

# Failed login attempts
ssh user@host 'sudo grep "Failed password" /var/log/auth.log | tail -10'
```

## Important Notes

- Test connectivity before running commands
- Use `ssh -o StrictHostKeyChecking=no` only for first-time connections the operator approves
- For long-running remote commands, consider `nohup` or `screen`/`tmux`
- Store server details (hostname, user, common paths) in memory for quick access
- Verify destructive commands before executing — confirm the target
- If SSH hangs, the timeout option `-o ConnectTimeout=10` prevents blocking
