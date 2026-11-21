import { admin, blogCsp, commentsSection, postComment } from './comments.js';
import { oauth } from './oauth.js';

const SITE = 'secpar.win';
const SITE_CSP = "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'";
// The blog editor (Decap CMS) builds its styles at run time and compiles its config schema,
// and talks to GitHub directly. This looser policy applies only to /blog/admin/.
const EDITOR_CSP = "default-src 'self'; script-src 'self' 'unsafe-eval'; style-src 'self' 'unsafe-inline'; font-src 'self' data:; img-src 'self' data: blob: https:; media-src 'self' blob: https:; connect-src 'self' https://api.github.com https://www.githubstatus.com https://raw.githubusercontent.com; frame-src 'self' blob:; worker-src 'self' blob:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'";
const EDITOR_FILES = { '/blog/admin/': '/editor/', '/blog/admin/config.yml': '/editor/config.yml', '/blog/admin/editor.js': '/editor/editor.js', '/blog/admin/decap-cms.js': '/editor/decap-cms.js' };

// The blog is built by Hugo into /blog. Post pages carry live comments, so asset
// ETags are dropped and conditional requests are not passed through.
async function blog(request, env, url) {
  const path = url.pathname;
  if (path === '/blog/comments' && request.method === 'POST') return postComment(request, env);
  if (path.startsWith('/blog/oauth/')) return oauth(request, env, url);
  if (path === '/blog/admin') return Response.redirect(`https://${SITE}/blog/admin/`, 301);
  if (EDITOR_FILES[path]) return env.ASSETS.fetch(new Request(new URL(EDITOR_FILES[path], url)));
  if (path.startsWith('/blog/admin/')) return admin(request, env, url);
  const original = await env.ASSETS.fetch(new Request(url, { method: request.method === 'HEAD' ? 'HEAD' : 'GET' }));
  if (original.status === 404) {
    const missing = await env.ASSETS.fetch(new Request(new URL('/blog/404', url)));
    return new Response(missing.body, { status: 404, headers: { 'content-type': 'text/html; charset=utf-8' } });
  }
  if (!(original.headers.get('content-type') || '').startsWith('text/html')) return original;
  const response = new Response(original.body, original);
  response.headers.delete('etag');
  response.headers.delete('last-modified');
  return new HTMLRewriter().on('section#comments[data-post]', {
    async element(el) { el.setInnerContent(await commentsSection(env, url, el.getAttribute('data-post')), { html: true }); },
  }).transform(response);
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    // www and the old blog subdomain both lead to secpar.win.
    if (url.hostname === `www.${SITE}`) { url.hostname = SITE; return Response.redirect(url.toString(), 301); }
    if (url.hostname === `blog.${SITE}`) return Response.redirect(`https://${SITE}/blog${url.pathname}${url.search}`, 301);
    if (url.pathname === '/health') return Response.json({ service: 'secpar-site', status: 'ok' });
    if (url.pathname.startsWith('/editor/')) return new Response('Not found', { status: 404 });

    const isBlog = url.pathname === '/blog' || url.pathname.startsWith('/blog/');
    const isEditor = url.pathname.startsWith('/blog/admin/') && !url.pathname.startsWith('/blog/admin/comments') && !url.pathname.startsWith('/blog/admin/api/');
    const original = isBlog ? await blog(request, env, url) : await env.ASSETS.fetch(request);
    if (original.headers.has('content-security-policy')) return original;
    const response = new Response(original.body, original);
    response.headers.set('X-Content-Type-Options', 'nosniff');
    response.headers.set('Referrer-Policy', 'strict-origin-when-cross-origin');
    response.headers.set('Content-Security-Policy', isEditor ? EDITOR_CSP : isBlog ? blogCsp : SITE_CSP);
    const html = (response.headers.get('content-type') || '').startsWith('text/html');
    response.headers.set('Cache-Control', url.pathname.startsWith('/assets/') ? 'public, max-age=3600'
      : url.pathname.startsWith('/blog/admin') ? 'no-store'
      : isBlog && html ? 'no-cache'
      : 'public, max-age=0, must-revalidate');
    return response;
  }
};
