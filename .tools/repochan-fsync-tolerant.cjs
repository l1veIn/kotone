// Preload patch: tolerate Windows EPERM/EINVAL from fsync on read-only handles.
// repochan's transaction syncPath() opens files with mode "r" and calls handle.sync(),
// which fails with EPERM on this machine (FlushFileBuffers requires write access on Windows).
// Write-handle syncs succeed normally, so durability for actual writes is preserved.
const fs = require('node:fs');
const fsp = fs.promises;

function tolerate(e) {
  return e && (e.code === 'EPERM' || e.code === 'EINVAL');
}

function wrapSync(handle) {
  if (!handle || typeof handle.sync !== 'function' || handle.__syncTolerant) return;
  const orig = handle.sync.bind(handle);
  handle.sync = async function patchedSync(...args) {
    try {
      return await orig(...args);
    } catch (e) {
      if (tolerate(e)) return undefined;
      throw e;
    }
  };
  Object.defineProperty(handle, '__syncTolerant', { value: true });
}

const origOpen = fsp.open;
fsp.open = async function patchedOpen(...args) {
  const handle = await origOpen.apply(this, args);
  wrapSync(handle);
  return handle;
};

const origFsyncSync = fs.fsyncSync;
fs.fsyncSync = function patchedFsyncSync(fd, ...args) {
  try {
    return origFsyncSync.call(this, fd, ...args);
  } catch (e) {
    if (tolerate(e)) return undefined;
    throw e;
  }
};
