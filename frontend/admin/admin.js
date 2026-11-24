const TOKEN_KEY = 'congratis_admin_token';
const loginPanel = document.getElementById('loginPanel');
const dashboard = document.getElementById('dashboard');
const loginForm = document.getElementById('loginForm');
const loginError = document.getElementById('loginError');
const ornamentList = document.getElementById('ornamentList');
const summary = document.getElementById('summary');
const createdLinks = document.getElementById('createdLinks');
const createdLinkList = document.getElementById('createdLinkList');
let adminToken = sessionStorage.getItem(TOKEN_KEY) || '';

function showLogin(message = '') {
  adminToken = '';
  sessionStorage.removeItem(TOKEN_KEY);
  dashboard.hidden = true;
  loginPanel.hidden = false;
  loginError.textContent = message;
}
function showDashboard() {
  loginPanel.hidden = true;
  dashboard.hidden = false;
}
async function api(path, options = {}) {
  const headers = new Headers(options.headers || {});
  headers.set('Authorization', `Bearer ${adminToken}`);
  if (options.body) headers.set('Content-Type', 'application/json');
  const response = await fetch(path, { ...options, headers });
  const body = await response.json().catch(() => ({}));
  if (response.status === 401) {
    showLogin('Сессия завершена. Войдите снова.');
    throw new Error('Требуется повторная авторизация');
  }
  if (!response.ok) throw new Error(body.message || 'Ошибка запроса');
  return body;
}
function formatDate(timestamp) {
  return timestamp ? new Date(timestamp * 1000).toLocaleString('ru-RU') : '—';
}
function renderOrnaments(items) {
  ornamentList.replaceChildren();
  const used = items.filter(item => item.used).length;
  summary.textContent = `Всего: ${items.length}/20 · Свободно: ${20 - items.length} · Использовано: ${used}`;
  for (let number = 1; number <= 20; number += 1) {
    const item = items.find(row => row.ornament_number === number);
    const card = document.createElement('article');
    card.className = `ornament-row ${item ? (item.used ? 'is-used' : 'is-free') : 'is-empty'}`;
    const heading = document.createElement('div');
    heading.className = 'ornament-number';
    heading.textContent = `Шар №${number}`;
    const details = document.createElement('div');
    details.className = 'ornament-details';
    const state = document.createElement('strong');
    state.textContent = !item ? 'Не создано' : item.used ? 'Использовано' : 'Ожидает участника';
    const description = document.createElement('span');
    description.textContent = !item ? 'Для этого шара ещё нет приглашения' : item.used
      ? `${item.display_name || 'Участник'} · ${formatDate(item.used_at)}`
      : `Создано: ${formatDate(item.created_at)}`;
    details.append(state, description);
    card.append(heading, details);
    if (item) {
      const remove = document.createElement('button');
      remove.className = 'danger-button';
      remove.type = 'button';
      remove.textContent = 'Удалить';
      remove.addEventListener('click', () => deleteInvitation(number, item.used));
      card.append(remove);
    }
    ornamentList.append(card);
  }
}
async function refresh() {
  const items = await api('/api/admin/ornaments');
  renderOrnaments(items);
}
async function deleteInvitation(number, used) {
  const warning = used
    ? `Внимание: шар №${number} уже содержит поздравление. Удалить и приглашение, и поздравление без возможности восстановления?`
    : `Удалить неиспользованное приглашение для шара №${number}?`;
  if (!confirm(warning)) return;
  try {
    await api(`/api/admin/ornaments/${number}`, {
      method: 'DELETE',
      body: JSON.stringify({ confirm_used: used })
    });
    await refresh();
  } catch (error) {
    alert(error.message);
  }
}
loginForm.addEventListener('submit', async event => {
  event.preventDefault();
  loginError.textContent = '';
  const token = document.getElementById('adminToken').value;
  const button = loginForm.querySelector('button[type="submit"]');
  button.disabled = true;
  try {
    const response = await fetch('/api/admin/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ token })
    });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.message || 'Не удалось войти');
    adminToken = token;
    sessionStorage.setItem(TOKEN_KEY, token);
    document.getElementById('adminToken').value = '';
    showDashboard();
    await refresh();
  } catch (error) {
    loginError.textContent = error.message;
  } finally {
    button.disabled = false;
  }
});
document.getElementById('createForm').addEventListener('submit', async event => {
  event.preventDefault();
  const count = Number(document.getElementById('createCount').value);
  const status = document.getElementById('createStatus');
  const button = event.currentTarget.querySelector('button[type="submit"]');
  button.disabled = true;
  status.textContent = 'Создаём приглашения…';
  createdLinks.hidden = true;
  createdLinkList.replaceChildren();
  try {
    const result = await api('/api/admin/ornaments', {
      method: 'POST',
      body: JSON.stringify({ count })
    });
    result.created.forEach(item => {
      const li = document.createElement('li');
      const label = document.createElement('strong');
      label.textContent = `Шар №${item.ornament_number}: `;
      const link = document.createElement('a');
      link.href = item.url;
      link.textContent = item.url;
      link.target = '_blank';
      link.rel = 'noopener noreferrer';
      li.append(label, link);
      createdLinkList.append(li);
    });
    createdLinks.hidden = false;
    status.textContent = `Создано приглашений: ${result.created.length}`;
    await refresh();
  } catch (error) {
    status.textContent = error.message;
  } finally {
    button.disabled = false;
  }
});
document.getElementById('refreshButton').addEventListener('click', () => refresh().catch(error => alert(error.message)));
document.getElementById('logoutButton').addEventListener('click', () => showLogin());
if (adminToken) {
  showDashboard();
  refresh().catch(error => { if (adminToken) showLogin(error.message); });
}
