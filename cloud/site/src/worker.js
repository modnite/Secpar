export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.hostname === 'www.secpar.win') { url.hostname = 'secpar.win'; return Response.redirect(url.toString(), 301); }
    if (url.pathname === '/health') return Response.json({ service: 'secpar-site', status: 'ok' });
    // The blog is built by Hugo into /blog and served on its own host.
    if (url.hostname === 'secpar.win' && (url.pathname === '/blog' || url.pathname.startsWith('/blog/'))) {
      return Response.redirect('https://blog.secpar.win' + (url.pathname.slice(5) || '/') + url.search, 301);
    }
    let original;
    if (url.hostname === 'blog.secpar.win' && !url.pathname.startsWith('/assets/') && url.pathname !== '/style.css') {
      const inner = new URL(url);
      inner.pathname = '/blog' + url.pathname;
      original = await env.ASSETS.fetch(new Request(inner, request));
      const location = original.headers.get('location');
      if (location?.startsWith('/blog/')) {
        original = new Response(null, { status: original.status, headers: { location: location.slice(5) } });
      } else if (original.status === 404) {
        const missing = await env.ASSETS.fetch(new Request(new URL('/blog/404.html', url), request));
        original = new Response(missing.body, { status: 404, headers: missing.headers });
      }
    } else {
      original = await env.ASSETS.fetch(request);
    }
    const response = new Response(original.body, original);
    response.headers.set('X-Content-Type-Options', 'nosniff');
    response.headers.set('Referrer-Policy', 'strict-origin-when-cross-origin');
    response.headers.set('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'");
    response.headers.set('Cache-Control', url.pathname.startsWith('/assets/') ? 'public, max-age=3600' : 'public, max-age=0, must-revalidate');
    return response;
  }
};
