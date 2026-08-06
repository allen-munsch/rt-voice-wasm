#!/usr/bin/env bash
# dirge-coding-agent.sh — voice-to-dirge bridge for code editing via ProcessAgent
#
# Reads transcript lines from stdin, routes each through dirge with the
# voice-coding prompt and --accept-all (auto-executes tools), then writes
# the JSON action to stdout.
#
# Usage: rt-voice-server --agent-hook './scripts/dirge-coding-agent.sh'
#
# Each stdin line → dirge (voice-coding prompt) → one JSON action on stdout.
# Exit when stdin closes (EOF).
set -euo pipefail

while IFS= read -r transcript; do
    # Skip empty lines
    [[ -z "${transcript}" ]] && continue

    result=$(dirge --print --output-format json \
        --prompt voice-coding \
        --accept-all \
        --model deepseek-v4-flash \
        "Voice command: ${transcript}" 2>/dev/null) || {
        echo '{"Continue":null}'
        continue
    }

    # Extract the result text from dirge's JSON wrapper
    reply=$(echo "${result}" | jq -r '.result // empty' 2>/dev/null)
    if [[ -n "${reply}" ]]; then
        # Escape for JSON string: backslash, double-quote, newline
        escaped=$(echo "${reply}" | jq -Rs '.' 2>/dev/null)
        echo "{\"Respond\":${escaped}}"
    else
        echo '{"Continue":null}'
    fi
done
