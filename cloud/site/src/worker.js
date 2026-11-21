import { admin, blogCsp, commentsSection, postComment } from './comments.js';

const BLOG = 'blog.secpar.win';
const SITE_CSP = "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'";

async function blog(request, env, url) {
  if (url.pathname === '/comments' && request.method === 'POST') return postComment(request, env);
  if (url.pathname === '/admin' || url.pathname.startsWith('/admin/')) return admin(request, env, url);
  // The blog is built by Hugo into /blog. Conditional headers are dropped because
  // post pages carry live comments, so an asset ETag would not describe them.
  const inner = new URL(url);
  inner.pathname = '/blog' + url.pathname;
  let original = await env.ASSETS.fetch(new Request(inner, { method: request.method === 'HEAD' ? 'HEAD' : 'GET' }));
  const location = original.headers.get('location');
  if (location?.startsWith('/blog/')) {
    return new Response(null, { status: original.status, headers: { location: location.slice(5) } });
  }
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
    if (url.hostname === 'www.secpar.win') { url.hostname = 'secpar.win'; return Response.redirect(url.toString(), 301); }
    if (url.pathname === '/health') return Response.json({ service: 'secpar-site', status: 'ok' });
    if (url.hostname === 'secpar.win' && (url.pathname === '/blog' || url.pathname.startsWith('/blog/'))) {
      return Response.redirect(`https://${BLOG}` + (url.pathname.slice(5) || '/') + url.search, 301);
    }
    const onBlog = url.hostname === BLOG;
    const shared = url.pathname.startsWith('/assets/') || url.pathname === '/style.css';
    const original = onBlog && !shared ? await blog(request, env, url) : await env.ASSETS.fetch(request);
    const response = new Response(original.body, original);
    response.headers.set('X-Content-Type-Options', 'nosniff');
    response.headers.set('Referrer-Policy', 'strict-origin-when-cross-origin');
    response.headers.set('Content-Security-Policy', onBlog ? blogCsp : SITE_CSP);
    const html = (response.headers.get('content-type') || '').startsWith('text/html');
    response.headers.set('Cache-Control', shared ? 'public, max-age=3600'
      : onBlog && url.pathname.startsWith('/admin') ? 'no-store'
      : onBlog && html ? 'no-cache'
      : 'public, max-age=0, must-revalidate');
    return response;
  }
};
