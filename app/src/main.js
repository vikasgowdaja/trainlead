import { invoke } from '@tauri-apps/api/core';
import { createIcons, Sun, Moon, Search, Upload, Inbox, Layers,
  FlaskConical, Save, MapPin, CalendarDays, Phone, MessageCircle, Check, Minus,
  CircleHelp, ArrowUpDown, RefreshCw, ShieldCheck, ArrowRight } from 'lucide';
import './style.css';

const icons = { Sun, Moon, Search, Upload, Inbox, Layers, FlaskConical,
  Save, MapPin, CalendarDays, Phone, MessageCircle, Check, Minus, CircleHelp,
  ArrowUpDown, RefreshCw, ShieldCheck, ArrowRight };
const state = { leads: [], reviews: [], tab: 'leads', busy: false };
const escape = value => String(value ?? '').replace(/[&<>"']/g, character => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
}[character]));
const icon = name => `<i data-lucide="${name}" aria-hidden="true"></i>`;
const native = Boolean(window.__TAURI_INTERNALS__);
const systemDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
document.documentElement.dataset.theme = localStorage.getItem('trainlead-theme') || (systemDark ? 'dark' : 'light');

document.querySelector('#app').innerHTML = `
  <header class="masthead">
    <div class="brand"><h1><img class="brand-wordmark" src="/brand/wordmark.png" alt="TrainLead" width="1200" height="302"></h1><span class="eyebrow">TRAINING REQUIREMENTS</span></div>
    <div class="header-actions"><span class="local-tag">${icon('shield-check')} Local</span>
      <button id="capture-open" class="icon-button" title="Capture a message" aria-label="Capture a message">${icon('save')}</button>
      <button id="theme" class="icon-button" title="Switch color theme" aria-label="Switch color theme">${icon('moon')}</button></div>
  </header>
  <div class="workspace">
    <aside class="capture-panel" aria-label="Capture requirements">
      <div class="section-title"><h2>Capture</h2><span class="offline-dot">Offline</span></div>
      <form id="capture-form">
        <label for="group">Source group</label><input id="group" maxlength="200" value="Pasted requirements" required>
        <label for="message">Message</label><textarea id="message" rows="7" placeholder="Paste a training requirement..." required></textarea>
        <div class="capture-actions"><button type="submit" class="primary">${icon('save')} <span id="submit-label">Save to inbox</span></button>
          <button type="button" id="upload" class="icon-button" title="Import chat export (.txt)" aria-label="Import chat export">${icon('upload')}</button></div>
        <input id="export-file" type="file" accept=".txt,text/plain" hidden>
      </form>
      <p class="state-note" id="extraction-note">Extraction off</p>
      <div class="divider"></div>
      <button id="demo" class="secondary">${icon('flask-conical')} Load demo</button>
      <p id="status" role="status" aria-live="polite">Opening workspace...</p>
      <div class="storage-note">${icon('shield-check')} On-device SQLite</div>
    </aside>
    <main>
      <nav class="tabs" aria-label="Workspace views">
        <button id="tab-leads" aria-pressed="true">${icon('layers')} Requirements <span id="lead-count">0</span></button>
        <button id="tab-inbox" aria-pressed="false">${icon('inbox')} Inbox <span id="inbox-count">0</span></button>
        <button id="refresh" class="icon-button" title="Refresh workspace" aria-label="Refresh workspace">${icon('refresh-cw')}</button>
      </nav>
      <div class="view-heading"><div><span class="eyebrow" id="view-label">YOUR OPPORTUNITIES</span><h2 id="view-title">Requirements</h2></div>
        <span id="result-count" class="result-count">0 leads</span></div>
      <div class="toolbar"><label class="search-field">${icon('search')}<input id="search" type="search" placeholder="Search tech, city, vendor..." aria-label="Search requirements"></label>
        <label id="repeat-control" class="check-label"><input id="repeated" type="checkbox"> Repeated only</label>
        <label id="sort-control" class="sort-label">${icon('arrow-up-down')}<select id="sort" aria-label="Sort requirements"><option value="date">Start date</option><option value="latest">Latest seen</option></select></label>
      </div>
      <section id="leads" class="lead-list" aria-label="Requirements"></section>
    </main>
  </div>
  <dialog id="privacy" aria-labelledby="privacy-title">
    <div class="privacy-symbol">${icon('shield-check')}</div><span class="eyebrow">BEFORE YOU START</span>
    <h2 id="privacy-title">Your messages stay here.</h2>
    <p>This preview stores group messages, vendor details and phone numbers on this device in an unencrypted SQLite database. No telemetry.</p>
    <p>Paste and import extraction uses Groq only when a GROQ_API_KEY is configured: messages matching training keywords are then sent to Groq's servers for processing. Without a key, messages stay on this device in the review inbox. Demo data is fictional.</p>
    <label class="consent"><input id="consent" type="checkbox"> I understand that messages may contain other people's personal data.</label>
    <button id="privacy-continue" class="primary" disabled>Open workspace ${icon('arrow-right')}</button>
  </dialog>
`;

