#!/usr/bin/env bash

set -euo pipefail

MODEL="${1:-gemma4-12b}"

curl -sS -N http://localhost:1234/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d "{
    \"model\": \"${MODEL}\",
    \"messages\": [
      {
        \"role\": \"system\",
        \"content\": \"You are a helpful assistant. Do not show reasoning. Answer directly.\"
      },
      {
        \"role\": \"user\",
        \"content\": \"Bonjour\"
      }
    ],
    \"stream\": false
  }" | jq .