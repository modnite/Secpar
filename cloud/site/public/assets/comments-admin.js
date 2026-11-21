// The owner's comment queue. The moderator key stays in this tab's session storage.
(() => {
  const keyForm = document.getElementById('key');
  const queue = document.getElementById('queue');
  const list = document.getElementById('list');
  const status = document.getElementById('status');
  const tabs = [...queue.querySelectorAll('[data-state]')];
  let key = sessionStorage.getItem('secpar-moderator') || '';
  let state = 'pending';

  const call = async (method, body) => {
    const response = await fetch(`/admin/api/comments${method === 'GET' ? `?state=${state}` : ''}`, {
      method, headers: { authorization: `Bearer ${key}`, ...(body ? { 'content-type': 'application/json' } : {}) },
      body: body ? JSON.stringify(body) : undefined,
    });
    if (response.status === 401) { lock('That key was not accepted.'); throw Error('key'); }
    if (!response.ok) throw Error(String(response.status));
    return response.json();
  };

  const lock = (message) => {
    key = '';
    sessionStorage.removeItem('secpar-moderator');
    queue.hidden = true;
    keyForm.hidden = false;
    if (message) keyForm.querySelector('label').dataset.error = message;
  };

  const button = (label, action) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'button small outline';
    b.textContent = label;
    b.addEventListener('click', async () => {
      b.disabled = true;
      try { await action(); await load(); } catch { b.disabled = false; status.textContent = 'That did not work. Try again.'; }
    });
    return b;
  };

  const load = async () => {
    status.textContent = 'Loading';
    const { comments } = await call('GET');
    list.replaceChildren(...comments.map((c) => {
      const li = document.createElement('li');
      const meta = document.createElement('p');
      meta.className = 'meta';
      const name = document.createElement('strong');
      name.textContent = c.name;
      const when = document.createElement('time');
      when.textContent = new Date(c.created * 1000).toLocaleString();
      const post = document.createElement('a');
      post.href = `/${c.post}/`;
      post.textContent = c.post;
      meta.append(name, when, post);
      const text = document.createElement('p');
      text.className = 'text';
      text.textContent = c.body;
      const actions = document.createElement('div');
      actions.className = 'row';
      if (state === 'pending') actions.append(button('Approve', () => call('POST', { action: 'approve', id: c.id })));
      actions.append(button('Delete', () => call('POST', { action: 'delete', id: c.id })));
      if (c.from_sender > 1) {
        actions.append(button(`Delete all ${c.from_sender} from this sender`, () => call('POST', { action: 'delete-sender', sender: c.sender })));
      }
      li.append(meta, text, actions);
      return li;
    }));
    status.textContent = comments.length ? '' : state === 'pending' ? 'Nothing waiting.' : 'No approved comments.';
  };

  const open = async () => {
    keyForm.hidden = true;
    queue.hidden = false;
    try { await load(); } catch { /* lock() already explained a bad key */ }
  };

  keyForm.addEventListener('submit', (event) => {
    event.preventDefault();
    key = keyForm.elements.key.value.trim();
    keyForm.reset();
    sessionStorage.setItem('secpar-moderator', key);
    open();
  });
  tabs.forEach((tab) => tab.addEventListener('click', () => {
    state = tab.dataset.state;
    tabs.forEach((t) => t.setAttribute('aria-pressed', String(t === tab)));
    load().catch(() => {});
  }));
  document.getElementById('lock').addEventListener('click', () => lock());
  if (key) open();
})();
