#!/usr/bin/env bash
# LawCraft goal loop on GitHub Actions (pattalkslaw-del/wordcraft, branch lawcraft-loop). Moved from Forgejo 2026-10-13.
# Issue labeled `goal` -> coder model builds on goal/issue-N through OpenCode, .lawcraft/test.sh (cargo xtask ci)
# runs, reviewer model(s) review the exact head. Findings go back to the coder.
# Coder: loop.json `coder` (Step 5 preview, free through OpenCode). After escalate_after_rounds failed rounds or a
# coder failure it escalates to `escalate_to` (DeepSeek, API) when DEEPSEEK_API_KEY is set.
# Reviewers run on the runner on Patrick's OAuth logins (Codex/ChatGPT, Grok/grok.com) from repository secrets.
# No OpenAI or xAI API keys are used. `sensitive` issues also get `second_reviewer`.
# PASS from every reviewer with tests not failing -> `goal-ready`. Never merges. After max_rounds -> `needs-orchestrator`.
set -euo pipefail

W=$(mktemp -d)  # scratch outside the checkout so nothing here gets committed
# The loop lives on the default branch (lawcraft-loop); goal branches come from main (= upstream) and never carry it.
L="$W/loop"; cp -r .lawcraft "$L"
API="https://api.github.com/repos/${GITHUB_REPOSITORY}"
TOK=$GH_TOKEN
export OPENCODE_CONFIG="$L/opencode.json"

jget() { node -e 'let o=JSON.parse(require("fs").readFileSync(0,"utf8"));for(const k of process.argv[1].split("."))o=o?.[k];console.log(o??"")' "$1"; }
jobj() { node -e 'const a=process.argv.slice(1),o={};for(let i=0;i<a.length;i+=2)o[a[i]]=a[i+1];console.log(JSON.stringify(o))' "$@"; }
api() { local method=$1 path=$2; shift 2
  curl -sS --fail-with-body -X "$method" -H "Authorization: Bearer $TOK" -H 'Accept: application/vnd.github+json' -H 'Content-Type: application/json' "$API$path" "$@"; }
comment() { api POST "/issues/$1/comments" -d "$(jobj body "${2:0:60000}")" >/dev/null; }
label() { api POST "/issues/$1/labels" -d "{\"labels\":[\"$2\"]}" >/dev/null; }
cfg() { jget "$1" < "$L/loop.json"; }

EV=$GITHUB_EVENT_PATH
ISSUE=$(jget issue.number < "$EV")
[ -z "$(jget issue.pull_request < "$EV")" ] || { echo "PR, not an issue"; exit 0; }
has_label() { node -e 'process.exit(require(process.argv[1]).issue.labels.some(l=>l.name===process.argv[2])?0:1)' "$EV" "$1"; }
ADDED=$(jget label.name < "$EV")
if [ -n "$ADDED" ]; then [ "$ADDED" = goal ]; else has_label goal; fi || { echo "not a goal label event"; exit 0; }
TITLE=$(jget issue.title < "$EV")
BODY=$(jget issue.body < "$EV")
BRANCH="goal/issue-$ISSUE"

# Per-issue overrides: body lines "Coder: provider/model" / "Reviewer: provider/model".
override() { printf '%s\n' "$BODY" | sed -nE "s/^$1:[[:space:]]*([^[:space:]]+).*/\1/p" | tail -1; }
CODER=$(override Coder); CODER=${CODER:-$(cfg coder)}
REVIEWER=$(override Reviewer); REVIEWER=${REVIEWER:-$(cfg reviewer)}
REVIEWERS="$REVIEWER"
if has_label sensitive && [ -n "$(cfg second_reviewer)" ]; then REVIEWERS="$REVIEWER $(cfg second_reviewer)"; fi
MAX_ROUNDS=$(cfg max_rounds)

