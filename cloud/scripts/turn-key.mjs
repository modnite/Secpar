// Makes a Cloudflare TURN key for the self-hosted relay, so friends on strict networks get
// relay addresses. Run by .github/workflows/selfhost-tunnel.yml on the build box. The key is
// written to a private file there and never printed.
import { writeFileSync } from "node:fs";

const token = process.env.CLOUDFLARE_API_TOKEN;
if (!token) throw new Error("CLOUDFLARE_API_TOKEN is not set");
const OUT = "/opt/secpar-accounts/inbox/turn-key";

async function api(path, method = "GET", body, step) {
  const r = await fetch(`https://api.cloudflare.com/client/v4${path}`, {
    method,
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok || j.success === false) {
    const why = (j.errors ?? []).map((e) => `${e.code} ${e.message}`).join("; ") || r.status;
    throw new Error(`${step ?? path}: ${why}`);
  }
  return j.result ?? j; // TURN key routes answer without the usual wrapper
}

const account = (await api("/accounts", "GET", undefined, "read the account"))[0].id;
const base = `/accounts/${account}/calls/turn_keys`;
const listed = await api(base, "GET", undefined, "list TURN keys");
for (const k of Array.isArray(listed) ? listed : []) {
  if (k?.name === "secpar-self-hosted" && k.uid) await api(`${base}/${k.uid}`, "DELETE", undefined, "remove an older TURN key");
}
const key = await api(`/accounts/${account}/calls/turn_keys`, "POST", { name: "secpar-self-hosted" }, "create the TURN key (needs Calls: Edit)");
if (!key?.uid || !key?.key) throw new Error("create the TURN key: no key in the answer");
writeFileSync(OUT, `TURN_KEY_ID=${key.uid}\nTURN_KEY_TOKEN=${key.key}\n`, { mode: 0o600 });
console.log(`TURN key secpar-self-hosted is ready; it is in ${OUT}.`);
