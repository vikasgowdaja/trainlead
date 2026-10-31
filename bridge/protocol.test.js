import test from 'node:test';
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { encrypt, decrypt, incomingMessage } from './protocol.js';

test('session encryption authenticates data and never contains plaintext credentials', () => {
  const key = randomBytes(32);
  const original = Buffer.from('private WhatsApp credentials');
  const encrypted = encrypt(original, key);
  assert(!encrypted.includes(original));
  assert.deepEqual(decrypt(encrypted, key), original);
  assert.throws(() => decrypt(encrypted, randomBytes(32)));
  encrypted[encrypted.length - 1] ^= 1;
  assert.throws(() => decrypt(encrypted, key));
});

test('only fresh incoming text from explicitly selected groups is collected', () => {
  const now = Date.now();
  const selected = new Set(['123@g.us']);
  const message = { key: { remoteJid: '123@g.us', id: 'abc', fromMe: false }, pushName: 'Vendor',
    messageTimestamp: Math.floor(now / 1000), message: { conversation: 'Training requirement AngularJS' } };
  assert.equal(incomingMessage(message, {'123@g.us': {subject: 'Vendors'}}, selected, now).message.source_group, 'Vendors');
  assert.equal(incomingMessage(message, {}, new Set(), now), null);
  assert.equal(incomingMessage({...message, key: {...message.key, fromMe:true}}, {}, selected, now), null);
  assert.equal(incomingMessage({...message, key: {...message.key, remoteJid:'123@s.whatsapp.net'}}, {}, selected, now), null);
  assert.equal(incomingMessage({...message, messageTimestamp: 1}, {}, selected, now), null);
});