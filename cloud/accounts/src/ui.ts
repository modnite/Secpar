import { icon, rubik400, rubik500, wordmark } from "./assets";

// Shared look for every page this Worker shows. It follows secpar.win:
// Rubik, flat dark panels, mint accents. Fonts, wordmark and stylesheet are
// served from this Worker so the content security policy can stay at 'self'.

export const escapeHtml = (s: string) =>
  s
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");

export const APP_ACCOUNT_NOTE =
  "This account is only used inside the Secpar app to link your computers and friends. There is nothing to log in to on the website.";

const css = `@font-face{font-family:Rubik;src:url('/assets/rubik-400.woff') format('woff');font-weight:400;font-display:swap}
@font-face{font-family:Rubik;src:url('/assets/rubik-500.woff') format('woff');font-weight:500;font-display:swap}
:root{color-scheme:dark;--bg:#0c0e0f;--panel:#14191c;--line:#2a3437;--edge:#57615e;--text:#f6f8f7;--muted:#a8b4b2;--mint:#0affb6;--orange:#e58553}
*{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
body{margin:0;min-height:100vh;display:flex;flex-direction:column;background:var(--bg);color:var(--text);font-family:Rubik,system-ui,sans-serif;font-size:16px;line-height:1.55}
a{color:inherit;text-decoration:none}
a:hover{color:var(--mint)}
a:focus-visible,button:focus-visible,input:focus-visible{outline:2px solid var(--mint);outline-offset:3px}
.wrap{width:100%;max-width:1160px;margin:auto;padding-inline:32px}
.skip{position:absolute;top:-80px;left:20px;background:var(--mint);color:#071610;padding:10px;z-index:2}
.skip:focus{top:12px}
.nav{height:96px;display:flex;align-items:center;justify-content:space-between;gap:16px}
.brand{display:inline-flex;align-items:center;flex-shrink:0}
.brand img{display:block;width:151px;height:auto}
.nav nav{display:flex;gap:28px;font-size:14px}
main{flex:1;padding-block:24px 72px}
.card{max-width:460px;margin:0 auto;padding:36px;border:1px solid var(--line);border-radius:12px;background:var(--panel)}
h1{font-size:30px;line-height:1.2;font-weight:500;letter-spacing:-.8px;margin:0 0 14px}
.lede{color:var(--muted);font-size:15px;margin:0 0 28px}
p{margin:0 0 16px}
strong{font-weight:500}
form{margin:0}
.field{margin-bottom:20px}
label{display:block;font-size:14px;font-weight:500;margin-bottom:8px}
input{display:block;width:100%;min-height:48px;padding:11px 14px;border:1px solid var(--edge);border-radius:7px;background:var(--bg);color:var(--text);font:inherit;font-size:16px}
input:hover{border-color:var(--muted)}
input:focus{border-color:var(--mint)}
.hint{margin:8px 0 0;font-size:13px;color:var(--muted)}
.button{width:100%;min-height:50px;padding:12px 24px;border:1px solid var(--mint);border-radius:7px;background:var(--mint);color:#051b12;font:inherit;font-size:15px;font-weight:500;line-height:1.3;cursor:pointer;display:inline-flex;justify-content:center;align-items:center;text-align:center;transition:background .15s,border-color .15s}
.button:hover{background:#61ffca;border-color:#61ffca;color:#051b12}
.button.outline{background:transparent;border-color:var(--edge);color:var(--text)}
.button.outline:hover{background:#12241f;border-color:var(--mint);color:var(--text)}
.links{display:flex;flex-wrap:wrap;gap:8px 24px;margin:24px 0 0;font-size:14px;color:var(--muted)}
.links a{text-decoration:underline;text-underline-offset:3px}
.links a:hover{color:var(--mint)}
.code{margin:0 0 20px;padding:18px 16px;border:1px solid var(--line);border-radius:7px;background:var(--bg);font-size:30px;font-weight:500;letter-spacing:6px;text-align:center;font-variant-numeric:tabular-nums;overflow-wrap:anywhere}
.notice{margin:0 0 24px;padding:12px 14px;border:1px solid #74452f;border-left-width:4px;border-radius:7px;color:var(--text);font-size:15px}
.status{display:grid;place-items:center;width:34px;height:34px;margin-bottom:18px;border:1px solid #3c6354;border-radius:50%;color:var(--mint);font-size:15px}
.fine{margin:20px 0 0;font-size:13px;color:var(--muted)}
footer{border-top:1px solid var(--line);padding-block:26px;font-size:12px;color:var(--muted);display:flex;gap:22px;flex-wrap:wrap}
footer p{margin:0}
@media(max-width:620px){.wrap{padding-inline:22px}.nav{height:76px}.brand img{width:117px}.nav nav{gap:18px;font-size:13px}main{padding-block:12px 48px}.card{padding:24px 20px}h1{font-size:26px}.code{font-size:24px;letter-spacing:4px}}
@media(prefers-reduced-motion:reduce){.button{transition:none}}
`;

