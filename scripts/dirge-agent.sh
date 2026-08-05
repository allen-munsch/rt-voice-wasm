#!/usr/bin/env bash
# dirge-agent.sh — stdin-to-dirge bridge for ProcessAgent
#
# Reads transcript lines from stdin, routes each through dirge with the
# phone-receptionist prompt, and writes the JSON action to stdout.
#
# Usage: rt-voice-server --agent-hook './scripts/dirge-agent.sh'
#
# Each stdin line → dirge → one JSON line on stdout.
# Exit when stdin closes (EOF) or dirge returns a terminal action.
set -euo pipefail

while IFS= read -r transcript; do
    # Skip empty lines
    [[ -z "${transcript}" ]] && continue

    result=$(dirge --print --output-format json \
        --prompt phone-receptionist \
        --model deepseek-v4-flash \
        "Caller transcript: ${transcript}" 2>/dev/null) || {
        echo '{"Continue": null}'
        continue
    }

    # Extract the inner action from dirge's JSON wrapper
    action=$(echo "${result}" | jq -r '.result // empty' 2>/dev/null)
    if [[ -z "${action}" ]]; then
        # Try parsing the whole line as a raw action (fallback)
        action="${result}"
    fi

    echo "${action}"
done
