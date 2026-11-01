import { invoke } from '@tauri-apps/api/core';
import { createIcons, Sun, Moon, Search, Upload, Inbox, Layers,
  FlaskConical, Save, MapPin, CalendarDays, Phone, MessageCircle, Check, Minus,
  CircleHelp, ArrowUpDown, RefreshCw, ShieldCheck, ArrowRight, KeyRound, RotateCw, Building2, GraduationCap, Plus,
  CircleCheck, Circle, FileText, Trash2, BadgeCheck, X, Send, TriangleAlert, UserPlus, Pencil, Download, Bell } from 'lucide';
import { createDirectory } from './directory.js';
import './style.css';

const icons = { Sun, Moon, Search, Upload, Inbox, Layers, FlaskConical,
  Save, MapPin, CalendarDays, Phone, MessageCircle, Check, Minus, CircleHelp,
  ArrowUpDown, RefreshCw, ShieldCheck, ArrowRight, KeyRound, RotateCw, Building2, GraduationCap, Plus,
  CircleCheck, Circle, FileText, Trash2, BadgeCheck, X, Send, TriangleAlert, UserPlus, Pencil, Download, Bell };
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
      <section id="whatsapp-panel" class="whatsapp-panel" aria-labelledby="whatsapp-title" hidden>
        <div class="section-title"><h2 id="whatsapp-title">WhatsApp</h2><span id="whatsapp-access" class="offline-dot">Off</span></div>
        <p id="whatsapp-note" class="whatsapp-note"></p>
        <button type="button" id="whatsapp-import" class="primary">${icon('download')} <span>Import from notifications</span></button>
        <div class="whatsapp-actions">
          <button type="button" id="whatsapp-access-button" class="secondary">${icon('bell')} Allow access</button>
          <button type="button" id="whatsapp-open" class="secondary">${icon('message-circle')} Open WhatsApp</button>
        </div>
      </section>
      <button type="button" id="ai-settings" class="ai-status">${icon('key-round')}<span id="extraction-note">AI extraction off</span></button>
      <div class="divider"></div>
      <button id="demo" class="secondary">${icon('flask-conical')} Load demo</button>
      <p id="status" role="status" aria-live="polite">Opening workspace...</p>
      <div class="storage-note">${icon('shield-check')} On-device SQLite</div>
    </aside>
    <main>
      <nav class="tabs" aria-label="Workspace views">
        <button id="tab-leads" aria-pressed="true">${icon('layers')} Requirements <span id="lead-count">0</span></button>
        <button id="tab-inbox" aria-pressed="false">${icon('inbox')} Inbox <span id="inbox-count">0</span></button>
        <button id="tab-vendors" aria-pressed="false">${icon('building-2')} Vendors <span id="vendor-count">0</span></button>
        <button id="tab-trainers" aria-pressed="false">${icon('graduation-cap')} Trainers <span id="trainer-count">0</span></button>
        <button id="refresh" class="icon-button" title="Refresh workspace" aria-label="Refresh workspace">${icon('refresh-cw')}</button>
      </nav>
      <div class="view-heading"><div><span class="eyebrow" id="view-label">YOUR OPPORTUNITIES</span><h2 id="view-title">Requirements</h2></div>
        <span id="result-count" class="result-count">0 leads</span></div>
      <div class="toolbar"><label class="search-field">${icon('search')}<input id="search" type="search" placeholder="Search tech, city, vendor..." aria-label="Search requirements"></label>
        <label id="repeat-control" class="check-label"><input id="repeated" type="checkbox"> Repeated only</label>
        <label id="sort-control" class="sort-label">${icon('arrow-up-down')}<select id="sort" aria-label="Sort requirements"><option value="date">Start date</option><option value="latest">Latest seen</option></select></label>
        <button id="retry-all" class="secondary retry-all" hidden>${icon('rotate-cw')} Retry all</button>
      </div>
      <section id="leads" class="lead-list" aria-label="Requirements"></section>
    </main>
  </div>
  <dialog id="privacy" aria-labelledby="privacy-title">
    <div class="privacy-symbol">${icon('shield-check')}</div><span class="eyebrow">BEFORE YOU START</span>
    <h2 id="privacy-title">Your messages stay here.</h2>
    <p>This preview stores group messages, vendor details and phone numbers on this device in an unencrypted SQLite database. No telemetry.</p>
    <p>Pasted and imported messages are read on this device by TrainLead's built-in rules. If you add a Groq API key in AI extraction settings, messages matching training keywords are instead sent to Groq's servers for better results. Demo data is fictional.</p>
    <p>On Android, if you allow notification access, TrainLead reads WhatsApp message notifications (groups and chats) and keeps them on this phone until imported. Other apps' notifications are ignored.</p>
    <label class="consent"><input id="consent" type="checkbox"> I understand that messages may contain other people's personal data.</label>
    <button id="privacy-continue" class="primary" disabled>Open workspace ${icon('arrow-right')}</button>
  </dialog>
  <dialog id="ai-dialog" aria-labelledby="ai-title">
    <div class="privacy-symbol">${icon('key-round')}</div><span class="eyebrow">AI EXTRACTION</span>
    <h2 id="ai-title">Connect Groq</h2>
    <p id="ai-state" class="ai-state">Not connected</p>
    <p>Optional. Without a key, TrainLead reads messages with built-in rules on this device. Groq usually understands unusual wording better.</p>
    <p>Create a key at console.groq.com/keys and paste it below. TrainLead checks it with Groq, then keeps it only in this app's private storage. It is never shown again.</p>
    <p>With a key, messages that look like training requirements are sent to Groq's servers. If Groq fails, TrainLead falls back to the built-in rules.</p>
    <form id="ai-form">
      <label for="groq-key">Groq API key</label>
      <input id="groq-key" type="password" autocomplete="off" spellcheck="false" autocapitalize="off" placeholder="gsk_..." required>
      <button type="submit" class="primary" id="ai-save">Save and check key</button>
    </form>
    <p id="ai-feedback" role="status" aria-live="polite"></p>
    <div class="dialog-actions"><button type="button" id="ai-remove" class="text-button" hidden>Remove saved key</button>
      <button type="button" id="ai-close" class="text-button">Close</button></div>
  </dialog>
