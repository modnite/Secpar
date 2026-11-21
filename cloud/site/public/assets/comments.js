// Loads Cloudflare Turnstile only when someone starts writing a comment,
// so reading a post fetches no third-party script.
(() => {
  const form = document.querySelector('.comment-form');
  const box = form && form.querySelector('.turnstile');
  if (!box) return;
  let loaded = false;
  const load = () => {
    if (loaded) return;
    loaded = true;
    window.secparTurnstile = () => window.turnstile.render(box, { sitekey: box.dataset.sitekey, action: 'comment', theme: 'dark', size: 'flexible' });
    const script = document.createElement('script');
    script.src = 'https://challenges.cloudflare.com/turnstile/v0/api.js?onload=secparTurnstile&render=explicit';
    script.async = true;
    document.head.append(script);
  };
  form.addEventListener('focusin', load);
  form.addEventListener('submit', (event) => {
    const token = form.querySelector('[name="cf-turnstile-response"]');
    if (token && token.value) return;
    event.preventDefault();
    load();
    box.dataset.waiting = 'true';
  });
})();
