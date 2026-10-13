// Independent review of origin/main...HEAD by one model; prints the review to stdout.
// Usage: node scripts/review.mjs <provider/model> <context-file>
// Provider endpoints and keys come from .lawcraft/opencode.json, so there is one list of models.
// LAWCRAFT_DIR points at the .lawcraft directory of the checkout (set by goal.sh).
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { execSync, execFileSync } from "node:child_process";
import http from "node:http";
import https from "node:https";

// node:http(s) instead of fetch: fetch gives up if response headers take >5 min, and slow reasoners (Grok) do.
const post = (url, headers, body) => new Promise((resolve, reject) => {
  const u = new URL(url);
  const req = (u.protocol === "https:" ? https : http).request(u,
    { method: "POST", headers: { ...headers, "Content-Length": Buffer.byteLength(body) } },
    (res) => { let d = ""; res.setEncoding("utf8"); res.on("data", (c) => (d += c)); res.on("end", () => resolve({ status: res.statusCode, text: d })); });
  req.on("error", reject);
  req.setTimeout(30 * 60e3, () => req.destroy(new Error("no response in 30 min")));
  req.end(body);
});

const [spec, contextFile] = process.argv.slice(2);
const [provider, ...rest] = spec.split("/");
const model = rest.join("/");
// oauth/<model>: reviewed through review-relay.sh on Patrick's subscription logins (Codex, Grok CLIs).
const cfg = provider === "oauth" ? null : JSON.parse(readFileSync(path.join(process.env.LAWCRAFT_DIR, "opencode.json"), "utf8")).provider[provider];
if (!cfg && provider !== "oauth") throw new Error(`provider ${provider} not in opencode.json`);
const env = (s) => s.replace(/\{env:(\w+)\}/g, (_, k) => process.env[k] ?? "");
const loop = JSON.parse(readFileSync(path.join(process.env.LAWCRAFT_DIR, "loop.json"), "utf8"));

const sh = (c) => execSync(c, { encoding: "utf8", maxBuffer: 1 << 28 });
const diff = sh("git diff origin/main...HEAD");
const context = readFileSync(contextFile, "utf8");
// Whole repo as context, capped; the diff and task always go in full.
// Codex (oauth/gpt-*) rejects a turn over 1,048,576 characters, so the repo shrinks to fit there.
let budget = loop.review_context_chars, repo = "";
if (spec.startsWith("oauth/gpt-")) budget = Math.min(budget, 1_000_000 - context.length - diff.length - 4_000);
for (const f of sh("git ls-files").split("\n").filter(Boolean)) {
  let body;
  try { body = readFileSync(f, "utf8"); } catch { continue; }
  if (body.includes("\0")) continue; // binary
  const chunk = `\n===== ${f}\n${body}`;
  if (chunk.length > budget) { repo += `\n===== (repo context truncated at ${f})`; break; }
  repo += chunk; budget -= chunk.length;
}

const system = `You are the independent code reviewer for LawCraft, legal tools being built into WordCraft (a clean-room Microsoft Word reimplementation in Rust). You did not write this code.
The first line of your reply must be exactly "VERDICT: PASS" (no blocking findings) or "VERDICT: FAIL".
Then: BLOCKING FINDINGS (severity, file:line, evidence, required fix), NONBLOCKING FINDINGS,
TEST/CLAIM AUDIT (are the builder's test claims evidenced; what was not run).
Judge against the issue, .lawcraft/LAWCRAFT.md and AGENTS.md (WordCraft's rules: clean room, no panics, every feature a CommandSpec with tests). A change that adds or edits .lawcraft/ or .github/workflows/ is a blocking finding. Do not invent requirements.`;
const user = `${context}\n\n##### DIFF (origin/main...HEAD)\n${diff}\n\n##### REPOSITORY\n${repo}`;

if (provider === "oauth") {
  // Runs the same relay script render-1 used, locally on the runner, with the OAuth logins goal.sh wrote.
  const raw = execFileSync("bash", [path.join(process.env.LAWCRAFT_DIR, "review-relay.sh")],
    { input: `${system}\n\n${user}`, encoding: "utf8", maxBuffer: 1 << 28, timeout: 60 * 60e3, env: { ...process.env, SSH_ORIGINAL_COMMAND: model } });
  const o = JSON.parse(raw);
  if (process.env.USAGE_OUT) writeFileSync(process.env.USAGE_OUT, String(o.tokens ?? 0));
  if (!o.text?.trim()) throw new Error(`${spec}: empty review`);
  console.log(o.text.trim());
  process.exit(0);
}
const res = await post(`${cfg.options.baseURL.replace(/\/$/, "")}/chat/completions`,
  { "Content-Type": "application/json", Authorization: `Bearer ${env(cfg.options.apiKey)}` },
  JSON.stringify({ model, messages: [{ role: "system", content: system }, { role: "user", content: user }] }));
if (res.status !== 200) throw new Error(`${spec}: HTTP ${res.status} ${res.text}`);
const data = JSON.parse(res.text);
if (process.env.USAGE_OUT) writeFileSync(process.env.USAGE_OUT, String((data.usage?.prompt_tokens ?? 0) + (data.usage?.completion_tokens ?? 0)));
const out = data.choices?.[0]?.message?.content?.trim();
if (!out) throw new Error(`${spec}: empty review`);
console.log(out);
