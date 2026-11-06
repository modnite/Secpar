export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.hostname === 'www.secpar.win') { url.hostname = 'secpar.win'; return Response.redirect(url.toString(), 301); }
    if (url.pathname === '/health') return Response.json({ service: 'secpar-site', status: 'ok' });
    const original = await env.ASSETS.fetch(request);
    const response = new Response(original.body, original);
    response.headers.set('X-Content-Type-Options', 'nosniff');
    response.headers.set('Referrer-Policy', 'strict-origin-when-cross-origin');
    response.headers.set('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'");
    response.headers.set('Cache-Control', url.pathname.startsWith('/assets/') ? 'public, max-age=3600' : 'public, max-age=0, must-revalidate');
    return response;
  }
};
