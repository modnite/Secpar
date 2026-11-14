// Secpar 4 devices. A device is one installation, known by its certificate.
// The account is the single source of truth for access: a hosting device asks
// for its access list and gets it signed, so it can keep working from a cached
// copy while this service is unreachable.
import { importX509, exportJWK, jwtVerify, SignJWT, type JWTPayload } from "jose";
import { EFFECTIVE } from "./settings";

interface Environment { DB: D1Database; PUBLIC_ORIGIN: string }
interface Owner { id: string }
interface Helpers {
  body(request: Request): Promise<Record<string, unknown>>;
  fail(status: number, code: string): never;
  hash(value: string): Promise<string>;
  limit(request: Request, env: unknown, operation: string, maximum: number): Promise<void>;
  signing(): Promise<{ privateKey: CryptoKey | Uint8Array; kid: string }>;
}
const now = () => Math.floor(Date.now() / 1000);
const q = (env: Environment, sql: string, ...args: (string | number)[]) => env.DB.prepare(sql).bind(...args);
const random = () => Array.from(crypto.getRandomValues(new Uint8Array(32)), (b) => b.toString(16).padStart(2, "0")).join("");
const json = (data: unknown, status = 200) =>
  new Response(JSON.stringify(data), { status, headers: { "content-type": "application/json", "cache-control": "no-store", "x-content-type-options": "nosniff" } });

const PLATFORMS = ["windows", "linux", "android"];
const MAX_DEVICES = 50;
/** How long a host may rely on one signed access list. */
export const ACCESS_SECONDS = 7 * 24 * 3600;
type Device = { id: string; owner: string; fingerprint: string; name: string; platform: string; hosts: number; created: number; last_seen: number };
const shown = (d: Device) => ({ id: d.id, fingerprint: d.fingerprint, name: d.name, platform: d.platform, hosts: !!d.hosts, created: d.created, last_seen: d.last_seen });

/** Reads a device certificate and a token signed with its key. Returns the
 * certificate's fingerprint and the token's claims. */
async function proven(b: Record<string, unknown>, typ: string, base: string, api: Helpers): Promise<{ fingerprint: string; payload: JWTPayload }> {
  if (typeof b.certificate !== "string" || b.certificate.length > 4096 || typeof b.proof !== "string" || b.proof.length > 4096) api.fail(400, "invalid_device_proof");
  const match = b.certificate.trim().match(/^-----BEGIN CERTIFICATE-----\s+([A-Za-z0-9+/=\s]+)\s+-----END CERTIFICATE-----$/);
  if (!match) api.fail(400, "invalid_device_certificate");
  const encoded = match[1].replace(/\s/g, "");
  const der = Buffer.from(encoded, "base64");
  if (der.toString("base64") !== encoded) api.fail(400, "invalid_device_certificate");
  const fingerprint = Buffer.from(await crypto.subtle.digest("SHA-256", der)).toString("hex");
  try {
    // The certificate is self-signed. Trust never comes from the certificate
    // itself, only from proof of holding its private key plus what the caller checks.
    const key = await importX509(b.certificate, "ES256");
    const jwk = await exportJWK(key);
    if (jwk.kty !== "EC" || jwk.crv !== "P-256") api.fail(403, "invalid_device_proof");
    const { payload } = await jwtVerify(b.proof, key, { algorithms: ["ES256"], typ, issuer: "secpar-device", audience: base, maxTokenAge: 90, clockTolerance: 5, requiredClaims: ["exp", "iat", "sub"] });
    return { fingerprint, payload };
  } catch {
    return api.fail(403, "invalid_device_proof");
  }
}

/** The signed access list for one hosting device: the owner's other devices, and
 * the devices of accepted, unblocked friends with what each friend may do. */
async function accessList(env: Environment, device: Device, base: string, api: Helpers): Promise<Response> {
  const owner = device.owner;
  const own = await q(env, "SELECT fingerprint,name FROM devices WHERE owner=? AND id<>? ORDER BY created,id", owner, device.id).all<{ fingerprint: string; name: string }>();
  const guests = await q(
    env,
    `SELECT d.fingerprint,d.name,a.id AS account,a.handle,COALESCE(s.connect,'ask') AS connect,${EFFECTIVE}
     FROM friends f
     JOIN accounts a ON a.id=CASE WHEN f.a=? THEN f.b ELSE f.a END
     JOIN devices d ON d.owner=a.id
     LEFT JOIN friend_settings s ON s.owner=? AND s.friend=a.id
     WHERE (f.a=? OR f.b=?) AND f.state='accepted'
       AND NOT EXISTS(SELECT 1 FROM blocks WHERE (owner=? AND blocked=a.id) OR (owner=a.id AND blocked=?))
     ORDER BY a.handle,d.created,d.id`,
    owner, owner, owner, owner, owner, owner,
  ).all<{ fingerprint: string; name: string; account: string; handle: string; connect: string; keyboard: number; mouse: number }>();
  const issued = now(), expires = issued + ACCESS_SECONDS;
  const key = await api.signing();
  const access = await new SignJWT({
    owner,
    devices: own.results.map((d) => ({ fingerprint: d.fingerprint, name: d.name })),
    guests: guests.results.map((g) => ({ fingerprint: g.fingerprint, name: g.name, account: g.account, handle: g.handle, trusted: g.connect === "always", keyboard: !!g.keyboard, mouse: !!g.mouse })),
  })
    .setProtectedHeader({ alg: "ES256", typ: "secpar-access+jwt", kid: key.kid })
    .setIssuer(base)
    .setAudience("secpar-host")
    .setSubject(device.fingerprint)
    .setIssuedAt(issued)
    .setExpirationTime(expires)
    .sign(key.privateKey);
  await q(env, "UPDATE devices SET last_seen=? WHERE id=?", issued, device.id).run();
  return json({ access, expires_at: expires });
}

