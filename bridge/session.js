import { readFile, writeFile, rename, mkdir } from 'node:fs/promises';
import { dirname } from 'node:path';
import { BufferJSON, initAuthCreds, proto } from '@whiskeysockets/baileys';
import { encrypt, decrypt } from './protocol.js';

export async function encryptedSession(path, key) {
  let data;
  try { data = JSON.parse(decrypt(await readFile(path), key).toString(), BufferJSON.reviver); }
  catch (error) { if (error.code !== 'ENOENT') throw error; }
  data ||= { creds: initAuthCreds(), keys: {}, selected: [], pending: [] };
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  let writing = Promise.resolve();
  function save() {
    const bytes = encrypt(Buffer.from(JSON.stringify(data, BufferJSON.replacer)), key);
    writing = writing.then(async () => {
      await writeFile(`${path}.tmp`, bytes, {mode: 0o600});
      await rename(`${path}.tmp`, path);
    });
    return writing;
  }
  return {
    data, save,
    auth: { creds: data.creds, keys: {
      get: async (type, ids) => {
        const result = {};
        for (const id of ids) {
          const value = data.keys[`${type}:${id}`];
          if (value) result[id] = type === 'app-state-sync-key' ? proto.Message.AppStateSyncKeyData.fromObject(value) : value;
        }
        return result;
      },
      set: async updates => {
        for (const [type, entries] of Object.entries(updates)) {
          for (const [id, value] of Object.entries(entries)) {
            if (value) data.keys[`${type}:${id}`] = value; else delete data.keys[`${type}:${id}`];
          }
        }
        await save();
      },
    } },
  };
}