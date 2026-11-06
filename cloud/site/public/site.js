fetch('/releases.json').then(r => { if (!r.ok) throw Error(); return r.json(); }).then(releases => {
  for (const platform of ['windows', 'linux', 'android']) {
    const release = releases[platform];
    if (!release || !/^\d+\.\d+\.\d+$/.test(release.version)) continue;
    const url = new URL(release.url);
    if (url.origin !== 'https://github.com' || !url.pathname.startsWith('/modnite/Secpar/releases/download/')) continue;
    const link = document.querySelector(`[data-download="${platform}"]`);
    const label = document.querySelector(`[data-version="${platform}"]`);
    if (link) link.href = release.url;
    if (label) label.textContent = release.version;
  }
}).catch(() => {});