`;

function refreshIcons() { createIcons({ icons, attrs: { 'stroke-width': 1.7 } }); }
const directory = createDirectory({ invoke, escape, icon, refreshIcons, announce });
function announce(message, error = false) {
  const status = document.querySelector('#status');
  status.textContent = message;
  status.classList.toggle('error', error);
}
function setBusy(busy) {
  state.busy = busy;
  document.querySelectorAll('#capture-form button, #demo, #refresh, #retry-all, [data-retry], #whatsapp-import').forEach(button => { button.disabled = busy; });
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
  const people = state.tab === 'vendors' || state.tab === 'trainers' ? state.tab.slice(0, -1) : null;
  const counts = directory.counts();
  document.querySelector('#lead-count').textContent = state.leads.length;
  document.querySelector('#inbox-count').textContent = state.reviews.length;
  document.querySelector('#vendor-count').textContent = counts.vendors;
  document.querySelector('#trainer-count').textContent = counts.trainers;
  for (const tab of ['leads', 'inbox', 'vendors', 'trainers']) {
    document.querySelector(`#tab-${tab}`).setAttribute('aria-pressed', String(state.tab === tab));
  }
  document.querySelector('#repeat-control').hidden = inbox || Boolean(people);
  document.querySelector('#sort-control').hidden = inbox || Boolean(people);
  document.querySelector('#retry-all').hidden = !inbox || !state.reviews.length;
  if (people) {
    document.querySelector('#view-title').textContent = people === 'vendor' ? 'Vendor onboarding' : 'Trainer onboarding';
    document.querySelector('#view-label').textContent = people === 'vendor' ? 'TRAINING COMPANIES' : 'TRAINER NETWORK';
    const shown = directory.render(people, document.querySelector('#leads'), search);
    document.querySelector('#result-count').textContent = `${shown} ${people}s`;
    refreshIcons();
    return;
  }
  document.querySelector('#view-title').textContent = inbox ? 'Review inbox' : 'Requirements';
  document.querySelector('#view-label').textContent = inbox ? 'AWAITING EXTRACTION' : 'YOUR OPPORTUNITIES';
  let entries;
  if (inbox) {
    entries = state.reviews.filter(entry => JSON.stringify(entry.message).toLowerCase().includes(search));
    document.querySelector('#leads').innerHTML = entries.map(entry => `<article class="lead-card review-card"><div class="card-top"><strong>${escape(entry.message.source_group)}</strong><span class="review-badge">Needs review</span></div><small>${escape(entry.message.sender || 'Pasted message')} / ${escape(entry.message.timestamp)}</small><pre>${escape(entry.message.raw_text)}</pre><p class="state-note">${escape(entry.review_reason)}</p><button class="text-button" data-retry="${Number(entry.id)}">${icon('rotate-cw')} Retry extraction</button></article>`).join('');
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
  [state.leads, state.reviews, state.extraction] = await Promise.all([invoke('list_leads'), invoke('list_reviews'), invoke('extraction_status'), directory.load()]);
  const note = document.querySelector('#extraction-note');
  note.textContent = state.extraction.enabled ? 'Extraction: Groq (rules as backup)' : 'Extraction: on-device rules / add Groq key';
  document.querySelector('#ai-settings').classList.toggle('enabled', state.extraction.enabled);
  document.querySelector('#submit-label').textContent = 'Extract';
  render();
}
function summarize(report, noun) {
  const engine = report.engine ? ` / via ${report.engine}` : '';
  if (!report.found && !report.needs_review) return `No training requirement found / ${report.skipped} ${noun} skipped${engine}`;
  const parts = [`Found ${report.found}: ${report.new} new, ${report.exact} exact repeat, ${report.similar} similar`];
  if (report.needs_review) parts.push(`${report.needs_review} to review inbox`);
  return parts.join(' / ') + engine;
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
  if (directory.handleClick(event)) return;
  if (event.target.closest('[data-demo]')) document.querySelector('#demo').click();
  const retry = event.target.closest('[data-retry]');
  if (retry) retryReview([Number(retry.dataset.retry)]);
});
document.querySelector('#retry-all').addEventListener('click', () => retryReview(null));
function retryReview(ids) {
  action(async () => {
    announce(state.extraction?.enabled ? 'Extracting with Groq...' : 'Extracting on this device...');
    const report = await invoke('retry_review', { ids });
    if (report.found) state.tab = 'leads';
    await reload();
    announce(summarize(report, ids ? 'message' : 'messages'), report.needs_review > 0 && !report.found);
  });
}
const aiDialog = document.querySelector('#ai-dialog');
function showAiState() {
  const status = state.extraction;
  const connected = Boolean(status?.enabled);
  document.querySelector('#ai-state').textContent = connected
    ? `Connected: Groq ${status.model} (key ${status.source === 'app' ? 'saved in this app' : 'from .env'})`
    : 'Not connected: messages wait in the review inbox';
  document.querySelector('#ai-state').classList.toggle('enabled', connected);
  document.querySelector('#ai-remove').hidden = status?.source !== 'app';
}
function openAiSettings() {
  if (!native) { announce('Open the installed TrainLead app to connect Groq', true); return; }
  document.querySelector('#ai-feedback').textContent = '';
  showAiState();
  aiDialog.showModal();
  document.querySelector('#groq-key').focus();
}
function aiFeedback(text, error = false) {
  const feedback = document.querySelector('#ai-feedback');
  feedback.textContent = text;
  feedback.classList.toggle('error', error);
}
document.querySelector('#ai-settings').addEventListener('click', openAiSettings);
document.querySelector('#ai-close').addEventListener('click', () => aiDialog.close());
document.querySelector('#ai-form').addEventListener('submit', async event => {
  event.preventDefault();
  const input = document.querySelector('#groq-key');
  const save = document.querySelector('#ai-save');
  save.disabled = true;
  aiFeedback('Checking key with Groq...');
  try {
    state.extraction = await invoke('save_groq_key', { key: input.value });
    input.value = '';
    await reload();
    showAiState();
    aiFeedback(state.reviews.length ? `Connected. Use "Retry all" in the inbox to re-extract ${state.reviews.length} waiting messages.` : 'Connected. New pastes will be extracted with Groq.');
  } catch (error) {
    aiFeedback(String(error), true);
  } finally {
    save.disabled = false;
  }
});
document.querySelector('#ai-remove').addEventListener('click', async () => {
  try {
    state.extraction = await invoke('clear_groq_key');
    await reload();
    showAiState();
    aiFeedback('Saved key removed.');
  } catch (error) {
    aiFeedback(String(error), true);
  }
});
document.querySelector('#capture-open').addEventListener('click', () => {
  document.querySelector('.capture-panel').scrollIntoView();
  document.querySelector('#message').focus({ preventScroll: true });
});
document.querySelector('#refresh').addEventListener('click', () => action(async () => { await reload(); announce('Workspace refreshed'); }));
document.querySelector('#tab-leads').addEventListener('click', () => { state.tab = 'leads'; render(); });
document.querySelector('#tab-inbox').addEventListener('click', () => { state.tab = 'inbox'; render(); });
document.querySelector('#tab-vendors').addEventListener('click', () => { state.tab = 'vendors'; render(); });
document.querySelector('#tab-trainers').addEventListener('click', () => { state.tab = 'trainers'; render(); });
directory.onClose(render);
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