function refreshIcons() { createIcons({ icons, attrs: { 'stroke-width': 1.7 } }); }
function announce(message, error = false) {
  const status = document.querySelector('#status');
  status.textContent = message;
  status.classList.toggle('error', error);
}
function setBusy(busy) {
  state.busy = busy;
  document.querySelectorAll('#capture-form button, #demo, #refresh').forEach(button => { button.disabled = busy; });
}
function formatDate(value) {
  if (!value) return 'Dates pending';
  const date = new Date(`${value}T12:00:00`);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleDateString('en-IN', { day: 'numeric', month: 'short', year: 'numeric' });
}
function provision(label, value) {
  const name = value === 'provided' ? 'check' : value === 'not_provided' ? 'minus' : 'circle-help';
  return `<span class="provision ${escape(value)}" title="${label}: ${escape(value.replaceAll('_', ' '))}">${icon(name)}${label}</span>`;
}
function leadCard(cluster) {
  const lead = cluster.primary;
  const data = lead.requirement;
  const date = data.start_date ? `${formatDate(data.start_date)}${data.end_date ? ` - ${formatDate(data.end_date)}` : ''}` : 'Dates pending';
  const demo = cluster.sources.every(source => source.source_group.startsWith('Demo:'));
  const vendor = data.vendor_name || data.vendor_company || 'Vendor not specified';
  const phone = data.vendor_phone;
  return `<article class="lead-card">
    <div class="card-top"><div class="tech-stack">${data.tech_stack.map(tech => `<span>${escape(tech)}</span>`).join('') || '<span>Tech pending</span>'}</div>
      <span class="repeat-badge ${cluster.seen > 1 ? 'repeated' : ''}">${cluster.seen > 1 ? `${cluster.seen} sightings` : 'New'}</span></div>
    <h3>${escape(data.college || data.summary)}</h3>
    <div class="lead-meta"><span>${icon('map-pin')}${escape([data.city, data.state].filter(Boolean).join(', ') || 'Location pending')}${data.mode ? ` <b class="mode">${escape(data.mode)}</b>` : ''}</span>
      <span>${icon('calendar-days')}${escape(date)}</span></div>
    ${data.audience ? `<p class="audience">${escape(data.audience)}${data.duration_days ? ` / ${data.duration_days} days` : ''}</p>` : ''}
    <div class="logistics">${provision('Travel', data.travel)}${provision('Food', data.food)}${provision('Stay', data.accommodation)}</div>
    <p class="rate-line">${escape(data.rate || data.logistics_note || 'Rate pending')}${data.payment_date ? ` <span>Payment ${escape(formatDate(data.payment_date))}</span>` : ''}</p>
    <div class="vendor-row"><div><strong>${escape(vendor)}</strong>${data.vendor_company ? `<small>${escape(data.vendor_company)}</small>` : ''}</div>
      ${demo ? '<span class="demo-label">DEMO</span>' : ''}
      ${phone ? `<a class="icon-button" href="tel:${encodeURIComponent(phone)}" title="Call vendor" aria-label="Call vendor">${icon('phone')}</a><a class="icon-button" href="https://wa.me/${encodeURIComponent(phone)}" title="Open in WhatsApp" aria-label="Open in WhatsApp">${icon('message-circle')}</a>` : ''}</div>
    <details><summary>Seen ${cluster.seen} times in ${cluster.groups} groups <span>${cluster.exact} exact / ${cluster.similar} similar</span></summary>
      <div class="source-list">${cluster.sources.map(source => `<div class="source"><strong>${escape(source.source_group)}</strong><span>${escape(source.sender || 'Unknown sender')} / ${escape(source.requirement.vendor_name || 'Vendor unknown')}</span><small>${escape(source.timestamp)} / ${escape(source.kind)}</small><pre>${escape(source.raw_text)}</pre></div>`).join('')}</div>
    </details>
  </article>`;
}

