import { normalizeLocale } from "./i18n";

const LABELS = {
  "pt-BR": {
    state: {
      Queued: "Na fila",
      Running: "Ativo",
      Paused: "Pausado",
      Completed: "Concluído",
      Failed: "Erro",
      Cancelled: "Cancelado",
    },
    phase: {
      Queued: "Na fila",
      Hashing: "Hashing",
      Chunking: "Chunking",
      Encrypting: "Encrypting",
      Uploading: "Uploading",
      Downloading: "Downloading",
      Reassembling: "Reassembling",
      Completed: "Concluído",
      Failed: "Erro",
      Cancelled: "Cancelado",
    },
    mode: {
      Single: "single",
      Chunked: "chunked",
    },
    upload: "Upload",
    download: "Download",
    calculating: "calculando",
  },
  "en-US": {
    state: {
      Queued: "Queued",
      Running: "Running",
      Paused: "Paused",
      Completed: "Completed",
      Failed: "Failed",
      Cancelled: "Cancelled",
    },
    phase: {
      Queued: "Queued",
      Hashing: "Hashing",
      Chunking: "Chunking",
      Encrypting: "Encrypting",
      Uploading: "Uploading",
      Downloading: "Downloading",
      Reassembling: "Reassembling",
      Completed: "Completed",
      Failed: "Failed",
      Cancelled: "Cancelled",
    },
    mode: {
      Single: "single",
      Chunked: "chunked",
    },
    upload: "Upload",
    download: "Download",
    calculating: "calculating",
  },
};

function labelsFor(locale) {
  return LABELS[normalizeLocale(locale)] || LABELS["pt-BR"];
}

export function errText(err) {
  if (typeof err === "string") return err;
  if (err?.message) return err.message;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

export function fmtSize(bytes, locale = "pt-BR") {
  const value = Number(bytes || 0);
  const resolved = normalizeLocale(locale);
  const nf = new Intl.NumberFormat(resolved, { maximumFractionDigits: value < 1024 ** 3 ? 1 : 2, minimumFractionDigits: value < 1024 ? 0 : value < 1024 ** 3 ? 1 : 2 });
  if (value < 1024) return `${nf.format(value)} B`;
  if (value < 1024 ** 2) return `${nf.format(value / 1024)} KB`;
  if (value < 1024 ** 3) return `${nf.format(value / 1024 ** 2)} MB`;
  return `${nf.format(value / 1024 ** 3)} GB`;
}

export function fmtSpeed(bps, locale = "pt-BR") {
  return bps ? `${fmtSize(bps, locale)}/s` : labelsFor(locale).calculating;
}

export function fmtTimeStamp(value, locale = "pt-BR") {
  if (!value) return "--";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return "--";
  return new Intl.DateTimeFormat(normalizeLocale(locale), { hour: "2-digit", minute: "2-digit" }).format(parsed);
}

export function fmtExpectedTime(seconds, locale = "pt-BR") {
  if (seconds === null || seconds === undefined) return "--";
  const parsed = new Date(Date.now() + Number(seconds) * 1000);
  if (Number.isNaN(parsed.getTime())) return "--";
  return new Intl.DateTimeFormat(normalizeLocale(locale), { hour: "2-digit", minute: "2-digit" }).format(parsed);
}

export function fmtDate(value, locale = "pt-BR") {
  if (!value) return "--";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return "--";
  return new Intl.DateTimeFormat(normalizeLocale(locale), { year: "numeric", month: "2-digit", day: "2-digit" }).format(parsed);
}

export function fmtDateTime(value, locale = "pt-BR") {
  if (!value) return "--";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return "--";
  return new Intl.DateTimeFormat(normalizeLocale(locale), {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(parsed);
}

export function normalizeChunkSize(bytes) {
  const value = Number(bytes);
  const allowed = [64, 128, 256].map((mib) => mib * 1024 * 1024);
  return allowed.includes(value) ? value : 128 * 1024 * 1024;
}

export function chunkSizeLabel(bytes) {
  return `${Math.round(normalizeChunkSize(bytes) / 1024 / 1024)} MiB`;
}

export function normalizeDownloadCacheThreshold(bytes) {
  const value = Number(bytes);
  return Number.isFinite(value) && value > 0 ? value : 2 * 1024 * 1024 * 1024;
}

export function defaultDownloadCacheModeForSize(bytes, thresholdBytes) {
  const threshold = normalizeDownloadCacheThreshold(thresholdBytes);
  return Number(bytes) > threshold ? "enabled" : "disabled";
}

export function transferDirection(transfer, locale = "pt-BR") {
  return String(transfer?.job_id || "").startsWith("download-") ? labelsFor(locale).download : labelsFor(locale).upload;
}

export function stateLabel(stateValue, locale = "pt-BR") {
  return labelsFor(locale).state[stateValue] || stateValue;
}

export function phaseLabel(phaseValue, locale = "pt-BR") {
  return labelsFor(locale).phase[phaseValue] || phaseValue;
}

export function modeLabel(modeValue, locale = "pt-BR") {
  return labelsFor(locale).mode[modeValue] || modeValue;
}

export function isTerminal(stateValue) {
  return stateValue === "Completed" || stateValue === "Failed" || stateValue === "Cancelled";
}

export function isActiveState(stateValue) {
  return stateValue === "Running" || stateValue === "Queued" || stateValue === "Paused";
}

export function dedupeTransfersForDisplay(items) {
  const grouped = new Map();
  for (const transfer of items) {
    const direction = String(transfer?.job_id || "").startsWith("download-") ? "download" : "upload";
    const key = `${direction}:${transfer.file_name}:${isActiveState(transfer.state) ? "active" : transfer.state}`;
    const current = grouped.get(key);
    if (!current) {
      grouped.set(key, transfer);
      continue;
    }
    const currentTime = new Date(current.updated_at).getTime();
    const nextTime = new Date(transfer.updated_at).getTime();
    if (nextTime >= currentTime) {
      grouped.set(key, transfer);
    }
  }
  return [...grouped.values()].sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime());
}
