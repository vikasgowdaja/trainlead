import makeWASocket, { DisconnectReason } from '@whiskeysockets/baileys';
import QRCode from 'qrcode';
import pino from 'pino';
import { createInterface } from 'node:readline';
import { rm } from 'node:fs/promises';
import { encryptedSession } from './session.js';
import { incomingMessage } from './protocol.js';

const api = new URL(process.env.TL_API || 'http://127.0.0.1:1423');
if (api.hostname !== '127.0.0.1' || api.protocol !== 'http:' || !process.env.TL_KEY) throw new Error('Bridge requires an authenticated loopback API');
const key = Buffer.from(process.env.TL_SESSION_KEY || '', 'base64');
if (key.length !== 32 || !process.env.TL_SESSION_FILE) throw new Error('Encrypted session storage is required');
const session = await encryptedSession(process.env.TL_SESSION_FILE, key);
let selected = new Set(session.data.selected);
let groups = {};
let socket;
let stopped = false;
let retries = 0;
let reconnectTimer;
let posting = false;
const emit = event => process.stdout.write(`${JSON.stringify(event)}\n`);

async function flush() {
  if (posting || stopped || !session.data.pending.length) return;
  posting = true;
  try {
    while (session.data.pending.length && !stopped) {
      const batch = session.data.pending.slice(0, 8);
      const response = await fetch(new URL('/api/ingest/messages', api), {
        method: 'POST', headers: {'content-type':'application/json', 'x-key':process.env.TL_KEY},
        body: JSON.stringify({messages: batch}), signal: AbortSignal.timeout(120000),
      });
      if (!response.ok) throw new Error('Ingestion unavailable');
      session.data.pending.splice(0, batch.length);
      await session.save();
    }
  } catch { emit({type:'notice', message:'Messages retained locally; ingestion will retry'}); }
  finally { posting = false; }
}

async function start() {
  if (stopped) return;
  emit({type:'status', state:'connecting'});
  socket = makeWASocket({auth:session.auth, logger:pino({level:'silent'}), markOnlineOnConnect:false,
    syncFullHistory:false, shouldSyncHistoryMessage:() => false, printQRInTerminal:false});
  socket.ev.on('creds.update', () => session.save().catch(() => emit({type:'notice',message:'Could not save encrypted session'})));
  socket.ev.on('connection.update', async update => {
    try {
      if (update.qr) emit({type:'qr', image:await QRCode.toDataURL(update.qr, {margin:3, width:320}), expires_in:60});
      if (update.connection === 'open') {
        retries = 0;
        groups = await socket.groupFetchAllParticipating();
        emit({type:'groups', groups:Object.entries(groups).map(([id, group]) => ({id, name:group.subject}))});
        emit({type:'status', state:'connected'});
        await flush();
      }
      if (update.connection === 'close' && !stopped) {
        const code = update.lastDisconnect?.error?.output?.statusCode;
        if (code === DisconnectReason.loggedOut) { emit({type:'status',state:'logged_out'}); return; }
        if (++retries > 5) { emit({type:'status',state:'error'}); return; }
        emit({type:'status',state:'reconnecting'});
        reconnectTimer = setTimeout(() => start().catch(() => emit({type:'status',state:'error'})), Math.min(30000, 1000 * 2 ** retries));
      }
    } catch { emit({type:'notice',message:'WhatsApp connection update failed'}); }
  });
  socket.ev.on('messages.upsert', async event => {
    if (event.type !== 'notify' || stopped) return;
    try {
      for (const message of event.messages) {
        const incoming = incomingMessage(message, groups, selected);
        if (incoming && !session.data.pending.some(entry => entry.event_id === incoming.event_id)) session.data.pending.push(incoming);
      }
      await session.save();
      await flush();
    } catch { emit({type:'notice',message:'Could not retain an incoming message'}); }
  });
}

const interval = setInterval(flush, 15000);
createInterface({input:process.stdin}).on('line', async line => {
  try {
    const command = JSON.parse(line);
    if (command.type === 'select') {
      selected = new Set(command.groups.filter(id => id.endsWith('@g.us')));
      session.data.selected = [...selected];
      await session.save();
    }
    if (command.type === 'disconnect' || command.type === 'unlink') {
      stopped = true;
      clearTimeout(reconnectTimer); clearInterval(interval);
      if (command.type === 'unlink') {
        await socket?.logout();
        await session.save();
        await rm(process.env.TL_SESSION_FILE, {force:true});
      } else { socket?.end(new Error('Local disconnect')); await session.save(); }
      emit({type:'status',state:'disconnected'});
      process.exit(0);
    }
  } catch { emit({type:'notice',message:'Bridge command failed'}); }
});
process.stdin.on('end', () => { stopped=true; clearInterval(interval); clearTimeout(reconnectTimer); socket?.end(new Error('Host stopped')); });
process.on('uncaughtException', () => { emit({type:'status',state:'error'}); process.exit(1); });
start().catch(() => { emit({type:'status',state:'error'}); process.exit(1); });
