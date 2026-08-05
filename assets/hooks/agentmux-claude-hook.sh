#!/bin/sh
# agentmux claude hook — installed by `agentmux --install-hooks`.
#
# Claude Code spawns this script per lifecycle event with one action:
#   working  (UserPromptSubmit / PreToolUse / PostToolUse)
#   blocked  (Notification — permission prompts)
#   idle     (Stop — response finished)
#   release  (SessionEnd — session over)
#
# It reports to the agentmux loopback server, which resolves the session from
# $PPID (the claude process). Whenever agentmux is not running there is no
# port file and the hook exits 0 silently — it never blocks claude.
# AGENTMUX_HOOK_ID=claude

action="${1:-}"

case "$action" in
  working|blocked|idle|release) ;;
  *) exit 0 ;;
esac

port_file="${AGENTMUX_PORT_FILE:-$HOME/.config/agentmux/agentmux.port}"
[ -r "$port_file" ] || exit 0

port="$(cat "$port_file" 2>/dev/null)" || exit 0
case "$port" in
  ''|*[!0-9]*) exit 0 ;;
esac

command -v curl >/dev/null 2>&1 || exit 0

curl -sS -m 2 -X POST "http://127.0.0.1:${port}/report" \
  -H 'Content-Type: application/json' \
  -d "{\"pid\": ${PPID}, \"agent\": \"claude\", \"state\": \"${action}\"}" \
  >/dev/null 2>&1 || true

exit 0
