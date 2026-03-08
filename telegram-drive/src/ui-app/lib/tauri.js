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

function safeSerialize(data) {
  try {
    return JSON.stringify(data);
  } catch {
    return String(data);
  }
}

export async function debugLog(channel, message, data) {
  const suffix = data === undefined ? "" : ` | ${safeSerialize(data)}`;
  const line = `${message}${suffix}`;
  console.info(`[savedrive:${channel}] ${line}`);
  try {
    await call("frontend_debug_log", { channel, message: line });
  } catch {
    // best-effort only
  }
}

export async function listenEvent(name, handler) {
  const { listen } = getTauri();
  if (!listen) {
    return () => {};
  }
  return listen(name, handler);
}

export async function listenFileDropEvent(handler) {
  if (!window.__TAURI__ && !window.__TAURI_INTERNALS__) {
    console.info("[savedrive:dragdrop] tauri APIs unavailable for native dragdrop listener");
    return () => {};
  }

  try {
    const { getCurrentWebview } = await import("@tauri-apps/api/webview");
    const current = getCurrentWebview?.();
    if (current?.onDragDropEvent) {
      console.info("[savedrive:dragdrop] native webview dragdrop listener attached");
      return current.onDragDropEvent(handler);
    }
  } catch {
    // fall through to legacy listener path
  }

  console.info("[savedrive:dragdrop] native webview dragdrop listener unavailable");
  return () => {};
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