const bytes = (b64: string) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
const files: Record<string, { body: BodyInit; type: string }> = {
  "/assets/account.css": { body: css, type: "text/css; charset=utf-8" },
  "/assets/rubik-400.woff": { body: bytes(rubik400), type: "font/woff" },
  "/assets/rubik-500.woff": { body: bytes(rubik500), type: "font/woff" },
  "/assets/secpar-wordmark.svg": { body: wordmark, type: "image/svg+xml" },
  "/assets/secpar-icon.svg": { body: icon, type: "image/svg+xml" },
};

// Static files for the pages below. Returns null for any other path.
export function assetRoute(request: Request): Response | null {
  const path = new URL(request.url).pathname;
  const file = Object.hasOwn(files, path) ? files[path] : undefined;
  if (!file) return null;
  if (request.method !== "GET" && request.method !== "HEAD")
    return new Response("Method not allowed.", {
      status: 405,
      headers: { allow: "GET, HEAD" },
    });
  return new Response(request.method === "HEAD" ? null : file.body, {
    headers: {
      "content-type": file.type,
      "cache-control": "public, max-age=3600",
      "x-content-type-options": "nosniff",
    },
  });
}

export const CSP =
  "default-src 'none'; style-src 'self'; font-src 'self'; img-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

export function renderPage(
  title: string,
  content: string,
  status = 200,
  extraHeaders: Record<string, string> = {},
): Response {
  const markup = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="theme-color" content="#0c0e0f">
<meta name="robots" content="noindex">
<title>${escapeHtml(title)} · Secpar</title>
<link rel="icon" type="image/svg+xml" href="/assets/secpar-icon.svg">
<link rel="stylesheet" href="/assets/account.css">
</head>
<body>
<a class="skip" href="#main">Skip to content</a>
<header class="nav wrap"><a class="brand" href="https://secpar.win/" aria-label="Secpar website"><img src="/assets/secpar-wordmark.svg" width="151" height="36" alt="Secpar"></a><nav aria-label="Main navigation"><a href="https://secpar.win/help/">Help</a><a href="https://secpar.win/#downloads">Download</a></nav></header>
<main id="main" class="wrap"><section class="card">
${content}
</section></main>
<footer class="wrap"><p><a href="https://github.com/modnite/Secpar">Source</a> · GPL-3.0</p><p><a href="https://secpar.win/">secpar.win</a></p></footer>
</body>
</html>`;
  return new Response(markup, {
    status,
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
      "x-content-type-options": "nosniff",
      "content-security-policy": CSP,
      ...extraHeaders,
    },
  });
}

// Building blocks so every page uses the same markup.
export const heading = (text: string, lede?: string) =>
  `<h1>${escapeHtml(text)}</h1>${lede ? `<p class="lede">${escapeHtml(lede)}</p>` : ""}`;
export const field = (
  id: string,
  label: string,
  attrs: string,
  hint?: string,
) =>
  `<div class="field"><label for="${id}">${escapeHtml(label)}</label><input id="${id}" ${attrs}${hint ? ` aria-describedby="${id}-hint"` : ""}>${hint ? `<p class="hint" id="${id}-hint">${escapeHtml(hint)}</p>` : ""}</div>`;