// Present only in the Android app: WhatsApp notifications queued by the notification listener.
const android = window.TrainLeadAndroid;
function showWhatsAppState() {
  if (!android) return false;
  const granted = android.isAccessGranted();
  const waiting = android.pending();
  document.querySelector('#whatsapp-panel').hidden = false;
  const access = document.querySelector('#whatsapp-access');
  access.textContent = granted ? 'Listening' : 'Off';
  access.classList.toggle('on', granted);
  document.querySelector('#whatsapp-access-button').hidden = granted;
  document.querySelector('#whatsapp-note').textContent = granted
    ? `${waiting ? `${waiting} new messages waiting.` : 'No new messages waiting.'} New WhatsApp messages are collected from notifications and imported when you open TrainLead.`
    : 'Allow notification access so TrainLead can collect WhatsApp group messages as they arrive.';
  return granted;
}
async function importWhatsApp(quiet = false) {
  if (!showWhatsAppState()) {
    if (!quiet) announce('Allow notification access first', true);
    return;
  }
  const total = { found: 0, new: 0, exact: 0, similar: 0, needs_review: 0, skipped: 0 };
  let imported = 0;
  let engine = '';
  for (let round = 0; round < 50; round += 1) {
    const messages = JSON.parse(android.peek(200));
    if (!messages.length) break;
    const report = await invoke('import_notifications', { messages });
    android.ack(messages.length);
    imported += messages.length;
    engine = report.engine;
    for (const key of Object.keys(total)) total[key] += report[key] || 0;
  }
  showWhatsAppState();
  if (!imported) {
    if (!quiet) announce('No new WhatsApp messages. Messages arrive here as notifications.');
    return;
  }
  if (total.found) state.tab = 'leads';
  await reload();
  announce(`WhatsApp: ${imported} messages / ${summarize({ ...total, engine }, 'messages')}`);
}
if (android) {
  document.querySelector('#whatsapp-import').addEventListener('click', () => action(() => importWhatsApp()));
  document.querySelector('#whatsapp-access-button').addEventListener('click', () => android.openAccessSettings());
  document.querySelector('#whatsapp-open').addEventListener('click', () => {
    if (!android.openWhatsApp()) announce('WhatsApp is not installed on this phone', true);
  });
  // Returning from WhatsApp or settings imports whatever arrived meanwhile.
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') action(() => importWhatsApp(true));
  });
}
refreshIcons();
reload()
  .then(() => { if (native) announce(state.extraction?.enabled ? 'Ready / extraction with Groq' : 'Ready / extraction on this device'); })
  .then(() => { if (android) return action(() => importWhatsApp(true)); })
  .catch(error => announce(String(error), true));