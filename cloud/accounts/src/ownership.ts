/** Owner-only bootstrap. Long-term device tokens never pass through this service.
 * Each short-lived result is encrypted at rest and consumed by its requesting
 * account session only. The host's current enrollment binds every request.
 */
import { isIP } from "node:net";

export interface Environment {
  DB: D1Database;
  GRANT_PRIVATE_JWK: string;
}
export interface Helpers {
  body(request: Request): Promise<Record<string, unknown>>;
  fail(status: number, code: string): never;
  hash(value: string): Promise<string>;
  limit(request: Request, env: unknown, operation: string, maximum: number): Promise<void>;
}
export interface Machine {
  computer: string;
  owner: string;
  certificate_fp: string;
  credential_hash: string;
}
type Pair = {
  id: string; computer: string; owner: string; requester_hash: string;
  host_key_hash: string; certificate_fp: string; device_name: string;
  expires: number; sealed: string | null;
};
const now = () => Math.floor(Date.now() / 1000);
const q = (env: Environment, sql: string, ...args: (string | number)[]) => env.DB.prepare(sql).bind(...args);
const json = (data: unknown, status = 200) => new Response(JSON.stringify(data), {
  status, headers: { "content-type": "application/json", "cache-control": "no-store", "x-content-type-options": "nosniff" },
});
const aad = (row: Pair) => new TextEncoder().encode(JSON.stringify([
  "secpar-owner-pairing-v1", row.id, row.computer, row.owner,
  row.requester_hash, row.host_key_hash, row.certificate_fp, row.expires,
]));
async function key(env: Environment): Promise<CryptoKey> {
  // Canonical private scalar survives JSON formatting changes. Purpose separation
  // prevents reuse of the ES256 signing operation/key material as an AES key.
  const jwk = JSON.parse(env.GRANT_PRIVATE_JWK);
  if (jwk.kty !== "EC" || jwk.crv !== "P-256" || typeof jwk.d !== "string" || !/^[\w-]{43}$/.test(jwk.d)) throw new Error("Invalid signing configuration");
  const material = await crypto.subtle.importKey("raw", Buffer.from(jwk.d, "base64url"), "HKDF", false, ["deriveKey"]);
  return crypto.subtle.deriveKey({ name: "HKDF", hash: "SHA-256", salt: new TextEncoder().encode("secpar-accounts"), info: new TextEncoder().encode("owner-pairing-at-rest-v1") }, material, { name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]);
}
async function seal(env: Environment, row: Pair, link: string): Promise<string> {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const bytes = await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: aad(row) }, await key(env), new TextEncoder().encode(link));
  return `${Buffer.from(iv).toString("base64url")}.${Buffer.from(bytes).toString("base64url")}`;
}
async function open(env: Environment, row: Pair): Promise<string> {
  const [iv, bytes, extra] = row.sealed!.split(".");
  if (!iv || !bytes || extra || Buffer.from(iv, "base64url").length !== 12) throw new Error("Invalid envelope");
  return new TextDecoder().decode(await crypto.subtle.decrypt({ name: "AES-GCM", iv: Buffer.from(iv, "base64url"), additionalData: aad(row) }, await key(env), Buffer.from(bytes, "base64url")));
}
function validLink(value: unknown, fingerprint: string): value is string {
  if (typeof value !== "string" || value.length > 8192) return false;
  try {
    const u = new URL(value);
    if (u.protocol !== "secpar:" || u.hostname !== "pair" || u.pathname || u.username || u.password || u.hash || u.port) return false;
    const keys = [...u.searchParams.keys()];
    if (new Set(keys).size !== keys.length || u.searchParams.get("fp") !== fingerprint) return false;
    const port = Number(u.searchParams.get("p"));
    const otp = u.searchParams.get("t") ?? "";
    const hosts = (u.searchParams.get("h") ?? "").split(",").filter(Boolean);
    if (!Number.isInteger(port) || port < 1 || port > 65535 || !otp || otp.length > 256 || hosts.length > 16 || hosts.some(h => !isIP(h))) return false;
    const relay = u.searchParams.get("r");
    if (relay) {
      const r = new URL(relay), pc = u.searchParams.get("pc");
      if (r.protocol !== "wss:" || r.pathname !== "/v1/phone" || r.username || r.password || r.hash || !pc || r.searchParams.get("pc") !== pc) return false;
    }
    return hosts.length > 0 || !!relay;
  } catch { return false; }
}
const active = "p.expires>? AND p.host_key_hash=k.credential_hash AND p.certificate_fp=k.certificate_fp AND EXISTS(SELECT 1 FROM sessions s WHERE s.hash=p.requester_hash AND s.user_id=p.owner AND s.expires>CAST(strftime('%s','now') AS INTEGER))";

