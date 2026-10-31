import { mkdir, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';

async function findWebView() {
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch('http://127.0.0.1:9223/json', { signal: AbortSignal.timeout(2000) });
      const pages = await response.json();
      const target = pages.find(entry => entry.title === 'TrainLead');
      if (target?.webSocketDebuggerUrl) return target;
    } catch (error) {
      if (Date.now() >= deadline) throw error;
    }
    await delay(200);
  }
  throw new Error('TrainLead WebView did not become ready within 30 seconds');
}
const target = await findWebView();
assert(target?.webSocketDebuggerUrl, 'TrainLead WebView is not available on forwarded port 9223');
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  socket.addEventListener('open', resolve, { once: true });
  socket.addEventListener('error', reject, { once: true });
});
let nextId = 0;
const requests = new Map();
socket.addEventListener('message', event => {
  const message = JSON.parse(event.data);
  const pending = requests.get(message.id);
  if (!pending) return;
  requests.delete(message.id);
  if (message.error) pending.reject(new Error(message.error.message));
  else pending.resolve(message.result);
});
function send(method, params = {}) {
  const id = ++nextId;
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => { requests.delete(id); reject(new Error(`${method} timed out`)); }, 30000);
    requests.set(id, {
      resolve: result => { clearTimeout(timeout); resolve(result); },
      reject: error => { clearTimeout(timeout); reject(error); },
    });
    socket.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const result = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
  return result.result.value;
}

try {
  await send('Runtime.enable');
  await send('Page.enable');
  const initial = await evaluate(`({native: Boolean(window.__TAURI_INTERNALS__), title: document.title, status: document.querySelector('#status').textContent})`);
  assert.equal(initial.native, true);
  assert(!initial.status.includes('not allowed'), initial.status);
  await evaluate(`(() => {
    if (document.querySelector('#privacy').open) {
      const checkbox = document.querySelector('#consent');
      checkbox.checked = true; checkbox.dispatchEvent(new Event('change'));
      document.querySelector('#privacy-continue').click();
    }
  })()`);
  const demo = await evaluate(`(async () => {
    const button = document.querySelector('#demo');
    button.click();
    await new Promise((resolve, reject) => {
      const observer = new MutationObserver(() => {
        if (!button.disabled) { observer.disconnect(); resolve(); }
      });
      observer.observe(button, {attributes:true, attributeFilter:['disabled']});
      setTimeout(() => { observer.disconnect(); reject(new Error('Demo did not finish')); }, 15000);
    });
    return {status:document.querySelector('#status').textContent,cards:document.querySelectorAll('.lead-card').length,text:document.querySelector('#leads').textContent};
  })()`);
  assert.equal(demo.cards, 2);
  assert(demo.status.startsWith('Demo: found 4'), demo.status);
  assert(demo.text.includes('Mysuru') && demo.text.includes('Bengaluru'));
  const expectedRepeated = await evaluate(`document.querySelectorAll('.repeat-badge.repeated').length`);
  const repeatCount = await evaluate(`(() => {
    const repeated = document.querySelector('#repeated'); repeated.checked=true; repeated.dispatchEvent(new Event('input'));
    return document.querySelectorAll('.lead-card').length;
  })()`);
  assert.equal(repeatCount, expectedRepeated);
  assert(repeatCount >= 1);
  const searchCount = await evaluate(`(() => {
    const repeated = document.querySelector('#repeated'); repeated.checked=false; repeated.dispatchEvent(new Event('input'));
    const search = document.querySelector('#search'); search.value='Bengaluru'; search.dispatchEvent(new Event('input'));
    const count = document.querySelectorAll('.lead-card').length;
    search.value=''; search.dispatchEvent(new Event('input')); return count;
  })()`);
  assert.equal(searchCount, 1);
  const capture = await evaluate(`(async () => {
    const input = document.querySelector('#message'); input.value='Emulator smoke test: training requirement pending review';
    const button = document.querySelector('#capture-form button[type="submit"]');
    document.querySelector('#capture-form').requestSubmit();
    await new Promise((resolve, reject) => {
      const observer = new MutationObserver(() => { if(!button.disabled){ observer.disconnect(); resolve(); } });
      observer.observe(button, {attributes:true, attributeFilter:['disabled']});
      setTimeout(() => { observer.disconnect(); reject(new Error('Capture did not finish')); },15000);
    });
    const status=document.querySelector('#status').textContent;
    const saved=document.querySelector('#leads').textContent.includes('Emulator smoke test');
    document.querySelector('#tab-leads').click();
    return {status,saved};
  })()`);
  assert(capture.saved, capture.status);
  assert(capture.status.startsWith('Saved 1'), capture.status);
  const layout = await evaluate(`({width:innerWidth,scrollWidth:document.documentElement.scrollWidth,icons:document.querySelectorAll('svg').length})`);
  assert(layout.scrollWidth <= layout.width, 'Mobile page overflows horizontally');
  assert(layout.icons > 15, 'Icons failed to render');
  await evaluate(`document.documentElement.dataset.theme='light'; localStorage.setItem('trainlead-theme','light'); document.querySelector('main').scrollIntoView()`);
  await mkdir(new URL('../.smoke/', import.meta.url), { recursive: true });
  const screenshot = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
  await writeFile(new URL('../.smoke/android-light.png', import.meta.url), Buffer.from(screenshot.data, 'base64'));
  console.log(JSON.stringify({ initial, demo: { status: demo.status, cards: demo.cards }, repeatCount, searchCount, capture, layout }, null, 2));
} finally {
  socket.close();
}