# Token ledger on branch loop-ledger (time, model, issue, tokens, cached). Caps: loop.json token_caps.
ledger() { git fetch -q origin loop-ledger 2>/dev/null && git show FETCH_HEAD:ledger.tsv || true; }
spent() { ledger | awk -F'\t' -v m="$1" '$2==m{s+=$4} END{print s+0}'; }
cap() { node -e 'console.log(require(process.argv[2]).token_caps?.[process.argv[1]]??"")' "$1" "$L/loop.json"; }
ledger_add() {
  [ "${2:-0}" -gt 0 ] || return 0
  for _ in 1 2 3 4 5; do
    parent=$(git fetch -q origin loop-ledger 2>/dev/null && git rev-parse FETCH_HEAD || true)
    { [ -z "$parent" ] || git show "$parent:ledger.tsv"; printf '%s\t%s\t#%s\t%s\t%s\n' "$(date -u +%FT%TZ)" "$1" "$ISSUE" "$2" "${3:-}"; } > "$W/ledger.tsv"
    tree=$(printf '100644 blob %s\tledger.tsv\n' "$(git hash-object -w "$W/ledger.tsv")" | git mktree)
    c=$(GIT_AUTHOR_NAME="goal loop" GIT_COMMITTER_NAME="goal loop" git commit-tree "$tree" ${parent:+-p "$parent"} -m "ledger: #$ISSUE $1 $2")
    git push -q origin "$c:refs/heads/loop-ledger" && return 0
    sleep $((RANDOM % 5 + 1))
  done
  echo "ledger push failed: $1 $2" >&2; return 1
}
check_cap() {
  local c; c=$(cap "$1"); [ -n "$c" ] || return 0
  local s; s=$(spent "$1")
  [ "$s" -lt "$c" ] && return 0
  comment "${PR:-$ISSUE}" "Goal loop stopped: \`$1\` has used $s of its $c-token cap. Labeled \`needs-orchestrator\`."
  label "$ISSUE" needs-orchestrator; exit 1
}

ESCALATE_TO=$(cfg escalate_to); ESCALATE_AFTER=$(cfg escalate_after_rounds)
case "$ESCALATE_TO" in deepseek-api/*) [ -n "${DEEPSEEK_API_KEY:-}" ] || ESCALATE_TO="" ;; esac
[ "$ESCALATE_TO" = "$CODER" ] && ESCALATE_TO=""
# Coder harness: cursor/<model> runs Cursor's CLI on Patrick's Cursor login (secret CURSOR_AUTH_JSON, OAuth tokens,
# not an API key); anything else runs through OpenCode.
setup_coder() {
  case "$CODER" in
    cursor/*)
      [ -n "${CURSOR_AUTH_JSON:-}" ] || { comment "$ISSUE" "Goal loop not started: coder \`$CODER\` needs repository secret \`CURSOR_AUTH_JSON\`."; exit 1; }
      command -v cursor-agent >/dev/null || [ -x "$HOME/.local/bin/cursor-agent" ] || curl -fsS https://cursor.com/install | bash >/dev/null
      export PATH="$HOME/.local/bin:$PATH"
      mkdir -p "$HOME/.config/cursor"; (umask 077; printenv CURSOR_AUTH_JSON > "$HOME/.config/cursor/auth.json") ;;
    deepseek-api/*)
      [ -n "${DEEPSEEK_API_KEY:-}" ] || { comment "$ISSUE" "Goal loop not started: coder \`$CODER\` needs repository secret \`DEEPSEEK_API_KEY\`."; exit 1; }
      command -v opencode >/dev/null || npm install -g --silent opencode-ai ;;
    *) command -v opencode >/dev/null || npm install -g --silent opencode-ai ;;
  esac
}
# Writes $W/build.md (final summary) and $W/build.err; sets ok and used ("tokens cached").
run_coder() {
  case "$CODER" in
    cursor/*)
      # Cursor's CLI waits on a terminal in print mode; script(1) gives it one.
      printf '%s' "$1" > "$W/prompt.txt"
      # Cursor's CLI sometimes dies mid-round with no error (dropped connection). Retry once, keeping the edits.
      for attempt in 1 2; do
        timeout 5400 script -qfec "cursor-agent -p --force --trust --sandbox disabled --output-format text --model '${CODER#cursor/}' \"\$(cat '$W/prompt.txt')\"" /dev/null \
          </dev/null > "$W/build.raw" 2> "$W/build.err" && ok=1 || ok=0
        [ "$ok" = 1 ] && break
        printf '\n\nYour previous attempt at this round was cut off. The working tree keeps its edits; continue from there.\n' >> "$W/prompt.txt"
      done
      sed 's/\x1b\[[0-9;?]*[a-zA-Z]//g; s/\r$//' "$W/build.raw" | grep -v '^Tip: ' > "$W/build.md" || true
      : > "$W/build.jsonl"; used="0 0" ;;
    *)
      # opencode headless hangs without </dev/null; --format json gives per-step token totals and the final text.
      opencode run --pure --dangerously-skip-permissions --format json -m "$CODER" </dev/null "$1" \
        > "$W/build.jsonl" 2> "$W/build.err" && ok=1 || ok=0
      used=$(node -e 'let t=0,c=0,txt=[];for(const l of require("fs").readFileSync(process.argv[1],"utf8").split("\n")){try{const e=JSON.parse(l);if(e.type==="step_finish"){t+=e.part.tokens?.total||0;c+=e.part.tokens?.cache?.read||0}if(e.type==="text"&&e.part?.text)txt.push(e.part.text)}catch{}}require("fs").writeFileSync(process.argv[2],txt.join("\n\n"));console.log(t,c)' "$W/build.jsonl" "$W/build.md") ;;
  esac
}
setup_coder
escalate() {  # $1 = why; returns 1 when there is nowhere to escalate
  [ -n "$ESCALATE_TO" ] && [ "$CODER" != "$ESCALATE_TO" ] || return 1
  comment "${PR:-$ISSUE}" "Escalating: $1 Coder \`$CODER\` -> \`$ESCALATE_TO\` for the remaining rounds."
  CODER=$ESCALATE_TO; setup_coder
}

# Reviewer logins (OAuth files from secrets; no API keys).
mkdir -p "$HOME/.codex" "$HOME/.grok"
(umask 077; [ -z "${CODEX_AUTH_JSON:-}" ] || printenv CODEX_AUTH_JSON > "$HOME/.codex/auth.json"; [ -z "${GROK_AUTH_JSON:-}" ] || printenv GROK_AUTH_JSON > "$HOME/.grok/auth.json")

git config user.email "goal-loop@agents.nemolegal.internal"; git config user.name "goal loop"
if git ls-remote --exit-code origin "refs/heads/$BRANCH" >/dev/null; then
  git fetch -q origin "$BRANCH"; git checkout -q -B "$BRANCH" "origin/$BRANCH"
else
  git checkout -q -B "$BRANCH" origin/main
fi
PR=$(api GET "/pulls?state=open&per_page=100" \
  | node -e 'const p=JSON.parse(require("fs").readFileSync(0,"utf8")).find(p=>p.head.ref===process.argv[1]);console.log(p?p.number:"")' "$BRANCH")

comment "$ISSUE" "Goal loop started on \`$BRANCH\`: coder \`$CODER\`, reviewer(s) \`$REVIEWERS\`. ${ESCALATE_TO:+Escalates to \`$ESCALATE_TO\` after $ESCALATE_AFTER failed rounds or a coder failure. }Stop after $MAX_ROUNDS. Nothing is merged automatically."

feedback="None. This is the first round."
if [ -n "$PR" ]; then
  # Resuming an existing branch: carry the last round's reviews and test result into round 1 of this run.
  prev=$(api GET "/issues/$PR/comments?per_page=100" | node -e 'const c=JSON.parse(require("fs").readFileSync(0,"utf8")).map(x=>x.body);const r=c.filter(b=>/^\*\*Round \d+ review\*\*/.test(b));if(!r.length)process.exit(0);const n=r[r.length-1].match(/^\*\*Round (\d+)/)[1];console.log(r.filter(b=>b.startsWith("**Round "+n+" review**")).join("\n\n"))' || true)
  [ -z "$prev" ] || feedback="This branch already has work from an earlier run. Its last review:
