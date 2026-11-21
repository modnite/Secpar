// Anonymous blog comments, held until the owner approves them.
// Bot and spam defence, cheapest check first: same-site origin, size limits, a honeypot
// field, a signed form time (too fast or too old is refused), link limits, a duplicate
// check, Cloudflare Turnstile verified on the server, and per-sender rate limits.
// Comments render as plain text, so links in them are never clickable.
const HOST = 'blog.secpar.win';
const ORIGIN = `https://${HOST}`;
const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
const encoder = new TextEncoder();

const escape = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const longDate = (seconds) => { const d = new Date(seconds * 1000); return `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]} ${d.getUTCFullYear()}`; };
const isoDate = (seconds) => new Date(seconds * 1000).toISOString().slice(0, 10);
const now = () => Math.floor(Date.now() / 1000);

async function hmac(key, text) {
  const k = await crypto.subtle.importKey('raw', encoder.encode(key), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  const sig = new Uint8Array(await crypto.subtle.sign('HMAC', k, encoder.encode(text)));
  return [...sig].map((b) => b.toString(16).padStart(2, '0')).join('');
}

function same(a, b) {
  const x = encoder.encode(a), y = encoder.encode(b);
  return x.length === y.length && crypto.subtle.timingSafeEqual(x, y);
}

export const commentsOpen = (env) => Boolean(env.DB && env.TURNSTILE_SITEKEY && env.TURNSTILE_SECRET && env.BLOG_ADMIN_KEY);

export const blogCsp = "default-src 'self'; script-src 'self' https://challenges.cloudflare.com; frame-src https://challenges.cloudflare.com; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'";

const NOTICES = {
  sent: 'Thanks. Your comment will appear once it is approved.',
  check: 'The bot check did not pass. Please try again.',
  expired: 'The form expired. Please post again.',
  fast: 'That was too quick. Please post again.',
  empty: 'Write a comment first.',
  long: 'Comments can be up to 2000 characters.',
  links: 'Comments can include at most two links.',
  name: 'Names can be up to 40 characters, without links.',
  busy: 'You have posted a lot recently. Please try again later.',
  paused: 'Comments are paused for now. Please try again later.',
};

function bodyHtml(text) {
  return text.split(/\n{2,}/).map((p) => `<p>${escape(p).replace(/\n/g, '<br>')}</p>`).join('');
}

/** Fills the post's comments section: approved comments, then the form. */
export async function commentsSection(env, url, post) {
  if (!commentsOpen(env)) return '<h2>Comments</h2><p class="empty">Comments are not open yet.</p>';
  const rows = (await env.DB.prepare('SELECT id, name, body, created FROM comments WHERE post=? AND approved=1 ORDER BY created LIMIT 500').bind(post).all()).results;
  const list = rows.length
    ? `<ol class="comments">${rows.map((c) => `<li id="c${c.id}"><p class="meta"><strong>${escape(c.name)}</strong><time datetime="${isoDate(c.created)}">${longDate(c.created)}</time></p>${bodyHtml(c.body)}</li>`).join('')}</ol>`
    : '<p class="empty">No comments yet.</p>';
  const notice = NOTICES[url.searchParams.get('comment')];
  const t = now(), stamp = `${t}.${await hmac(env.BLOG_ADMIN_KEY, `form:${post}:${t}`)}`;
  return `<h2>Comments</h2>${list}
<form class="comment-form" method="post" action="/comments">${notice ? `<p class="notice${url.searchParams.get('comment') === 'sent' ? '' : ' error'}" role="status">${escape(notice)}</p>` : ''}
<input type="hidden" name="post" value="${escape(post)}"><input type="hidden" name="t" value="${stamp}">
<label>Name (optional)<input name="name" maxlength="40" autocomplete="nickname" placeholder="Anonymous"></label>
<label>Comment<textarea name="body" required minlength="2" maxlength="2000" rows="5"></textarea></label>
<div class="trap" aria-hidden="true"><label>Leave this empty<input name="website" tabindex="-1" autocomplete="off"></label></div>
<div class="turnstile" data-sitekey="${escape(env.TURNSTILE_SITEKEY)}"></div>
<div class="row"><button class="button small outline" type="submit">Post comment</button><p>Comments appear after approval. No account needed.</p></div>
</form><script src="/assets/comments.js" defer></script>`;
}

const back = (post, code) => Response.redirect(`${ORIGIN}/${post}/?comment=${code}#comments`, 303);

export async function postComment(request, env) {
  if (!commentsOpen(env)) return new Response('Comments are not open.', { status: 404 });
  if (request.headers.get('origin') !== ORIGIN) return new Response('Forbidden', { status: 403 });
  if (!(request.headers.get('content-type') || '').startsWith('application/x-www-form-urlencoded')) return new Response('Unsupported', { status: 415 });
  if (Number(request.headers.get('content-length') || 0) > 16384) return new Response('Too large', { status: 413 });
  const raw = await request.text();
  if (raw.length > 16384) return new Response('Too large', { status: 413 });
  const form = new URLSearchParams(raw);
  const post = form.get('post') || '';
  if (!/^[a-z0-9][a-z0-9._-]{0,120}$/.test(post)) return new Response('Unknown post', { status: 400 });
  const page = await env.ASSETS.fetch(new Request(`${ORIGIN}/blog/${post}/`));
  if (page.status !== 200) return new Response('Unknown post', { status: 400 });

  // Bots fill every field; people never see this one. Pretend it worked.
  if (form.get('website')) return back(post, 'sent');
  const [ts, sig] = (form.get('t') || '').split('.');
  const age = now() - Number(ts);
  if (!sig || !same(sig, await hmac(env.BLOG_ADMIN_KEY, `form:${post}:${ts}`))) return back(post, 'expired');
  if (age > 86400) return back(post, 'expired');
  if (age < 4) return back(post, 'fast');

  const body = (form.get('body') || '').replace(/\r\n?/g, '\n').replace(/[\u0000-\u0008\u000b-\u001f\u007f‪-‮⁦-⁩]/g, '').replace(/\n{3,}/g, '\n\n').trim();
  const name = (form.get('name') || '').replace(/\s+/g, ' ').replace(/[\u0000-\u001f\u007f‪-‮⁦-⁩]/g, '').trim() || 'Anonymous';
  if (body.length < 2) return back(post, 'empty');
  if (body.length > 2000) return back(post, 'long');
  if (name.length > 40 || /https?:|www\.|\.(com|net|ru|xyz|top)\b/i.test(name)) return back(post, 'name');
  if ((body.match(/https?:\/\/|www\./gi) || []).length > 2 || /<a\s|\[url[=\]]/i.test(body)) return back(post, 'links');

  const ip = request.headers.get('cf-connecting-ip') || '';
  const sender = await hmac(env.BLOG_ADMIN_KEY, `sender:${ip}`);
  const t = now();
  const recent = await env.DB.prepare('SELECT SUM(created > ?) AS recent, COUNT(*) AS day FROM comments WHERE sender=? AND created > ?')
    .bind(t - 600, sender, t - 86400).first();
  if ((recent?.recent || 0) >= 3 || (recent?.day || 0) >= 12) return back(post, 'busy');
  const queue = await env.DB.prepare('SELECT COUNT(*) AS n FROM comments WHERE approved=0').first();
  if ((queue?.n || 0) >= 300) return back(post, 'paused');
  const duplicate = await env.DB.prepare('SELECT 1 FROM comments WHERE post=? AND body=? AND created > ? LIMIT 1').bind(post, body, t - 86400).first();
  if (duplicate) return back(post, 'sent');

  const check = new FormData();
  check.append('secret', env.TURNSTILE_SECRET);
  check.append('response', form.get('cf-turnstile-response') || '');
  if (ip) check.append('remoteip', ip);
  check.append('idempotency_key', crypto.randomUUID());
  let verdict;
  try {
    verdict = await (await fetch('https://challenges.cloudflare.com/turnstile/v0/siteverify', { method: 'POST', body: check, signal: AbortSignal.timeout(10000) })).json();
  } catch { return back(post, 'check'); }
  // TURNSTILE_TESTING is only for local runs with Cloudflare's test keys, which report example.com and no action.
  const testing = env.TURNSTILE_TESTING === 'local' && verdict?.metadata?.result_with_testing_key;
  if (!verdict?.success || (!testing && (verdict.hostname !== HOST || verdict.action !== 'comment'))) return back(post, 'check');

  await env.DB.prepare('INSERT INTO comments (post, name, body, created, approved, sender) VALUES (?, ?, ?, ?, 0, ?)').bind(post, name, body, t, sender).run();
  return back(post, 'sent');
}

const json = (data, status = 200) => new Response(JSON.stringify(data), { status, headers: { 'content-type': 'application/json', 'cache-control': 'no-store' } });

const ADMIN_PAGE = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex"><title>Comments | Secpar blog</title><link rel="icon" href="/assets/secpar-icon.svg" type="image/svg+xml"><link rel="stylesheet" href="/style.css"><link rel="stylesheet" href="/assets/blog.css"></head>
<body><header class="nav wrap"><a class="brand" href="/" aria-label="Secpar blog"><img src="/assets/secpar-wordmark.svg" width="151" height="36" alt="Secpar"></a><nav aria-label="Main navigation"><a href="/">Blog</a></nav></header>
<main class="wrap blog moderate"><h1>Comments</h1>
<form id="key" class="comment-form"><label>Moderator key<input type="password" name="key" autocomplete="current-password" required></label><div class="row"><button class="button small outline" type="submit">Open</button></div></form>
<div id="queue" hidden><nav class="tabs"><button type="button" data-state="pending" aria-pressed="true">Waiting</button><button type="button" data-state="approved" aria-pressed="false">Approved</button><button type="button" id="lock">Lock</button></nav><p id="status" class="empty" role="status"></p><ol class="comments" id="list"></ol></div>
</main><script src="/assets/comments-admin.js" defer></script></body></html>`;

/** The owner's moderation page and its API, behind the moderator key. */
export async function admin(request, env, url) {
  if (url.pathname === '/admin' || url.pathname === '/admin/') {
    return new Response(ADMIN_PAGE, { headers: { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store', 'x-robots-tag': 'noindex' } });
  }
  if (!commentsOpen(env)) return json({ error: 'closed' }, 404);
  const key = (request.headers.get('authorization') || '').replace(/^Bearer /, '');
  if (!key || !same(await hmac('moderator', key), await hmac('moderator', env.BLOG_ADMIN_KEY))) return json({ error: 'key' }, 401);
  if (url.pathname === '/admin/api/comments' && request.method === 'GET') {
    const approved = url.searchParams.get('state') === 'approved' ? 1 : 0;
    const rows = (await env.DB.prepare(`SELECT c.id, c.post, c.name, c.body, c.created, c.sender,
        (SELECT COUNT(*) FROM comments s WHERE s.sender=c.sender) AS from_sender
      FROM comments c WHERE approved=? ORDER BY created ${approved ? 'DESC' : 'ASC'} LIMIT 200`).bind(approved).all()).results;
    return json({ comments: rows });
  }
  if (url.pathname === '/admin/api/comments' && request.method === 'POST') {
    if (request.headers.get('origin') !== ORIGIN) return json({ error: 'origin' }, 403);
    const { action, id, sender } = await request.json().catch(() => ({}));
    if (action === 'approve') await env.DB.prepare('UPDATE comments SET approved=1 WHERE id=?').bind(Number(id)).run();
    else if (action === 'delete') await env.DB.prepare('DELETE FROM comments WHERE id=?').bind(Number(id)).run();
    else if (action === 'delete-sender' && /^[a-f0-9]{64}$/.test(sender || '')) await env.DB.prepare('DELETE FROM comments WHERE sender=?').bind(sender).run();
    else return json({ error: 'action' }, 400);
    return json({ ok: true });
  }
  return json({ error: 'not_found' }, 404);
}
