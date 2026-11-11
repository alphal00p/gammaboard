// Page-session LRU: no large results in localStorage, and no cached control
// responses. Keys include the run, task, section and request phase.
const entries = new Map();
const MAX_BYTES = 64 * 1024 * 1024;
const MAX_ENTRIES = 64;
const TTL_MS = 30 * 60 * 1000;
let bytes = 0;

export function clearResourceCache() { entries.clear(); bytes = 0; }
export function deleteResourceCache(key) { const entry = entries.get(key); if (entry) { bytes -= entry.bytes; entries.delete(key); } }
export function readResourceCache(key) {
  const entry = entries.get(key);
  if (!entry) return null;
  if (Date.now() - entry.saved > TTL_MS) { entries.delete(key); bytes -= entry.bytes; return null; }
  entries.delete(key); entries.set(key, entry);
  return entry.value;
}
export function writeResourceCache(key, value) {
  if (!key) return;
  const size = JSON.stringify(value).length * 2;
  const previous = entries.get(key);
  if (previous) { bytes -= previous.bytes; entries.delete(key); }
  if (size > MAX_BYTES) return;
  entries.set(key, { value, bytes: size, saved: Date.now() }); bytes += size;
  while (bytes > MAX_BYTES || entries.size > MAX_ENTRIES) {
    const oldest = entries.keys().next().value;
    bytes -= entries.get(oldest).bytes; entries.delete(oldest);
  }
}