function render() {
  const search = document.querySelector('#search').value.trim().toLowerCase();
  const repeated = document.querySelector('#repeated').checked;
  const inbox = state.tab === 'inbox';
  document.querySelector('#lead-count').textContent = state.leads.length;
  document.querySelector('#inbox-count').textContent = state.reviews.length;
  document.querySelector('#tab-leads').setAttribute('aria-pressed', String(!inbox));
  document.querySelector('#tab-inbox').setAttribute('aria-pressed', String(inbox));
  document.querySelector('#view-title').textContent = inbox ? 'Review inbox' : 'Requirements';
  document.querySelector('#view-label').textContent = inbox ? 'AWAITING EXTRACTION' : 'YOUR OPPORTUNITIES';
  document.querySelector('#repeat-control').hidden = inbox;
  document.querySelector('#sort-control').hidden = inbox;
  let entries;
  if (inbox) {
    entries = state.reviews.filter(entry => JSON.stringify(entry.message).toLowerCase().includes(search));
    document.querySelector('#leads').innerHTML = entries.map(entry => `<article class="lead-card review-card"><div class="card-top"><strong>${escape(entry.message.source_group)}</strong><span class="review-badge">Needs review</span></div><small>${escape(entry.message.sender || 'Pasted message')} / ${escape(entry.message.timestamp)}</small><pre>${escape(entry.message.raw_text)}</pre><p class="state-note">${escape(entry.review_reason)}</p></article>`).join('');
  } else {
    entries = state.leads.filter(cluster => (!repeated || cluster.seen > 1) && JSON.stringify(cluster).toLowerCase().includes(search));
    if (document.querySelector('#sort').value === 'date') entries.sort((left, right) => (left.primary.requirement.start_date || '9999').localeCompare(right.primary.requirement.start_date || '9999'));
    document.querySelector('#leads').innerHTML = entries.map(leadCard).join('');
  }
  document.querySelector('#result-count').textContent = `${entries.length} ${inbox ? 'messages' : 'leads'}`;
  if (!entries.length) document.querySelector('#leads').innerHTML = `<div class="empty-state">${icon(inbox ? 'inbox' : 'layers')}<h3>${search || repeated ? 'No matches' : inbox ? 'Inbox is clear' : 'No requirements yet'}</h3><span>${search || repeated ? 'No results for these filters' : inbox ? '0 messages awaiting review' : '0 opportunities in your workspace'}</span>${!inbox && !search && !repeated ? `<button data-demo class="secondary">${icon('flask-conical')} Load demo</button>` : ''}</div>`;
  refreshIcons();
}

