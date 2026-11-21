// GitHub sign-in for the blog editor (Decap CMS). Decap opens /oauth/auth in a popup;
// GitHub returns to /oauth/callback, which hands the token back to the editor window.
// Only GitHub accounts that can push to the repository get a token back.
const ORIGIN = 'https://secpar.win';
const REPO = 'modnite/Secpar';
const STATE_COOKIE = '__Host-secpar-oauth';

const html = (body, nonce, status = 200, headers = {}) => new Response(
  `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="robots" content="noindex"><title>Secpar blog sign-in</title></head><body>${body}</body></html>`,
  { status, headers: {
    'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store',
    'content-security-policy': `default-src 'none'; script-src 'nonce-${nonce}'; base-uri 'none'; frame-ancestors 'none'`,
    ...headers,
  } });

// Tells the editor window the result, using Decap's handshake: announce, wait for the
// editor's reply from our own origin, then send the result only to that origin.
function finish(result, payload) {
  const nonce = crypto.randomUUID().replace(/-/g, '');
  const message = JSON.stringify(`authorization:github:${result}:${JSON.stringify(payload)}`).replace(/</g, '\\u003c');
  return html(`<p>${result === 'success' ? 'Signed in. You can close this window.' : 'Sign-in failed. You can close this window.'}</p>
<script nonce="${nonce}">(function(){var o=${JSON.stringify(ORIGIN)};function r(e){if(e.origin!==o)return;window.removeEventListener('message',r);window.opener.postMessage(${message},o);setTimeout(function(){window.close()},300)}window.addEventListener('message',r);window.opener&&window.opener.postMessage('authorizing:github',o)})();</script>`,
  nonce, 200, { 'set-cookie': `${STATE_COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0` });
}

export async function oauth(request, env, url) {
  if (!env.BLOG_GITHUB_CLIENT_ID || !env.BLOG_GITHUB_CLIENT_SECRET) {
    return html('<p>Editor sign-in is not set up yet.</p>', 'none', 503);
  }
  if (url.pathname === '/blog/oauth/auth') {
    const state = crypto.randomUUID();
    const target = new URL('https://github.com/login/oauth/authorize');
    target.searchParams.set('client_id', env.BLOG_GITHUB_CLIENT_ID);
    target.searchParams.set('redirect_uri', `${ORIGIN}/blog/oauth/callback`);
    target.searchParams.set('scope', 'public_repo');
    target.searchParams.set('state', state);
    target.searchParams.set('allow_signup', 'false');
    return new Response(null, { status: 302, headers: {
      location: target.toString(), 'cache-control': 'no-store',
      'set-cookie': `${STATE_COOKIE}=${state}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=600`,
    } });
  }
  if (url.pathname === '/blog/oauth/callback') {
    const cookie = (request.headers.get('cookie') || '').split(/;\s*/).find((c) => c.startsWith(`${STATE_COOKIE}=`));
    const expected = cookie ? cookie.slice(STATE_COOKIE.length + 1) : '';
    const state = url.searchParams.get('state') || '';
    const code = url.searchParams.get('code') || '';
    if (!expected || state !== expected || !code) return finish('error', { message: 'The sign-in expired. Try again.' });
    const exchange = await fetch('https://github.com/login/oauth/access_token', {
      method: 'POST', signal: AbortSignal.timeout(10000),
      headers: { accept: 'application/json', 'content-type': 'application/json', 'user-agent': 'secpar-blog' },
      body: JSON.stringify({ client_id: env.BLOG_GITHUB_CLIENT_ID, client_secret: env.BLOG_GITHUB_CLIENT_SECRET, code, redirect_uri: `${ORIGIN}/blog/oauth/callback` }),
    }).then((r) => r.json()).catch(() => ({}));
    const token = exchange.access_token;
    if (!token) return finish('error', { message: 'GitHub did not accept the sign-in. Try again.' });
    const repo = await fetch(`https://api.github.com/repos/${REPO}`, {
      signal: AbortSignal.timeout(10000),
      headers: { authorization: `Bearer ${token}`, accept: 'application/vnd.github+json', 'user-agent': 'secpar-blog' },
    }).then((r) => r.json()).catch(() => ({}));
    if (!repo?.permissions?.push) return finish('error', { message: 'This GitHub account cannot edit the Secpar blog.' });
    return finish('success', { token, provider: 'github' });
  }
  return new Response('Not found', { status: 404 });
}
