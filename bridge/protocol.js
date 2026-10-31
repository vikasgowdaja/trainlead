import { createCipheriv, createDecipheriv, randomBytes } from 'node:crypto';

export function encrypt(bytes, key) {
  if (key.length !== 32) throw new Error('Session encryption requires a 32-byte key');
  const nonce = randomBytes(12);
  const cipher = createCipheriv('aes-256-gcm', key, nonce);
  const ciphertext = Buffer.concat([cipher.update(bytes), cipher.final()]);
  return Buffer.concat([nonce, cipher.getAuthTag(), ciphertext]);
}

export function decrypt(bytes, key) {
  if (key.length !== 32 || bytes.length < 28) throw new Error('Invalid encrypted session');
  const decipher = createDecipheriv('aes-256-gcm', key, bytes.subarray(0, 12));
  decipher.setAuthTag(bytes.subarray(12, 28));
  return Buffer.concat([decipher.update(bytes.subarray(28)), decipher.final()]);
}

export function incomingMessage(message, groups, selected, now = Date.now()) {
  const jid = message.key?.remoteJid;
  if (!jid?.endsWith('@g.us') || message.key.fromMe || !message.key.id || !selected.has(jid)) return null;
  const content = message.message?.ephemeralMessage?.message || message.message?.viewOnceMessage?.message || message.message;
  const text = content?.conversation || content?.extendedTextMessage?.text || content?.imageMessage?.caption || content?.videoMessage?.caption;
  if (!text?.trim()) return null;
  const seconds = Number(message.messageTimestamp);
  if (!Number.isFinite(seconds) || seconds * 1000 < now - 24 * 60 * 60 * 1000) return null;
  return {
    event_id: `${jid}:${message.key.id}`,
    message: { source_group: groups[jid]?.subject || jid, sender: message.pushName || message.key.participant || null,
      timestamp: new Date(seconds * 1000).toISOString(), raw_text: text },
  };
}