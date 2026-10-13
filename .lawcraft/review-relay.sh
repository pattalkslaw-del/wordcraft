#!/usr/bin/env bash
# Review relay on render-1: runs one review on Patrick's subscription logins (ChatGPT via codex, grok.com via the Grok CLI) so reviews don't bill the APIs. Installed as an SSH forced command for one key:
#   command="/home/patrick/bin/pixelprint-review-relay",restrict ssh-ed25519 AAAA... pixelprint-review-relay
# SSH_ORIGINAL_COMMAND = model id. stdin = full review prompt. stdout = one JSON line {text, tokens}.
# Tools are off / read-only sandbox in an empty temp dir: the model only reads the prompt.
set -euo pipefail
export PATH="$HOME/.local/bin:$HOME/.grok/bin:$HOME/.npm-global/bin:/usr/local/bin:/usr/bin:/bin:$PATH"
model=${SSH_ORIGINAL_COMMAND:-}
d=$(mktemp -d); trap 'rm -rf "$d"' EXIT
cd "$d"; cat > prompt.md
case "$model" in
  gpt-6.1-sol)
    codex exec --json --sandbox read-only --skip-git-repo-check -m "$model" -o out.md - < prompt.md > ev.jsonl 2> err.txt \
      || { tail -c 2000 err.txt >&2; exit 1; }
    node -e 'const fs=require("fs");let t=0;for(const l of fs.readFileSync("ev.jsonl","utf8").split("\n")){try{const e=JSON.parse(l);if(e.type==="turn.completed")t+=(e.usage.input_tokens||0)+(e.usage.output_tokens||0)}catch{}}console.log(JSON.stringify({text:fs.readFileSync("out.md","utf8"),tokens:t}))' ;;
  grok-4.7)
    # Grok CLI on Patrick's grok.com OAuth login. Grok can narrate before the verdict, so the text is cut to start at VERDICT:. No tools, no plan step, reviewer-only system prompt, at most 4 turns: the model only reads the prompt.
    grok --prompt-file prompt.md -m "$model" --output-format json --tools "" --no-plan \
      --system-prompt-override "You are a code reviewer. You have no tools and no files; everything you need is in the message. Answer directly in text." \
      --no-subagents --disable-web-search --max-turns 4 < /dev/null > out.json 2> err.txt \
      || { tail -c 2000 err.txt >&2; exit 1; }
    node -e 'const o=JSON.parse(require("fs").readFileSync("out.json","utf8"));const u=o.usage||{};let t=o.text||"";const i=t.indexOf("VERDICT:");if(i>0)t=t.slice(i);console.log(JSON.stringify({text:t,tokens:(u.input_tokens||0)+(u.output_tokens||0)}))' ;;
  *) echo "model not allowed: $model" >&2; exit 2 ;;
esac
