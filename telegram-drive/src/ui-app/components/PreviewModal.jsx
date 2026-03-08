import React from "react";

export default function PreviewModal({ t, open, path, onClose }) {
  if (!open || !path) return <div id="previewModal" className="hidden"><img id="previewImage" alt="" /></div>;
  return <div id="previewModal" className="modal-shell"><div className="modal-card max-w-4xl"><div className="mb-4 flex justify-end"><button type="button" className="ghost-btn" onClick={onClose}>{t("preview.close")}</button></div><img id="previewImage" src={path} alt={t("preview.alt")} className="max-h-[70vh] w-full rounded-3xl object-contain" /></div></div>;
}
