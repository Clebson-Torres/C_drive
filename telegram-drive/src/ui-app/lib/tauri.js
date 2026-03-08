export function getTauri() {
  const tauri = window.__TAURI__ || {};
  const tauriInternals = window.__TAURI_INTERNALS__ || {};
  return {
    invoke: tauri.core?.invoke || tauri.invoke || tauriInternals.invoke,
    listen: tauri.event?.listen || tauriInternals.event?.listen,
    convertFileSrc: tauri.core?.convertFileSrc || tauri.convertFileSrc || tauriInternals.convertFileSrc,
  };
}

export async function call(cmd, args = {}) {
  const { invoke } = getTauri();
  if (!invoke) {
    throw new Error("Tauri invoke API unavailable");
  }
  const response = await invoke(cmd, args);
  if (!response.ok) {
    throw new Error(response.error || "unknown backend error");
  }
  return response.data;
}

export async function listenEvent(name, handler) {
  const { listen } = getTauri();
  if (!listen) {
    return () => {};
  }
  return listen(name, handler);
}

export function toAssetUrl(path) {
  if (!path) return null;
  const { convertFileSrc } = getTauri();
  if (typeof convertFileSrc === "function") {
    try {
      return convertFileSrc(path);
    } catch {
      // fall through
    }
  }
  const normalized = String(path).replace(/\\/g, "/");
  if (/^[a-zA-Z]:\//.test(normalized)) {
    return `file:///${normalized}`;
  }
  return normalized;
}
