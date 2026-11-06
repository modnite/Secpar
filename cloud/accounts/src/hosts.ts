import { importX509, exportJWK, calculateJwkThumbprint, jwtVerify, type JWTPayload } from "jose";
import { ownerRoutes } from "./ownership";

interface Environment { DB: D1Database; PUBLIC_ORIGIN: string; GRANT_PRIVATE_JWK: string }
interface Owner { id: string }
interface Helpers {
  body(request: Request): Promise<Record<string, unknown>>;
  fail(status: number, code: string): never;
  hash(value: string): Promise<string>;
  limit(request: Request, env: unknown, operation: string, maximum: number): Promise<void>;
}
const now = () => Math.floor(Date.now()/1000);
const q = (env: Environment, sql: string, ...args: (string|number)[]) => env.DB.prepare(sql).bind(...args);
const random = () => Array.from(crypto.getRandomValues(new Uint8Array(32)), b=>b.toString(16).padStart(2,"0")).join("");
const json = (data: unknown, status=200) => new Response(JSON.stringify(data),{status,headers:{"content-type":"application/json","cache-control":"no-store","x-content-type-options":"nosniff"}});
type Machine = {computer:string;owner:string;certificate_fp:string;name:string;platform:string;credential_hash:string};

async function friends(env: Environment, owner: string, guest: string): Promise<boolean> {
  const [a,b]=owner<guest?[owner,guest]:[guest,owner];
  return !!await q(env,"SELECT 1 FROM friends f WHERE f.a=? AND f.b=? AND f.state='accepted' AND NOT EXISTS(SELECT 1 FROM blocks WHERE (owner=? AND blocked=?) OR (owner=? AND blocked=?))",a,b,owner,guest,guest,owner).first();
}

