import React from "react";

export default function NewFolderModal({ t, open, value, onChange, onConfirm, onCancel }) {
  if (!open) return <div id="newFolderModal" className="hidden" />;
  return <div id="newFolderModal" className="modal-shell"><div className="modal-card max-w-lg"><div className="text-xs font-semibold uppercase tracking-[0.26em] text-sky-700">{t("newFolder.badge")}</div><h3 className="mt-2 text-2xl font-semibold text-slate-950">{t("newFolder.title")}</h3><label className="mt-6 block text-sm font-medium text-slate-700">{t("newFolder.label")}<input id="newFolderInput" value={value} onChange={(event) => onChange(event.target.value)} className="mt-2 w-full rounded-2xl border border-slate-200 bg-white px-4 py-3" /></label><div className="mt-8 flex justify-end gap-3"><button id="btnNewFolderCancel" type="button" className="ghost-btn" onClick={onCancel}>{t("newFolder.cancel")}</button><button id="btnNewFolderConfirm" type="button" className="primary-btn" onClick={onConfirm}>{t("newFolder.confirm")}</button></div></div></div>;
}