/** Routes a device uses with its own key and no signed-in person, so a hosting
 * computer can refresh its access list before anyone has logged in to it. */
export async function deviceKeyRoutes(request: Request, env: Environment, api: Helpers): Promise<Response | null> {
  const { pathname: path } = new URL(request.url), method = request.method;
  // The service's time, so a device with a wrong clock can still date its proof.
  if (path === "/v1/device/time" && method === "GET") return json({ now: now() });
  if (path !== "/v1/device/access" || method !== "POST") return null;
  await api.limit(request, env, "device_key_access", 120);
  const base = new URL(env.PUBLIC_ORIGIN).origin;
  const { fingerprint, payload } = await proven(await api.body(request), "secpar-device-request+jwt", base, api);
  if (payload.sub !== fingerprint || payload.purpose !== "secpar-access") api.fail(403, "invalid_device_proof");
  const device = await q(env, "SELECT * FROM devices WHERE fingerprint=? AND hosts=1", fingerprint).first<Device>();
  // A removed device and one that never hosted look the same from outside.
  if (!device) api.fail(404, "device_not_found");
  return accessList(env, device, base, api);
}

export async function deviceRoutes(request: Request, env: Environment, me: Owner, api: Helpers): Promise<Response | null> {
  const { pathname: path } = new URL(request.url), method = request.method;
  if (!path.startsWith("/v1/devices")) return null;
  const base = new URL(env.PUBLIC_ORIGIN).origin;

  if (path === "/v1/devices/challenge" && method === "POST") {
    await api.limit(request, env, "device_challenge", 20);
    const challenge = random(), issued = now();
    await q(env, "INSERT INTO device_challenges(hash,owner,expires) VALUES(?,?,?)", await api.hash(challenge), me.id, issued + 300).run();
    return json({ challenge, owner: me.id, origin: base, issued_at: issued, expires_at: issued + 300 }, 201);
  }

  if (path === "/v1/devices" && method === "POST") {
    await api.limit(request, env, "device_enrol", 20);
    // Trust comes from the signed-in owner's one-use challenge plus proof of
    // holding this certificate's private key.
    const { fingerprint, payload } = await proven(await api.body(request), "secpar-device+jwt", base, api);
    if (payload.sub !== me.id || payload.purpose !== "secpar-device" || payload.fingerprint !== fingerprint || typeof payload.challenge !== "string" || !/^[a-f0-9]{64}$/.test(payload.challenge))
      api.fail(403, "invalid_device_proof");
    const { name, platform, hosts } = payload;
    if (typeof name !== "string" || !name.trim() || name.length > 80 || typeof platform !== "string" || !PLATFORMS.includes(platform) || typeof hosts !== "boolean" || (hosts && platform === "android"))
      api.fail(400, "invalid_device_details");
    const existing = await q(env, "SELECT id,owner FROM devices WHERE fingerprint=?", fingerprint).first<{ id: string; owner: string }>();
    if (existing && existing.owner !== me.id) api.fail(409, "device_belongs_to_another_account");
    if (!existing) {
      const count = await q(env, "SELECT COUNT(*) AS n FROM devices WHERE owner=?", me.id).first<{ n: number }>();
      if ((count?.n ?? 0) >= MAX_DEVICES) api.fail(409, "too_many_devices");
    }
    // Use the challenge up before changing anything, so a replay does nothing.
    const used = await q(env, "DELETE FROM device_challenges WHERE hash=? AND owner=? AND expires>? RETURNING owner", await api.hash(payload.challenge as string), me.id, now()).first();
    if (!used) api.fail(409, "device_challenge_expired_or_used");
    const id = existing?.id ?? crypto.randomUUID();
    if (existing) await q(env, "UPDATE devices SET name=?,platform=?,hosts=?,last_seen=? WHERE id=? AND owner=?", name.trim(), platform, hosts ? 1 : 0, now(), id, me.id).run();
    else await q(env, "INSERT INTO devices(id,owner,fingerprint,name,platform,hosts,created,last_seen) VALUES(?,?,?,?,?,?,?,?)", id, me.id, fingerprint, name.trim(), platform, hosts ? 1 : 0, now(), now()).run();
    return json({ id, fingerprint, name: name.trim(), platform, hosts }, existing ? 200 : 201);
  }

  if (path === "/v1/devices" && method === "GET") {
    const rows = await q(env, "SELECT * FROM devices WHERE owner=? ORDER BY created,id", me.id).all<Device>();
    return json({ devices: rows.results.map(shown) });
  }

  const one = path.match(/^\/v1\/devices\/([a-f0-9-]{36})(\/access)?$/);
  if (!one) return null;
  const device = await q(env, "SELECT * FROM devices WHERE id=? AND owner=?", one[1], me.id).first<Device>();
  if (!device) api.fail(404, "device_not_found");

  if (!one[2] && method === "DELETE") {
    await q(env, "DELETE FROM devices WHERE id=? AND owner=?", device.id, me.id).run();
    return new Response(null, { status: 204 });
  }

  if (one[2] && method === "GET") {
    if (!device.hosts) api.fail(404, "device_not_found");
    await api.limit(request, env, "device_access", 120);
    return accessList(env, device, base, api);
  }
  return null;
}
