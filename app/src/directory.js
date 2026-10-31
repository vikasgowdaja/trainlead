// Vendor and trainer onboarding screens. Data lives in the local directory database.
const VENDOR_DOCS = ['Registration certificate', 'GST certificate', 'PAN card', 'Other'];
const TRAINER_DOCS = ['Resume', 'ID proof', 'Certificate', 'Other'];
const MODES = ['offline', 'online', 'hybrid'];
const TRAVEL = { local_only: 'Local only', within_state: 'Within state', anywhere: 'Anywhere in India' };
const MAX_UPLOAD = 2 * 1024 * 1024;

export function createDirectory({ invoke, escape, icon, refreshIcons, announce }) {
  const data = { vendors: [], trainers: [], suggestions: [] };
  const editor = { kind: 'vendor', record: null, draft: null };
  let onClose = null;
  const dialog = document.createElement('dialog');
  dialog.id = 'profile-dialog';
  dialog.className = 'profile-dialog';
  dialog.setAttribute('aria-labelledby', 'profile-title');
  document.body.append(dialog);

  const label = (kind, status) => ({ draft: 'Draft', pending: 'Pending review', rejected: 'Rejected',
    approved: kind === 'vendor' ? 'Active' : 'Verified' }[status] || status);
  const badge = (kind, status) => `<span class="status-badge ${escape(status)}">${escape(label(kind, status))}</span>`;
  const value = item => escape(item ?? '');
  const list = items => (items || []).join(', ');
  const today = new Date().toISOString().slice(0, 10);
  const progress = checklist => {
    const done = checklist.filter(step => step.done).length;
    return `<div class="progress" title="${done} of ${checklist.length} steps"><span style="width:${Math.round(done / checklist.length * 100)}%"></span></div><small>${done}/${checklist.length} steps</small>`;
  };
  const rupees = amount => `₹${Number(amount).toLocaleString('en-IN')}`;

  async function load() { Object.assign(data, await invoke('load_directory')); }

  function vendorCard(record) {
    const profile = record.profile;
    return `<article class="lead-card profile-card">
      <div class="card-top"><div class="tech-stack">${profile.domains.slice(0, 6).map(domain => `<span>${escape(domain)}</span>`).join('') || '<span>Domains pending</span>'}</div>${badge('vendor', record.status)}</div>
      <h3>${escape(profile.display_name || profile.legal_name)}</h3>
      <div class="lead-meta"><span>${icon('map-pin')}${escape([profile.city, profile.state].filter(Boolean).join(', ') || 'Location pending')}</span>
        <span>${icon('phone')}${escape(profile.contact_name || 'Contact pending')}${profile.contact_phone ? ` / +${escape(profile.contact_phone)}` : ''}</span>
        <span>${icon('layers')}${record.postings} WHATSAPP POSTINGS / ${record.documents.length} documents</span></div>
      ${record.review_note ? `<p class="review-note">${icon('triangle-alert')}${escape(record.review_note)}</p>` : ''}
      <div class="profile-footer">${progress(record.checklist)}<button class="secondary" data-open="vendor:${record.id}">${icon('pencil')} Open</button></div>
    </article>`;
  }

  function trainerCard(record) {
    const profile = record.profile;
    const expired = record.documents.filter(document => document.expires_on && document.expires_on < today).length;
    const rate = profile.rate_min ? `${rupees(profile.rate_min)}${profile.rate_max ? ` - ${rupees(profile.rate_max)}` : ''} / day` : 'Rate pending';
    return `<article class="lead-card profile-card">
      <div class="card-top"><div class="tech-stack">${profile.skills.slice(0, 6).map(skill => `<span class="${skill.verified ? 'verified' : ''}">${skill.verified ? icon('badge-check') : ''}${escape(skill.name)} <b>${skill.proficiency}/5</b></span>`).join('') || '<span>Skills pending</span>'}</div>${badge('trainer', record.status)}</div>
      <h3>${escape(profile.name)}</h3>
      <div class="lead-meta"><span>${icon('map-pin')}${escape([profile.city, profile.state].filter(Boolean).join(', ') || 'City pending')} / ${escape(TRAVEL[profile.travel] || '')}</span>
        <span>${icon('graduation-cap')}${profile.experience_years ?? '?'} years / ${escape(profile.modes.join(', ') || 'modes pending')}</span>
        <span>${icon('file-text')}${escape(rate)} / ${record.documents.length} documents</span></div>
      ${expired ? `<p class="review-note">${icon('triangle-alert')}${expired} certificate${expired > 1 ? 's' : ''} expired</p>` : ''}
      ${record.review_note ? `<p class="review-note">${icon('triangle-alert')}${escape(record.review_note)}</p>` : ''}
      <div class="profile-footer">${progress(record.checklist)}<button class="secondary" data-open="trainer:${record.id}">${icon('pencil')} Open</button></div>
    </article>`;
  }

  function render(kind, container, search) {
    const records = kind === 'vendor' ? data.vendors : data.trainers;
    const shown = records.filter(record => JSON.stringify(record.profile).toLowerCase().includes(search));
    const pending = records.filter(record => record.status === 'pending').length;
    const add = `<div class="directory-actions"><button class="primary" data-new="${kind}">${icon(kind === 'vendor' ? 'building-2' : 'user-plus')} Add ${kind}</button>
      ${pending ? `<span class="pending-note">${pending} awaiting your review</span>` : ''}</div>`;
    const cards = shown.map(kind === 'vendor' ? vendorCard : trainerCard).join('');
    const empty = `<div class="empty-state">${icon(kind === 'vendor' ? 'building-2' : 'graduation-cap')}<h3>${search ? 'No matches' : `No ${kind}s yet`}</h3>
      <span>${search ? 'No results for this search' : kind === 'vendor' ? 'Add a training company to start its onboarding' : 'Add a trainer with skills, rates and documents'}</span></div>`;
    const suggestions = kind === 'vendor' && data.suggestions.length ? `<section class="suggestions"><h3>Seen in your WhatsApp requirements</h3>
      ${data.suggestions.slice(0, 8).map((suggestion, index) => `<div class="suggestion"><div><strong>${escape(suggestion.company || suggestion.name || 'Unknown vendor')}</strong>
        <small>+${escape(suggestion.phone)} / ${suggestion.postings} posting${suggestion.postings > 1 ? 's' : ''}</small></div>
        <button class="text-button" data-suggest="${index}">${icon('plus')} Add as vendor</button></div>`).join('')}</section>` : '';
    container.innerHTML = add + (shown.length ? `<div class="lead-list">${cards}</div>` : empty) + suggestions;
    return shown.length;
  }

  function field(name, text, current, attributes = '') {
    return `<label class="field"><span>${text}</span><input name="${name}" value="${value(current)}" ${attributes}></label>`;
  }

  function checklistHtml(checklist) {
    return `<ul class="checklist">${checklist.map(step => `<li class="${step.done ? 'done' : ''}">${icon(step.done ? 'circle-check' : 'circle')}${escape(step.label)}${step.required ? '' : ' <small>(optional)</small>'}</li>`).join('')}</ul>`;
  }

  function vendorForm(profile) {
    return `<fieldset><legend>Company</legend>
        ${field('legal_name', 'Legal name *', profile.legal_name, 'required maxlength="160"')}
        ${field('display_name', 'Display name', profile.display_name, 'maxlength="160"')}
        <div class="field-row">${field('gstin', 'GSTIN', profile.gstin, 'maxlength="15" autocapitalize="characters" placeholder="29ABCDE1234F1Z5"')}
        ${field('pan', 'PAN', profile.pan, 'maxlength="10" autocapitalize="characters" placeholder="ABCDE1234F"')}</div>
        ${field('website', 'Website', profile.website, 'type="url" placeholder="https://"')}</fieldset>
      <fieldset><legend>Address</legend>
        ${field('address', 'Street address', profile.address, 'maxlength="300"')}
        <div class="field-row">${field('city', 'City *', profile.city, 'maxlength="80"')}${field('state', 'State *', profile.state, 'maxlength="80"')}</div>
        ${field('pincode', 'PIN code', profile.pincode, 'inputmode="numeric" maxlength="6"')}</fieldset>
      <fieldset><legend>Contact person *</legend>
        ${field('contact_name', 'Name', profile.contact_name, 'maxlength="120"')}
        <div class="field-row">${field('contact_phone', 'Phone', profile.contact_phone, 'type="tel"')}${field('contact_email', 'Email', profile.contact_email, 'type="email"')}</div>
        ${field('whatsapp_numbers', 'WhatsApp numbers they post from (comma separated)', list(profile.whatsapp_numbers), 'inputmode="tel"')}</fieldset>
      <fieldset><legend>Domains served *</legend>
        ${field('domains', 'Skills or areas, comma separated', list(profile.domains), 'placeholder="Java, Angular, Soft Skills"')}
        <label class="field"><span>Notes</span><textarea name="notes" rows="3" maxlength="2000">${value(profile.notes)}</textarea></label></fieldset>`;
  }

  function skillRow(skill, index, saved) {
    const options = [1, 2, 3, 4, 5].map(level => `<option value="${level}" ${Number(skill.proficiency) === level ? 'selected' : ''}>${level}</option>`).join('');
    const verify = saved && skill.name ? `<button type="button" class="text-button" data-verify="${escape(skill.name)}" data-to="${!skill.verified}">${icon('badge-check')} ${skill.verified ? 'Unverify' : 'Verify'}</button>` : '';
    return `<div class="repeat-row skill-row" data-index="${index}">
      <label class="field"><span>Skill</span><input name="skill_name" value="${value(skill.name)}" placeholder="AngularJS"></label>
      <label class="field small"><span>Level 1-5</span><select name="skill_proficiency">${options}</select></label>
      <label class="field small"><span>Years</span><input name="skill_years" type="number" min="0" max="60" value="${value(skill.years)}"></label>
      <label class="field small"><span>Last used</span><input name="skill_last_used" type="number" min="1970" max="2100" value="${value(skill.last_used)}"></label>
      <label class="field small"><span>Trainings</span><input name="skill_trainings" type="number" min="0" value="${value(skill.trainings_delivered)}"></label>
      <label class="field"><span>Evidence link</span><input name="skill_evidence" type="url" placeholder="https://" value="${value(skill.evidence_url)}"></label>
      <div class="row-actions">${skill.verified ? `<span class="verified-tag">${icon('badge-check')} Verified</span>` : ''}${verify}
        <button type="button" class="icon-button" data-remove-row title="Remove skill" aria-label="Remove skill">${icon('trash-2')}</button></div></div>`;
  }

  function blockRow(block, index) {
    return `<div class="repeat-row block-row" data-index="${index}">
      <label class="field small"><span>From</span><input name="block_from" type="date" value="${value(block.from)}"></label>
      <label class="field small"><span>To</span><input name="block_to" type="date" value="${value(block.to)}"></label>
      <label class="field"><span>Note</span><input name="block_note" maxlength="120" value="${value(block.note)}" placeholder="Booked elsewhere"></label>
      <div class="row-actions"><button type="button" class="icon-button" data-remove-row title="Remove period" aria-label="Remove period">${icon('trash-2')}</button></div></div>`;
  }

  function trainerForm(profile, saved) {
    return `<fieldset><legend>Profile</legend>
        ${field('name', 'Full name *', profile.name, 'required maxlength="120"')}
        <div class="field-row">${field('city', 'City *', profile.city, 'maxlength="80"')}${field('state', 'State', profile.state, 'maxlength="80"')}</div>
        <div class="field-row">${field('phone', 'Phone *', profile.phone, 'type="tel"')}${field('email', 'Email', profile.email, 'type="email"')}</div>
        <div class="field-row">${field('experience_years', 'Years of experience *', profile.experience_years, 'type="number" min="0" max="60"')}
        ${field('languages', 'Languages (comma separated)', list(profile.languages), 'placeholder="English, Kannada, Hindi"')}</div>
        <label class="field"><span>Bio</span><textarea name="bio" rows="3" maxlength="2000">${value(profile.bio)}</textarea></label></fieldset>
      <fieldset><legend>Rates and travel</legend>
        <div class="field-row">${field('rate_min', 'Day rate from (₹) *', profile.rate_min, 'type="number" min="1"')}${field('rate_max', 'Day rate up to (₹)', profile.rate_max, 'type="number" min="1"')}</div>
        <label class="field"><span>Travel</span><select name="travel">${Object.entries(TRAVEL).map(([key, text]) => `<option value="${key}" ${profile.travel === key ? 'selected' : ''}>${text}</option>`).join('')}</select></label>
        <div class="mode-options"><span>Modes *</span>${MODES.map(mode => `<label class="check-label"><input type="checkbox" name="modes" value="${mode}" ${profile.modes.includes(mode) ? 'checked' : ''}> ${mode}</label>`).join('')}</div></fieldset>
      <fieldset><legend>Skills *</legend><div id="skill-rows">${profile.skills.map((skill, index) => skillRow(skill, index, saved)).join('')}</div>
        <button type="button" class="text-button" data-add-row="skill">${icon('plus')} Add skill</button></fieldset>
      <fieldset><legend>Unavailable dates</legend><div id="block-rows">${profile.unavailable.map(blockRow).join('')}</div>
        <button type="button" class="text-button" data-add-row="block">${icon('plus')} Add unavailable period</button></fieldset>`;
  }

  function documentsHtml(record) {
    if (!record) return '<fieldset><legend>Documents</legend><p class="state-note">Save the profile first, then upload documents.</p></fieldset>';
    const kinds = editor.kind === 'vendor' ? VENDOR_DOCS : TRAINER_DOCS;
    const rows = record.documents.map(document => {
      const expired = document.expires_on && document.expires_on < today;
      return `<li><div><strong>${escape(document.kind)}</strong><small>${escape(document.file_name)} / ${Math.max(1, Math.round(document.size / 1024))} KB${document.expires_on ? ` / ${expired ? 'expired' : 'valid until'} ${escape(document.expires_on)}` : ''}</small>
        ${document.credential_url ? `<small>${escape(document.credential_url)}</small>` : ''}</div>
        ${expired ? `<span class="status-badge rejected">Expired</span>` : ''}
        <button type="button" class="icon-button" data-delete-document="${document.id}" title="Remove document" aria-label="Remove ${escape(document.kind)}">${icon('trash-2')}</button></li>`;
    }).join('');
    return `<fieldset><legend>Documents</legend><ul class="document-list">${rows || '<li class="state-note">No documents yet</li>'}</ul>
      <div class="upload-box"><div class="field-row"><label class="field"><span>Type</span><select id="document-kind">${kinds.map(kind => `<option>${kind}</option>`).join('')}</select></label>
        <label class="field" id="document-expiry" hidden><span>Valid until</span><input type="date" id="document-expires"></label></div>
        <label class="field" id="document-credential" hidden><span>Credential link</span><input type="url" id="document-url" placeholder="https://"></label>
        <label class="secondary upload-button">${icon('file-text')} Upload PDF, PNG or JPEG (max 2 MB)<input type="file" id="document-file" accept=".pdf,.png,.jpg,.jpeg,application/pdf,image/png,image/jpeg" hidden></label></div></fieldset>`;
  }

  function actionsHtml(record) {
    const status = record?.status || 'draft';
    const approve = editor.kind === 'vendor' ? 'Approve vendor' : 'Verify trainer';
    return `<div class="profile-actions">
      <button type="submit" class="primary">${icon('save')} Save</button>
      ${record && ['draft', 'rejected'].includes(status) ? `<button type="button" class="secondary" data-review="submit">${icon('send')} Submit for review</button>` : ''}
      ${status === 'pending' ? `<button type="button" class="secondary approve" data-review="approve">${icon('circle-check')} ${approve}</button>
        <button type="button" class="secondary reject" data-review="reject">${icon('x')} Reject</button>` : ''}
      ${record ? `<button type="button" class="text-button danger" data-review="delete">${icon('trash-2')} Delete</button>` : ''}
      <button type="button" class="text-button" data-close>Close</button></div>
      <div class="reject-box" hidden><label class="field"><span>Reason for rejection</span><textarea id="reject-reason" rows="2" maxlength="500"></textarea></label>
        <button type="button" class="secondary reject" data-review="confirm-reject">Confirm rejection</button></div>`;
  }

  function open(kind, record, prefill) {
    editor.kind = kind;
    editor.record = record;
    const profile = record?.profile || prefill || (kind === 'vendor'
      ? { legal_name: '', whatsapp_numbers: [], domains: [] }
      : { name: '', languages: [], modes: [], travel: 'local_only', skills: [], unavailable: [] });
    const title = kind === 'vendor' ? (profile.display_name || profile.legal_name || 'New vendor') : (profile.name || 'New trainer');
    dialog.innerHTML = `<form id="profile-form" novalidate>
      <div class="profile-head"><span class="eyebrow">${kind === 'vendor' ? 'VENDOR ONBOARDING' : 'TRAINER ONBOARDING'}</span>
        <h2 id="profile-title">${escape(title)}</h2>${record ? badge(kind, record.status) : badge(kind, 'draft')}</div>
      ${record?.review_note ? `<p class="review-note">${icon('triangle-alert')}Rejected: ${escape(record.review_note)}</p>` : ''}
      ${record ? checklistHtml(record.checklist) : '<p class="state-note">Fill in the details and save. Required fields are marked *.</p>'}
      ${kind === 'vendor' ? vendorForm(profile) : trainerForm(profile, Boolean(record))}
      ${documentsHtml(record)}
      <p id="profile-feedback" role="status" aria-live="polite"></p>
      ${actionsHtml(record)}</form>`;
    refreshIcons();
    if (!dialog.open) dialog.showModal();
  }

  const number = text => (text.trim() === '' ? null : Number(text));
  const optional = text => (text.trim() === '' ? null : text.trim());
  const split = text => text.split(',').map(part => part.trim()).filter(Boolean);

  function collect(form) {
    const get = name => form.elements[name]?.value ?? '';
    if (editor.kind === 'vendor') {
      return { legal_name: get('legal_name').trim(), display_name: optional(get('display_name')), gstin: optional(get('gstin')),
        pan: optional(get('pan')), address: optional(get('address')), city: optional(get('city')), state: optional(get('state')),
        pincode: optional(get('pincode')), website: optional(get('website')), contact_name: optional(get('contact_name')),
        contact_phone: optional(get('contact_phone')), contact_email: optional(get('contact_email')),
        whatsapp_numbers: split(get('whatsapp_numbers')), domains: split(get('domains')), notes: optional(get('notes')) };
    }
    const previous = editor.record?.profile.skills || [];
    const skills = [...form.querySelectorAll('.skill-row')].map(row => {
      const input = name => row.querySelector(`[name="${name}"]`).value;
      const name = input('skill_name').trim();
      return { name, proficiency: Number(input('skill_proficiency')), years: number(input('skill_years')),
        last_used: number(input('skill_last_used')), trainings_delivered: number(input('skill_trainings')),
        evidence_url: optional(input('skill_evidence')), verified: previous.some(skill => skill.name === name && skill.verified) };
    }).filter(skill => skill.name);
    const unavailable = [...form.querySelectorAll('.block-row')].map(row => {
      const input = name => row.querySelector(`[name="${name}"]`).value;
      return { from: input('block_from'), to: input('block_to') || input('block_from'), note: optional(input('block_note')) };
    }).filter(block => block.from);
    return { name: get('name').trim(), city: optional(get('city')), state: optional(get('state')), phone: optional(get('phone')),
      email: optional(get('email')), languages: split(get('languages')), experience_years: number(get('experience_years')),
      rate_min: number(get('rate_min')), rate_max: number(get('rate_max')), travel: get('travel') || 'local_only',
      modes: [...form.querySelectorAll('[name="modes"]:checked')].map(box => box.value), bio: optional(get('bio')), skills, unavailable };
  }

  function feedback(text, error = false) {
    const element = dialog.querySelector('#profile-feedback');
    if (!element) return;
    element.textContent = text;
    element.classList.toggle('error', error);
  }

  async function reopen(id) {
    await load();
    const records = editor.kind === 'vendor' ? data.vendors : data.trainers;
    open(editor.kind, records.find(record => record.id === id));
  }

  async function save(form) {
    const command = editor.kind === 'vendor' ? 'save_vendor' : 'save_trainer';
    return invoke(command, { id: editor.record?.id ?? null, profile: collect(form) });
  }

  async function run(work, done) {
    const buttons = [...dialog.querySelectorAll('button')];
    buttons.forEach(button => { button.disabled = true; });
    try {
      await work();
      if (done) feedback(done);
    } catch (error) {
      buttons.forEach(button => { button.disabled = false; });
      feedback(String(error), true);
    }
  }

  async function review(action, form, button) {
    const id = editor.record.id;
    const kind = editor.kind;
    if (action === 'delete') {
      if (button.dataset.armed !== 'true') {
        button.dataset.armed = 'true';
        button.lastChild.textContent = ' Tap again to delete permanently';
        return;
      }
      await run(async () => { await invoke('review_profile', { kind, id, action, reason: null }); dialog.close(); announce(`${kind === 'vendor' ? 'Vendor' : 'Trainer'} deleted`); });
      return;
    }
    if (action === 'reject') { dialog.querySelector('.reject-box').hidden = false; dialog.querySelector('#reject-reason').focus(); return; }
    const reason = action === 'confirm-reject' ? dialog.querySelector('#reject-reason').value : null;
    const command = action === 'confirm-reject' ? 'reject' : action;
    const messages = { submit: 'Submitted for review', approve: kind === 'vendor' ? 'Vendor approved and active' : 'Trainer verified', reject: 'Rejected with reason' };
    await run(async () => {
      if (command === 'submit') await save(form);
      await invoke('review_profile', { kind, id, action: command, reason });
      await reopen(id);
    }, messages[command]);
  }

  async function upload(file) {
    if (!file) return;
    if (file.size > MAX_UPLOAD) { feedback('Documents must be at most 2 MB', true); return; }
    const data64 = await new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result).split(',')[1] || '');
      reader.onerror = () => reject(new Error('Could not read the file'));
      reader.readAsDataURL(file);
    });
    const kind = dialog.querySelector('#document-kind').value;
    const document = { kind, file_name: file.name, data_base64: data64,
      credential_url: kind === 'Certificate' ? optional(dialog.querySelector('#document-url').value) : null,
      expires_on: kind === 'Certificate' ? optional(dialog.querySelector('#document-expires').value) : null };
    const id = editor.record.id;
    await run(async () => { await invoke('add_document', { kind: editor.kind, owner: id, document }); await reopen(id); }, `${kind} uploaded`);
  }

  dialog.addEventListener('submit', event => {
    event.preventDefault();
    run(async () => { const id = await save(event.target); await reopen(id); }, 'Saved');
  });
  dialog.addEventListener('click', event => {
    const target = event.target.closest('button');
    if (!target) return;
    const form = dialog.querySelector('#profile-form');
    if (target.matches('[data-close]')) { dialog.close(); return; }
    if (target.matches('[data-review]')) { review(target.dataset.review, form, target); return; }
    if (target.matches('[data-remove-row]')) { target.closest('.repeat-row').remove(); return; }
    if (target.matches('[data-add-row="skill"]')) {
      dialog.querySelector('#skill-rows').insertAdjacentHTML('beforeend', skillRow({ name: '', proficiency: 3 }, Date.now(), false));
      refreshIcons();
      return;
    }
    if (target.matches('[data-add-row="block"]')) {
      dialog.querySelector('#block-rows').insertAdjacentHTML('beforeend', blockRow({}, Date.now()));
      refreshIcons();
      return;
    }
    if (target.matches('[data-verify]')) {
      const id = editor.record.id;
      run(async () => { await invoke('verify_skill', { trainer: id, skill: target.dataset.verify, verified: target.dataset.to === 'true' }); await reopen(id); }, 'Skill verification updated');
      return;
    }
    if (target.matches('[data-delete-document]')) {
      const id = editor.record.id;
      run(async () => { await invoke('delete_document', { id: Number(target.dataset.deleteDocument) }); await reopen(id); }, 'Document removed');
    }
  });
  dialog.addEventListener('change', event => {
    if (event.target.id === 'document-file') { upload(event.target.files[0]); return; }
    if (event.target.id === 'document-kind') {
      const certificate = event.target.value === 'Certificate';
      dialog.querySelector('#document-expiry').hidden = !certificate;
      dialog.querySelector('#document-credential').hidden = !certificate;
    }
  });
  dialog.addEventListener('close', () => { onClose?.(); });

  function handleClick(event) {
    const target = event.target.closest('[data-open], [data-new], [data-suggest]');
    if (!target) return false;
    if (target.dataset.new) open(target.dataset.new, null);
    if (target.dataset.open) {
      const [kind, id] = target.dataset.open.split(':');
      open(kind, (kind === 'vendor' ? data.vendors : data.trainers).find(record => record.id === Number(id)));
    }
    if (target.dataset.suggest) {
      const suggestion = data.suggestions[Number(target.dataset.suggest)];
      open('vendor', null, { legal_name: suggestion.company || suggestion.name || '', contact_name: suggestion.name,
        contact_phone: suggestion.phone, whatsapp_numbers: [suggestion.phone], domains: [] });
    }
    return true;
  }

  return {
    load, render, handleClick,
    counts: () => ({ vendors: data.vendors.length, trainers: data.trainers.length }),
    onClose: callback => { onClose = callback; },
  };
}