/** Only machine-scoped routes are handled before account-session authentication. */
export async function hostRoutes(request: Request, env: Environment, me: Owner|null, api: Helpers): Promise<Response|null> {
  const {pathname:path}=new URL(request.url), method=request.method;
  if(path.startsWith("/v1/host/")) {
    await api.limit(request,env,"host_api",300);
    const token=request.headers.get("authorization")?.match(/^Bearer (h1_[a-f0-9]{64})$/)?.[1];
    if(!token) api.fail(401,"host_auth_required");
    const machine=await q(env,"SELECT k.computer,c.owner,k.certificate_fp,c.name,c.platform,k.credential_hash FROM host_keys k JOIN computers c ON c.id=k.computer WHERE k.credential_hash=?",await api.hash(token)).first<Machine>();
    if(!machine) api.fail(401,"host_credential_revoked");
    const ownerResponse = await ownerRoutes(request, env, null, machine, api);
    if(ownerResponse) return ownerResponse;
    const {credential_hash: _credentialHash, ...publicMachine} = machine;
    if(path==="/v1/host/status" && method==="GET") return json(publicMachine);
    if(path==="/v1/host/heartbeat" && method==="POST") {
      await api.limit(request,env,"host_heartbeat",120);
      await q(env,"UPDATE host_keys SET last_seen=? WHERE computer=?",now(),machine.computer).run();
      return json({...publicMachine,sharing:false}); // Sharing is deliberately disabled until host enforcement exists.
    }
    if(path==="/v1/host/link" && method==="DELETE") {
      await unlink(env,machine.computer);
      return json({unlinked:true});
    }
    const lease=path.match(/^\/v1\/host\/requests\/([a-f0-9-]{36})$/);
    if(lease && method==="GET") {
      const row=await q(env,"SELECT id,guest,approved,state,expires FROM connection_requests WHERE id=? AND computer=?",lease[1],machine.computer).first<{id:string;guest:string;approved:string|null;state:string;expires:number}>();
      if(!row) api.fail(404,"request_not_found");
      const active=row.state==="claimed"&&row.expires>now()&&await friends(env,machine.owner,row.guest);
      return json({...row,approved:row.approved?JSON.parse(row.approved):[],active});
    }
    return json({error:"host_scope_denied"},403);
  }
  if(!me) return null;
  const ownerResponse = await ownerRoutes(request, env, me.id, null, api);
  if(ownerResponse) return ownerResponse;
  const base=new URL(env.PUBLIC_ORIGIN).origin;
  if(path==="/v1/host-enrollment/start"&&method==="POST") {
    await api.limit(request,env,"host_enroll",10);
    const challenge=random(), issued=now();
    await q(env,"INSERT INTO host_challenges(hash,owner,expires) VALUES(?,?,?)",await api.hash(challenge),me.id,issued+300).run();
    return json({challenge,owner:me.id,origin:base,issued_at:issued,expires_at:issued+300},201);
  }
  if(path==="/v1/host-enrollment/finish"&&method==="POST") {
    await api.limit(request,env,"host_finish",10);
    const b=await api.body(request);
    if(typeof b.certificate!=="string"||b.certificate.length>4096||typeof b.proof!=="string"||b.proof.length>4096) api.fail(400,"invalid_host_proof");
    const match=b.certificate.trim().match(/^-----BEGIN CERTIFICATE-----\s+([A-Za-z0-9+/=\s]+)\s+-----END CERTIFICATE-----$/);
    if(!match) api.fail(400,"invalid_host_certificate");
    const encoded=match[1].replace(/\s/g,"");
    const der=Buffer.from(encoded,"base64");
    if(der.toString("base64")!==encoded) api.fail(400,"invalid_host_certificate");
    const fp=Buffer.from(await crypto.subtle.digest("SHA-256",der)).toString("hex");
    let payload: JWTPayload, thumbprint: string;
    try {
      // A self-signed certificate is expected. Trust comes from the authenticated
      // owner's one-use challenge plus possession of this exact certificate key,
      // not a certificate-chain claim or a token-supplied JWK/URL.
      const key=await importX509(b.certificate,"ES256");
      const jwk=await exportJWK(key);
      if(jwk.kty!=="EC"||jwk.crv!=="P-256") api.fail(403,"invalid_host_proof");
      thumbprint=await calculateJwkThumbprint(jwk);
      ({payload}=await jwtVerify(b.proof,key,{algorithms:["ES256"],typ:"secpar-host-enrollment+jwt",issuer:"secpar-host",audience:base,maxTokenAge:90,clockTolerance:5,requiredClaims:["exp","iat","sub"]}));
    } catch { return api.fail(403,"invalid_host_proof"); }
    if(payload.sub!==me.id||payload.purpose!=="secpar-host-enrollment"||payload.tls_fingerprint!==fp||typeof payload.challenge!=="string"||!/^[a-f0-9]{64}$/.test(payload.challenge)||typeof payload.iat!=="number"||typeof payload.exp!=="number"||payload.exp>payload.iat+90) api.fail(403,"invalid_host_proof");
    const name=payload.name,platform=payload.platform;
    if(typeof name!=="string"||!name.trim()||name.length>80||typeof platform!=="string"||!["windows","linux","macos"].includes(platform)) api.fail(400,"invalid_host_metadata");
    const existing=await q(env,"SELECT k.computer,c.owner FROM host_keys k JOIN computers c ON c.id=k.computer WHERE k.thumbprint=?",thumbprint).first<{computer:string;owner:string}>();
    if(existing&&existing.owner!==me.id) api.fail(409,"host_already_linked");
    // Consume atomically before minting a machine credential. A replay cannot
    // rotate credentials, even if two finish requests arrive at the same time.
    const challenge=await q(env,"DELETE FROM host_challenges WHERE hash=? AND owner=? AND expires>? RETURNING owner",await api.hash(payload.challenge),me.id,now()).first();
    if(!challenge) api.fail(409,"host_challenge_expired_or_used");
    const computer=existing?.computer??crypto.randomUUID();
    const credential="h1_"+random();
    if(existing) {
      await env.DB.batch([
        q(env,"UPDATE computers SET name=?,platform=? WHERE id=? AND owner=?",name.trim(),platform,computer,me.id),
        q(env,"UPDATE host_keys SET certificate_fp=?,credential_hash=?,last_seen=0,sharing=0 WHERE computer=?",fp,await api.hash(credential),computer),
        q(env,"UPDATE connection_requests SET state='revoked' WHERE computer=?",computer),
        q(env,"DELETE FROM owner_pairings WHERE computer=?",computer),
      ]);
    }else{
      await env.DB.batch([
        q(env,"INSERT INTO computers(id,owner,name,platform,created) VALUES(?,?,?,?,?)",computer,me.id,name.trim(),platform,now()),
        q(env,"INSERT INTO host_keys(computer,thumbprint,certificate_fp,credential_hash) VALUES(?,?,?,?)",computer,thumbprint,fp,await api.hash(credential)),
      ]);
    }
    return json({computer,owner:me.id,certificate_fp:fp,credential,sharing:false},201);
  }
  const link=path.match(/^\/v1\/computers\/([a-f0-9-]{36})\/link$/);
  if(link&&method==="DELETE") {
    if(!await q(env,"SELECT 1 FROM computers WHERE id=? AND owner=?",link[1],me.id).first()) api.fail(404,"computer_not_found");
    await unlink(env,link[1]);
    return json({unlinked:true});
  }
  if(path==="/v1/friend-computers"&&method==="GET") {
    const rows=await q(env,"SELECT c.id,c.name,c.platform,c.owner FROM computers c JOIN host_keys k ON k.computer=c.id JOIN friends f ON (f.a=? AND f.b=c.owner) OR (f.b=? AND f.a=c.owner) WHERE f.state='accepted' AND k.sharing=1 AND k.last_seen>? AND NOT EXISTS(SELECT 1 FROM blocks b WHERE (b.owner=? AND b.blocked=c.owner) OR (b.owner=c.owner AND b.blocked=?))",me.id,me.id,now()-90,me.id,me.id).all();
    return json(rows.results);
  }
  return null;
}

async function unlink(env: Environment, computer: string) {
  await env.DB.batch([
    q(env,"UPDATE connection_requests SET state='revoked' WHERE computer=?",computer),
    q(env,"DELETE FROM owner_pairings WHERE computer=?",computer),
    q(env,"DELETE FROM host_keys WHERE computer=?",computer),
  ]);
}
