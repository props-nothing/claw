---
name: System Administration
description: This skill should be used when the user asks to "check system health", "manage processes", "find large files", "check disk space", "manage network", "install a package", "set up a cron job", or needs local system administration for processes, services, files, networking, and system health on macOS or Linux.
version: 1.0.0
tags: [sysadmin, macos, linux, system, local]
author: Claw Team
---

# System Administration

## Overview

Procedural guide for local system administration — system health checks, process management, file operations, networking, package management, and scheduled tasks on macOS and Linux.

## System Health

### Quick health check

```bash
# System info
uname -a

# Uptime and load
uptime

# Disk usage
df -h

# Memory (macOS)
vm_stat | head -10
# Memory (Linux)
free -h

# CPU usage — top processes
ps aux --sort=-%cpu | head -10
```

### macOS specific

```bash
# System info
system_profiler SPSoftwareDataType SPHardwareDataType

# Battery status
pmset -g batt

# Wi-Fi info
/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport -I

# Open apps
osascript -e 'tell application "System Events" to get name of every process whose background only is false'
```

## Process Management

```bash
# Find a process
pgrep -fl <name>
ps aux | grep <name>

# Kill a process
kill <pid>
kill -9 <pid>  # force kill

# Kill by name
pkill <name>
killall <name>  # macOS
```

## File Management

```bash
# Find files
find / -name "*.log" -mtime -1 2>/dev/null  # modified in last day
find . -size +100M  # large files
mdfind <query>  # macOS Spotlight search

# Disk usage by directory
du -sh */ | sort -rh | head -10

# File permissions
ls -la <path>
chmod 755 <file>
chown user:group <file>
```

## Networking

```bash
# IP addresses
ifconfig | grep "inet "
# or
ip addr show  # Linux

# Test connectivity
ping -c 3 google.com

# DNS lookup
nslookup <domain>
dig <domain>

# Open ports
lsof -i -P -n | grep LISTEN  # macOS
ss -tlnp  # Linux

# Download a file
curl -O <url>
wget <url>  # if available
```

## Package Management

### macOS (Homebrew)

```bash
brew update
brew install <package>
brew list
brew upgrade
brew cleanup
```

### Ubuntu/Debian

```bash
sudo apt update
sudo apt install <package>
sudo apt list --installed
```

## Scheduled Tasks

### macOS (launchd)

```bash
# List running agents
launchctl list | grep -v "com.apple"

# Load/unload
launchctl load ~/Library/LaunchAgents/com.example.plist
launchctl unload ~/Library/LaunchAgents/com.example.plist
```

### Linux (cron/systemd)

```bash
crontab -l  # list cron jobs
crontab -e  # edit cron jobs
systemctl list-timers  # systemd timers
```

## Important Notes

- Check the OS before running commands (macOS vs Linux)
- Use `which <command>` to verify a tool is available before using it
- Be careful with `sudo` — verify the command is correct first
- For destructive operations (rm -rf, disk formatting), confirm with the operator
- Store commonly used server IPs, paths, and commands in memory