async function reload() {
  if (!native) {
    announce('Browser preview / native commands unavailable');
    render();
    return;
  }
  [state.leads, state.reviews, state.extraction] = await Promise.all([invoke('list_leads'), invoke('list_reviews'), invoke('extraction_status')]);
  const note = document.querySelector('#extraction-note');
  note.textContent = state.extraction.enabled ? `Extraction on / Groq ${state.extraction.model}` : 'Extraction off / add GROQ_API_KEY to .env';
  note.classList.toggle('enabled', state.extraction.enabled);
  document.querySelector('#submit-label').textContent = state.extraction.enabled ? 'Extract' : 'Save to inbox';
  render();
}
function summarize(report, noun) {
  if (!report.found && !report.needs_review) return `No training requirement found / ${report.skipped} ${noun} skipped`;
  const parts = [`Found ${report.found}: ${report.new} new, ${report.exact} exact repeat, ${report.similar} similar`];
  if (report.needs_review) parts.push(`${report.needs_review} to review inbox`);
  return parts.join(' / ');
}
async function action(work) {
  if (state.busy) return;
  if (!native) { announce('Open the installed TrainLead app for local storage', true); return; }
  setBusy(true);
  try {
    await work();
  } catch (error) {
    announce(String(error), true);
  } finally {
    setBusy(false);
  }
}

document.querySelector('#capture-form').addEventListener('submit', event => {
  event.preventDefault();
  action(async () => {
    const report = await invoke('capture_text', { text: document.querySelector('#message').value, group: document.querySelector('#group').value, export: false });
    document.querySelector('#message').value = '';
    state.tab = report.found ? 'leads' : 'inbox';
    await reload();
    document.querySelector('main').scrollIntoView();
    announce(summarize(report, 'message'));
  });
});
document.querySelector('#upload').addEventListener('click', () => document.querySelector('#export-file').click());
document.querySelector('#export-file').addEventListener('change', event => {
  const file = event.target.files[0];
  if (!file) return;
  action(async () => {
    if (file.size > 2 * 1024 * 1024) throw new Error('Export must be at most 2 MiB');
    const report = await invoke('capture_text', { text: await file.text(), group: file.name.replace(/\.txt$/i, ''), export: true });
    state.tab = report.found ? 'leads' : 'inbox';
    await reload();
    document.querySelector('main').scrollIntoView();
    announce(summarize(report, 'messages'));
  }).finally(() => { event.target.value = ''; });
});
document.querySelector('#demo').addEventListener('click', () => action(async () => {
  const report = await invoke('load_demo');
  state.tab = 'leads';
  await reload();
  document.querySelector('main').scrollIntoView();
  announce(`Demo: found ${report.found} / ${report.new} new / ${report.exact} exact / ${report.similar} similar`);
}));
document.querySelector('#leads').addEventListener('click', event => {
  if (event.target.closest('[data-demo]')) document.querySelector('#demo').click();
});
document.querySelector('#capture-open').addEventListener('click', () => {
  document.querySelector('.capture-panel').scrollIntoView();
  document.querySelector('#message').focus({ preventScroll: true });
});
document.querySelector('#refresh').addEventListener('click', () => action(async () => { await reload(); announce('Workspace refreshed'); }));
document.querySelector('#tab-leads').addEventListener('click', () => { state.tab = 'leads'; render(); });
document.querySelector('#tab-inbox').addEventListener('click', () => { state.tab = 'inbox'; render(); });
['search', 'repeated', 'sort'].forEach(name => document.querySelector(`#${name}`).addEventListener('input', render));
document.querySelector('#theme').addEventListener('click', () => {
  const theme = document.documentElement.dataset.theme === 'light' ? 'dark' : 'light';
  document.documentElement.dataset.theme = theme;
  localStorage.setItem('trainlead-theme', theme);
});
const dialog = document.querySelector('#privacy');
document.querySelector('#consent').addEventListener('change', event => { document.querySelector('#privacy-continue').disabled = !event.target.checked; });
dialog.addEventListener('cancel', event => event.preventDefault());
document.querySelector('#privacy-continue').addEventListener('click', () => {
  localStorage.setItem('trainlead-local-consent-v2', 'accepted');
  dialog.close();
});
if (localStorage.getItem('trainlead-local-consent-v2') !== 'accepted') dialog.showModal();
refreshIcons();
reload().then(() => { if (native) announce(state.extraction?.enabled ? 'Ready / Groq extraction on' : 'Ready / local capture only'); }).catch(error => announce(String(error), true));