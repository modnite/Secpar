// Puts relay.secpar.win back on its Worker: the Worker's own storage holds every host's relay key,
// which a self-hosted relay cannot copy. The account service stays on the tunnel.
const token = process.env.CLOUDFLARE_API_TOKEN;
if (!token) throw new Error("CLOUDFLARE_API_TOKEN is not set");
const HOST = "relay.secpar.win";

async function api(path, method = "GET", body, step) {
  const r = await fetch(`https://api.cloudflare.com/client/v4${path}`, {
    method,
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok || j.success === false) throw new Error(`${step ?? path}: ${(j.errors ?? []).map((e) => `${e.code} ${e.message}`).join("; ") || r.status}`);
  return j.result;
}

const account = (await api("/accounts"))[0].id;
const zone = (await api("/zones?name=secpar.win"))[0].id;
const tunnel = (await api(`/accounts/${account}/cfd_tunnel?name=secpar-home&is_deleted=false`))[0];
if (tunnel) {
  await api(`/accounts/${account}/cfd_tunnel/${tunnel.id}/configurations`, "PUT", {
    config: { ingress: [{ hostname: "accounts.secpar.win", service: "http://127.0.0.1:8787", originRequest: {} }, { service: "http_status:404" }] },
  }, "route only the account service through the tunnel");
}
for (const r of await api(`/zones/${zone}/dns_records?name=${HOST}`)) await api(`/zones/${zone}/dns_records/${r.id}`, "DELETE", undefined, "remove the tunnel record");
await api(`/accounts/${account}/workers/domains`, "PUT", { hostname: HOST, service: "secpar-rendezvous", environment: "production", zone_id: zone }, "give the name back to the Worker");
console.log(`${HOST} is on its Worker again.`);
