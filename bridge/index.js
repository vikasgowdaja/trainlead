// Linked-device bridge. Use a SECONDARY number that is a member of the groups. Read-only: never sends.
// Run: node index.js, then WhatsApp > Linked devices > Link a device > scan the QR printed here.
import makeWASocket, { useMultiFileAuthState, DisconnectReason } from '@whiskeysockets/baileys'
import qrcode from 'qrcode-terminal'
const API = process.env.API || 'http://127.0.0.1:8000', KEY = process.env.TL_KEY || ''
const only = (process.env.GROUPS || '').split(',').map(s => s.trim().toLowerCase()).filter(Boolean) // e.g. GROUPS="trainer,college"
const names = {}
async function start() {
  const { state, saveCreds } = await useMultiFileAuthState('auth')
  const sock = makeWASocket({ auth: state })
  sock.ev.on('creds.update', saveCreds)
  sock.ev.on('connection.update', ({ connection, lastDisconnect, qr }) => {
    if (qr) qrcode.generate(qr, { small: true })
    if (connection === 'open') console.log('linked, listening to groups')
    if (connection === 'close' && lastDisconnect?.error?.output?.statusCode !== DisconnectReason.loggedOut) start()
  })
  sock.ev.on('messages.upsert', async ({ messages }) => {
    for (const m of messages) {
      const jid = m.key.remoteJid
      if (!jid?.endsWith('@g.us') || m.key.fromMe) continue
      const text = m.message?.conversation || m.message?.extendedTextMessage?.text || m.message?.imageMessage?.caption
      if (!text) continue
      try {
        names[jid] ??= (await sock.groupMetadata(jid)).subject
        if (only.length && !only.some(o => names[jid].toLowerCase().includes(o))) continue
        const phone = (m.key.participant || '').split('@')[0]
        await fetch(API + '/api/ingest/messages', { method: 'POST', headers: { 'content-type': 'application/json', 'x-key': KEY },
          body: JSON.stringify({ source: names[jid], messages: [{ sender: `${m.pushName || ''} ${phone}`.trim(), ts: new Date(m.messageTimestamp * 1000).toISOString(), text }] }) })
      } catch (e) { console.error(e.message) }
    }
  })
}
start()
