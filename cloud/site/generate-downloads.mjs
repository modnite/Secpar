import { writeFile } from 'node:fs/promises';
const headers = { 'User-Agent': 'Secpar-site', Accept: 'application/vnd.github+json' };
if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
const response = await fetch('https://api.github.com/repos/modnite/Secpar/releases?per_page=100', { headers, signal: AbortSignal.timeout(20000) });
if (!response.ok) throw Error(`Release lookup failed (${response.status})`);
const releases = await response.json(), result = {};
const names = { windows: 'Secpar-Windows-x64-setup.exe', linux: 'Secpar-Linux-x64.AppImage', android: 'Secpar.apk' };
for (const [platform, name] of Object.entries(names)) {
  const candidates = releases.filter(r => !r.draft && /^v\d+\.\d+\.\d+$/.test(r.tag_name)).sort((a, b) => {
    const x = a.tag_name.slice(1).split('.').map(Number), y = b.tag_name.slice(1).split('.').map(Number);
    return y[0] - x[0] || y[1] - x[1] || y[2] - x[2];
  });
  for (const release of candidates) {
    const asset = release.assets.find(a => a.name === name);
    if (!asset) continue;
    const expected = `https://github.com/modnite/Secpar/releases/download/${release.tag_name}/${name}`;
    if (asset.browser_download_url !== expected || !/^sha256:[a-f0-9]{64}$/.test(asset.digest) || !(asset.size > 0)) continue;
    result[platform] = { version: release.tag_name.slice(1), url: expected, size: asset.size, sha256: asset.digest.slice(7) };
    break;
  }
  if (!result[platform]) throw Error(`No verified ${platform} download found`);
}
await writeFile(new URL('./public/releases.json', import.meta.url), JSON.stringify(result, null, 2) + '\n');
console.log('Updated public platform downloads:', Object.keys(result).join(', '));