export async function ownerRoutes(request: Request, env: Environment, owner: string | null, machine: Machine | null, api: Helpers): Promise<Response | null> {
  const { pathname: path } = new URL(request.url), method = request.method;
  if (machine && path === "/v1/host/owner-pairings" && method === "GET") {
    const rows = await q(env, `SELECT p.id,p.device_name,p.expires FROM owner_pairings p JOIN host_keys k ON k.computer=p.computer WHERE p.computer=? AND p.owner=? AND ${active} AND p.sealed IS NULL ORDER BY p.expires LIMIT 8`, machine.computer, machine.owner, now()).all();
    return json(rows.results);
  }
  const delivery = path.match(/^\/v1\/host\/owner-pairings\/([a-f0-9-]{36})$/);
  if (machine && delivery && method === "POST") {
    await api.limit(request, env, "owner_pair_delivery", 60);
    const row = await q(env, `SELECT p.* FROM owner_pairings p JOIN host_keys k ON k.computer=p.computer WHERE p.id=? AND p.computer=? AND p.owner=? AND ${active} AND p.sealed IS NULL`, delivery[1], machine.computer, machine.owner, now()).first<Pair>();
    if (!row) api.fail(404, "pairing_request_unavailable");
    const body = await api.body(request);
    if (!validLink(body.link, row.certificate_fp)) api.fail(400, "invalid_pairing_result");
    const sealed = await seal(env, row, body.link);
    const changed = await q(env, "UPDATE owner_pairings SET sealed=? WHERE id=? AND sealed IS NULL AND expires>? AND host_key_hash=(SELECT credential_hash FROM host_keys WHERE computer=?) RETURNING id", sealed, row.id, now(), machine.computer).first();
    if (!changed) api.fail(409, "pairing_request_unavailable");
    return json({ ready: true });
  }
  if (!owner) return null;
  if (path === "/v1/computers" && method === "GET") {
    const rows = await q(env, "SELECT c.id,c.name,c.platform,k.certificate_fp,COALESCE(k.last_seen,0) AS last_seen,CASE WHEN k.computer IS NOT NULL THEN 1 ELSE 0 END AS linked,CASE WHEN k.last_seen>? THEN 1 ELSE 0 END AS online FROM computers c LEFT JOIN host_keys k ON k.computer=c.id WHERE c.owner=? ORDER BY c.name,c.id", now() - 90, owner).all();
    return json(rows.results);
  }
  if (!path.startsWith("/v1/owner-pairings")) return null;
  // The native device session is the recipient capability. A second signed-in
  // device cannot consume this device's temporary result, even on one account.
  const token = request.headers.get("authorization")?.match(/^Bearer ([a-f0-9]{64})$/)?.[1];
  if (!token) api.fail(401, "native_session_required");
  const requester = await api.hash(token);
  if (path === "/v1/owner-pairings" && method === "POST") {
    await api.limit(request, env, "owner_pair_start", 20);
    const b = await api.body(request);
    if (typeof b.computer !== "string" || typeof b.device !== "string" || !b.device.trim() || b.device.length > 80 || /[\u0000-\u001f\u007f]/.test(b.device)) api.fail(400, "invalid_pairing_request");
    const host = await q(env, "SELECT c.id,k.credential_hash,k.certificate_fp FROM computers c JOIN host_keys k ON k.computer=c.id WHERE c.id=? AND c.owner=?", b.computer, owner).first<{id:string;credential_hash:string;certificate_fp:string}>();
    if (!host) api.fail(404, "computer_not_found");
    await q(env, "DELETE FROM owner_pairings WHERE expires<=? OR NOT EXISTS(SELECT 1 FROM host_keys k WHERE k.computer=owner_pairings.computer AND k.credential_hash=owner_pairings.host_key_hash)", now()).run();
    const id = crypto.randomUUID(), expires = now() + 120;
    const inserted = await q(env, "INSERT INTO owner_pairings(id,computer,owner,requester_hash,host_key_hash,certificate_fp,device_name,expires) SELECT ?,?,?,?,?,?,?,? WHERE (SELECT count(*) FROM owner_pairings WHERE owner=? AND expires>?)<8 AND EXISTS(SELECT 1 FROM host_keys WHERE computer=? AND credential_hash=?) RETURNING id", id, host.id, owner, requester, host.credential_hash, host.certificate_fp, b.device.trim(), expires, owner, now(), host.id, host.credential_hash).first();
    if (!inserted) api.fail(429, "too_many_pairing_requests");
    return json({ id, expires, state: "pending" }, 201);
  }
  const result = path.match(/^\/v1\/owner-pairings\/([a-f0-9-]{36})$/);
  if (result && (method === "GET" || method === "DELETE")) {
    const row = await q(env, `SELECT p.* FROM owner_pairings p JOIN host_keys k ON k.computer=p.computer WHERE p.id=? AND p.owner=? AND p.requester_hash=? AND ${active}`, result[1], owner, requester, now()).first<Pair>();
    if (!row) api.fail(404, "pairing_request_unavailable");
    if (method === "DELETE") {
      await q(env, "DELETE FROM owner_pairings WHERE id=? AND requester_hash=?", row.id, requester).run();
      return json({ cancelled: true });
    }
    if (!row.sealed) return json({ id: row.id, state: "pending", expires: row.expires }, 202);
    const consumed = await q(env, "DELETE FROM owner_pairings WHERE id=? AND requester_hash=? AND expires>? AND sealed IS NOT NULL AND host_key_hash=(SELECT credential_hash FROM host_keys WHERE computer=?) RETURNING *", row.id, requester, now(), row.computer).first<Pair>();
    if (!consumed) api.fail(404, "pairing_request_unavailable");
    try { return json({ id: row.id, link: await open(env, consumed), certificate_fp: row.certificate_fp, expires: row.expires }); }
    catch { return api.fail(409, "pairing_result_unavailable"); }
  }
  return json({ error: "not_found" }, 404);
}