$prev"
fi
for round in $(seq 1 "$MAX_ROUNDS"); do
  if [ -n "$ESCALATE_AFTER" ] && [ "$round" -eq $((ESCALATE_AFTER + 1)) ]; then
    escalate "no clean review after $ESCALATE_AFTER rounds." || true
  fi
  before=$(git rev-parse HEAD)
  check_cap "$CODER"
  prompt="You are the builder for LawCraft, legal tools being built into WordCraft (a clean-room Microsoft Word reimplementation in Rust).
Read AGENTS.md (WordCraft's rules: clean room, no panics, every feature a CommandSpec with tests, cargo xtask ci green) first and follow it, and follow the LawCraft spec below.
Implement GitHub issue #$ISSUE in this checkout (branch $BRANCH). Do not commit, push, or switch branches; the loop does that.
Do not add or edit anything under .lawcraft/ or .github/workflows/.

LawCraft spec (LAWCRAFT.md):
$(cat "$L/LAWCRAFT.md")

Issue title: $TITLE
Issue body:
$BODY

Feedback from the previous round (fix every blocking finding and failing test):
$feedback

End with a short summary: what changed, which tests you ran and their results, and anything you did not do and why."
  run_coder "$prompt"
  ledger_add "$CODER" $used
  if [ "$ok" != 1 ] && escalate "coder \`$CODER\` failed in round $round ($(tail -c 300 "$W/build.err"; grep -h '"type":"error"' "$W/build.jsonl" 2>/dev/null | tail -1 | cut -c1-300))."; then
    git reset -q --hard HEAD && git clean -qfd; continue
  fi
  [ "$ok" = 1 ] || { comment "${PR:-$ISSUE}" "Goal loop stopped: coder \`$CODER\` failed in round $round.
\`\`\`
$(tail -40 "$W/build.err"; grep -h '"type":"error"' "$W/build.jsonl" 2>/dev/null | tail -5 | cut -c1-2000)
\`\`\`"; label "$ISSUE" needs-orchestrator; exit 1; }
  touch "$W/build.md"

  # The loop's own files are never part of a goal change.
  rm -rf .lawcraft; git checkout -q HEAD -- .github 2>/dev/null || true; git clean -qfd -- .github
  git add -A
  if git diff --cached --quiet && escalate "coder \`$CODER\` made no changes in round $round."; then
    git reset -q; continue
  fi
  if git diff --cached --quiet && [ "$(git rev-list --count origin/main..HEAD)" -gt 0 ] && [ -z "${reviewed_head:-}" ]; then
    # Nothing new, but the branch already carries unreviewed work (an earlier run, or a coder swap): test and review it.
    skip_commit=1
  elif git diff --cached --quiet; then
    comment "${PR:-$ISSUE}" "Goal loop stopped: coder \`$CODER\` made no changes in round $round.

$(tail -80 "$W/build.md")"
    label "$ISSUE" needs-orchestrator; exit 1
  fi
  if [ -z "${skip_commit:-}" ]; then
    git -c user.name="$CODER (goal loop)" commit -q -m "goal #$ISSUE: round $round" -m "$TITLE" -m "Coder: $CODER"
    git push -q origin "$BRANCH"
  fi
  skip_commit=""
  HEAD_SHA=$(git rev-parse HEAD); reviewed_head=$HEAD_SHA

  if [ -z "$PR" ]; then
    PR=$(api POST /pulls -d "$(jobj head "$BRANCH" base main title "goal #$ISSUE: $TITLE" body "Closes #$ISSUE

Opened by the LawCraft goal loop. \`goal-ready\` means every reviewer passed the exact head with tests not failing; Patrick merges.")" | jget number)
  fi
  comment "$PR" "**Round $round build** by \`$CODER\` ($before → $HEAD_SHA)

$(tail -80 "$W/build.md")"

  if bash "$L/test.sh" > "$W/test.log" 2>&1; then tests=passed; else tests=failed; fi
  git reset -q --hard HEAD && git clean -qfd -e target  # drop test debris (CARGO_TARGET_DIR is outside the checkout)

  printf 'GitHub issue #%s, head %s\nIssue title: %s\nIssue body:\n%s\n\nBuilder summary:\n%s\n\n.lawcraft/test.sh (cargo xtask ci): %s\n%s\n' \
    "$ISSUE" "$HEAD_SHA" "$TITLE" "$BODY" "$(tail -80 "$W/build.md")" "$tests" "$(tail -60 "$W/test.log")" > "$W/context.md"

  pass=1; feedback=""
  for rv in $REVIEWERS; do
    check_cap "$rv"
    if ! LAWCRAFT_DIR="$L" USAGE_OUT="$W/usage" node "$L/review.mjs" "$rv" "$W/context.md" > "$W/review.md" 2> "$W/review.err"; then
      comment "$PR" "Goal loop stopped: reviewer \`$rv\` failed in round $round.
\`\`\`
$(tail -20 "$W/review.err")
\`\`\`"; label "$ISSUE" needs-orchestrator; exit 1
    fi
    ledger_add "$rv" "$(cat "$W/usage" 2>/dev/null || echo 0)"
    comment "$PR" "**Round $round review** by \`$rv\` of \`$HEAD_SHA\` (tests: $tests)

$(cat "$W/review.md")"
    head -1 "$W/review.md" | grep -qx 'VERDICT: PASS' || pass=0
    feedback="$feedback
Review by $rv of $HEAD_SHA:
$(cat "$W/review.md")
"
  done

  if [ "$pass" = 1 ] && [ "$tests" = passed ]; then
    label "$PR" goal-ready
    comment "$PR" "Review passed \`$HEAD_SHA\` in round $round with \`cargo xtask ci\` green. Labeled \`goal-ready\`. Waiting on Patrick to accept and merge."
    exit 0
  fi
  feedback="$feedback
.lawcraft/test.sh (cargo xtask ci): $tests
$(tail -60 "$W/test.log")"
done

label "$ISSUE" needs-orchestrator
comment "${PR:-$ISSUE}" "Goal loop stopped after $MAX_ROUNDS rounds without a clean review. Labeled \`needs-orchestrator\`: split or re-spec this, or set a \`Coder:\`/\`Reviewer:\` line in the issue body."
exit 1
