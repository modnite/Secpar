// Isolated workerd/D1 test. No Cloudflare credentials or live account are used.
// dispatchFetch routes the production-shaped audience URL into this local runtime.
import assert from 'node:assert/strict';
import {build} from 'esbuild';
import {Miniflare,convertV4MiniflareOptions} from 'miniflare';
import {fileURLToPath} from 'node:url';
import {readFileSync} from 'node:fs';
import {SignJWT,importPKCS8} from 'jose';
const base='https://accounts.secpar.win';
const bundled=await build({entryPoints:[fileURLToPath(new URL('../src/index.ts',import.meta.url))],bundle:true,format:'esm',platform:'neutral',conditions:['workerd','worker','browser'],mainFields:['module','main'],external:['node:*'],write:false,target:'es2022'});
const runtime=new Miniflare(convertV4MiniflareOptions({modules:true,script:bundled.outputFiles[0].text,compatibilityDate:'2026-11-04',compatibilityFlags:['nodejs_compat'],bindings:{PUBLIC_ORIGIN:base},d1Databases:{DB:'enrollment-test'}}));
try {
const db=await runtime.getD1Database('DB');
for(const migration of ['0001_accounts.sql','0002_passwords.sql','0003_hosts.sql']) {
 const sql=readFileSync(new URL('../migrations/'+migration,import.meta.url),'utf8');
 for(const statement of sql.split(';').map(s=>s.trim()).filter(Boolean))await db.prepare(statement).run();
}
const sessionHash=Buffer.from(await crypto.subtle.digest('SHA-256',new TextEncoder().encode('d'.repeat(64)))).toString('hex');
await db.prepare('INSERT INTO accounts(id,github_id,handle,name) VALUES(?,?,?,?)').bind('runtime-fixture','runtime-fixture','Fixture','Fixture').run();
await db.prepare('INSERT INTO sessions(hash,user_id,expires) VALUES(?,?,?)').bind(sessionHash,'runtime-fixture',Math.floor(Date.now()/1000)+60).run();
const account='d'.repeat(64);
const cert=readFileSync(new URL('./fixtures/host-cert.pem',import.meta.url),'utf8');
const key=await importPKCS8(readFileSync(new URL('./fixtures/host-key.pem',import.meta.url),'utf8'),'ES256');
async function call(path,token=account,method='GET',body){
 const r=await runtime.dispatchFetch(base+path,{method,headers:{authorization:`Bearer ${token}`,...(body?{'content-type':'application/json'}:{})},body:body?JSON.stringify(body):undefined});
 return {status:r.status,data:await r.json()};
}
const start=await call('/v1/host-enrollment/start',account,'POST');assert.equal(start.status,201);
const fp=Buffer.from(await crypto.subtle.digest('SHA-256',Buffer.from(cert.replace(/-----[^-]+-----/g,'').replace(/\s/g,''),'base64'))).toString('hex');
const proof=await new SignJWT({purpose:'secpar-host-enrollment',challenge:start.data.challenge,tls_fingerprint:fp,name:'Local runtime fixture',platform:'windows'})
 .setProtectedHeader({alg:'ES256',typ:'secpar-host-enrollment+jwt'}).setIssuer('secpar-host').setSubject(start.data.owner).setAudience(start.data.origin).setIssuedAt(start.data.issued_at).setExpirationTime(start.data.issued_at+90).sign(key);
const finish=await call('/v1/host-enrollment/finish',account,'POST',{certificate:cert,proof});assert.equal(finish.status,201,finish.data.error);
assert.equal(finish.data.certificate_fp,fp);
assert.equal((await call('/v1/host/status',finish.data.credential)).status,200);
assert.equal((await call('/v1/host/heartbeat',finish.data.credential,'POST')).status,200);
assert.equal((await call('/v1/me',finish.data.credential)).status,401);
assert.equal((await call('/v1/host-enrollment/finish',account,'POST',{certificate:cert,proof})).status,409);
assert.equal((await call('/v1/host/link',finish.data.credential,'DELETE')).status,200);
assert.equal((await call('/v1/host/status',finish.data.credential)).status,401);
console.log('Cloudflare local runtime: certificate proof, D1 enrollment, heartbeat, scope, replay and revocation PASS');

} finally { await runtime.dispose(); }
