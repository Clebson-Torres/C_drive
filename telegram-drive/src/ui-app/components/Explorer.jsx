import React from "react";
import { fmtDate, fmtDateTime, fmtSize } from "../lib/format";

function FolderTreeIcon({ active }) {
  return (
    <span className={`inline-flex h-9 w-9 items-center justify-center rounded-2xl ${active ? "bg-sky-500 text-white" : "bg-amber-100 text-amber-700"}`}>
      <svg viewBox="0 0 24 24" className="h-5 w-5 fill-current" aria-hidden="true">
        <path d="M3 6.75A2.75 2.75 0 0 1 5.75 4h4.07c.73 0 1.41.36 1.82.97l.45.68c.14.21.38.35.64.35h5.52A2.75 2.75 0 0 1 21 8.75v8.5A2.75 2.75 0 0 1 18.25 20H5.75A2.75 2.75 0 0 1 3 17.25v-10.5Z" />
      </svg>
    </span>
  );
}

function EntryTypeIcon({ kind, active }) {
  if (kind === "Folder") {
    return <FolderTreeIcon active={active} />;
  }
  return (
    <span className={`inline-flex h-9 w-9 items-center justify-center rounded-2xl ${active ? "bg-sky-500 text-white" : "bg-slate-100 text-slate-500"}`}>
      <svg viewBox="0 0 24 24" className="h-5 w-5 fill-current" aria-hidden="true">
        <path d="M7 3.75A2.75 2.75 0 0 0 4.25 6.5v11A2.75 2.75 0 0 0 7 20.25h10A2.75 2.75 0 0 0 19.75 17.5v-7.19a2.75 2.75 0 0 0-.8-1.94l-3.57-3.56a2.75 2.75 0 0 0-1.94-.81H7Zm7.25 1.91 3.59 3.59H15.5a1.25 1.25 0 0 1-1.25-1.25V5.66Z" />
      </svg>
    </span>
  );
}

export default function Explorer({
  t,
  locale,
  folders,
  listing,
  selectedEntry,
  details,
  currentFolderId,
  dragActive,
  onFolderOpen,
  onEntrySelect,
  onDragEnter,
  onDragOver,
  onDragLeave,
  onDrop,
  isDownloadBusy,
  onFileDownload,
  onFilePreview,
}) {
  const rows = [
    ...listing.folders.map((folder) => ({ ...folder, kind: "Folder" })),
    ...listing.files.map((file) => ({ ...file, kind: "File" })),
  ];

  return (
    <div className={`grid min-h-[520px] items-start gap-5 xl:grid-cols-[220px_minmax(0,1fr)_290px] ${dragActive ? "rounded-[34px] ring-4 ring-sky-200/80" : ""}`}>
      <aside className="flex max-h-[72vh] min-h-0 flex-col overflow-hidden rounded-[28px] border border-white/70 bg-white/50 p-5 shadow-glass backdrop-blur-xl">
        <div className="mb-4 text-xs font-semibold uppercase tracking-[0.26em] text-sky-700">{t("explorer.folders")}</div>
        <ul id="folderTree" className="custom-scroll min-h-0 flex-1 space-y-2 overflow-y-auto pr-1">
          {folders.map((folder) => {
            const active = currentFolderId === folder.id;
            return (
              <li key={folder.id}>
                <button
                  type="button"
                  onClick={() => onFolderOpen(folder.id)}
                  className={`flex w-full items-center gap-3 rounded-2xl px-3 py-2 text-left text-sm transition ${active ? "bg-sky-100 text-sky-900 shadow-sm" : "bg-white/70 text-slate-700 hover:bg-slate-100"}`}
                >
                  <FolderTreeIcon active={active} />
                  <span className="min-w-0 flex-1 truncate">{folder.name}</span>
                </button>
              </li>
            );
          })}
        </ul>
      </aside>

      <section
        id="dropZone"
        onDragEnter={onDragEnter}
        onDragOver={onDragOver}
        onDragLeave={onDragLeave}
        onDrop={onDrop}
        className="flex max-h-[72vh] min-h-0 flex-col gap-4 overflow-hidden rounded-[30px] border border-white/70 bg-white/55 p-5 shadow-glass backdrop-blur-xl"
      >
        <div id="dropzoneCard" className={`rounded-[24px] border-2 border-dashed p-4 transition ${dragActive ? "border-sky-400 bg-sky-50" : "border-slate-300 bg-white/75"}`}>
          <div className="flex items-center justify-between gap-6">
            <div>
              <div className="text-xs font-semibold uppercase tracking-[0.26em] text-slate-500">Drag & Drop</div>
              <div className="mt-2 text-lg font-semibold text-slate-950">{t("explorer.dropTitle")}</div>
              <p className="mt-2 text-sm leading-6 text-slate-600">{t("explorer.dropBody")}</p>
            </div>
            <div className="rounded-2xl bg-slate-900 px-4 py-3 text-sm text-slate-100">{dragActive ? t("explorer.dropStateActive") : t("explorer.dropStateIdle")}</div>
          </div>
        </div>

        <div className="flex min-h-0 flex-1 flex-col rounded-[30px] border border-slate-200 bg-white/85">
          <div className="mb-0 flex items-center justify-between px-5 pt-5"><div className="text-xs font-semibold uppercase tracking-[0.26em] text-sky-700">{t("explorer.files")}</div><div id="emptyState" className={`${rows.length ? "hidden" : "text-sm text-slate-500"}`}>{t("explorer.emptyFolder")}</div></div>
          <div className="min-h-0 flex-1 overflow-hidden rounded-[28px] p-4 pt-4">
            <div className="custom-scroll h-full overflow-auto rounded-3xl border border-slate-200">
              <table className="min-w-full table-fixed text-left text-sm">
                <colgroup>
                  <col />
                  <col className="w-[116px]" />
                  <col className="w-[100px]" />
                  <col className="w-[128px]" />
                  <col className="w-[170px]" />
                </colgroup>
                <thead className="sticky top-0 z-10 rounded-3xl bg-slate-900/95 text-slate-100">
                  <tr>
                    <th className="px-4 py-3">{t("explorer.name")}</th>
                    <th className="px-4 py-3">{t("explorer.size")}</th>
                    <th className="px-4 py-3">{t("explorer.type")}</th>
                    <th className="px-4 py-3">{t("explorer.uploadDate")}</th>
                    <th className="px-4 py-3">{t("explorer.actions")}</th>
                  </tr>
                </thead>
                <tbody id="fileRows" className="bg-white">
                  {rows.map((entry) => {
                    const active = selectedEntry?.id === entry.id && selectedEntry?.kind === entry.kind;
                    const downloadBusy = entry.kind === "File" && isDownloadBusy?.(entry);
                    return (
                      <tr
                        key={`${entry.kind}-${entry.id}`}
                        className={`border-t border-slate-200 transition ${active ? "bg-sky-50" : "hover:bg-slate-50"}`}
                        onClick={() => onEntrySelect(entry)}
                        onDoubleClick={() => entry.kind === "Folder" && onFolderOpen(entry.id)}
                      >
                        <td className="px-4 py-3 font-medium text-slate-900">
                          <div className="flex items-center gap-3">
                            <EntryTypeIcon kind={entry.kind} active={active} />
                            <div className="min-w-0">
                              <div className="truncate">{entry.name}</div>
                              {entry.origin === "Imported" ? <div className="text-xs text-slate-500">{t("explorer.imported")}</div> : null}
                            </div>
                          </div>
                        </td>
                        <td className="px-4 py-3 text-slate-600 whitespace-nowrap">{entry.kind === "Folder" ? "--" : fmtSize(entry.size, locale)}</td>
                        <td className="px-4 py-3 text-slate-600 whitespace-nowrap">{entry.kind === "Folder" ? t("explorer.typeFolder") : entry.storage_mode || entry.mime_type}</td>
                        <td className="px-4 py-3 text-slate-600 whitespace-nowrap">{fmtDate(entry.updated_at || entry.created_at, locale)}</td>
                        <td className="px-4 py-3 whitespace-nowrap">
                          {entry.kind === "File" ? (
                            <div className="flex gap-2">
                              <button type="button" disabled={downloadBusy} className={`dl-btn table-btn ${downloadBusy ? "cursor-not-allowed opacity-60" : ""}`} onClick={(event) => { event.stopPropagation(); onFileDownload(entry); }}>{t("explorer.download")}</button>
                              {String(entry.mime_type || "").startsWith("image/") ? <button type="button" className="table-btn" onClick={(event) => { event.stopPropagation(); onFilePreview(entry); }}>{t("explorer.preview")}</button> : null}
                            </div>
                          ) : (
                            <button type="button" className="table-btn" onClick={(event) => { event.stopPropagation(); onFolderOpen(entry.id); }}>{t("explorer.open")}</button>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </section>

      <aside className="flex max-h-[72vh] min-h-0 flex-col overflow-hidden rounded-[28px] border border-white/70 bg-white/50 p-5 shadow-glass backdrop-blur-xl">
        <div className="mb-4 text-xs font-semibold uppercase tracking-[0.26em] text-sky-700">{t("explorer.details")}</div>
        <div id="selectionDetails" className="custom-scroll min-h-0 flex-1 space-y-3 overflow-y-auto pr-1 text-sm text-slate-700">
          {details ? (
            <>
              <div className="text-xl font-semibold text-slate-950">{details.name}</div>
              <div>{t("explorer.detailType")}: {details.kind === "Folder" ? t("explorer.typeFolder") : details.storageMode || details.mimeType || "File"}</div>
              <div>{t("explorer.detailSize")}: {details.kind === "Folder" ? "--" : fmtSize(details.size, locale)}</div>
              <div>{t("explorer.detailUpdated")}: {details.updatedAt ? fmtDateTime(details.updatedAt, locale) : "--"}</div>
              {details.folderId !== undefined ? <div>{t("explorer.detailFolder")}: {details.folderId}</div> : null}
              <div className="mt-4 flex flex-col gap-3">
                {details.kind === "File" ? (
                  <>
                    <button type="button" disabled={isDownloadBusy?.(selectedEntry)} className={`primary-btn w-full ${isDownloadBusy?.(selectedEntry) ? "cursor-not-allowed opacity-60" : ""}`} onClick={() => onFileDownload(selectedEntry)}>{t("explorer.detailDownload")}</button>
                    {String(details.mimeType || "").startsWith("image/") ? <button type="button" className="ghost-btn w-full" onClick={() => onFilePreview(selectedEntry)}>{t("explorer.detailPreview")}</button> : null}
                  </>
                ) : (
                  <button type="button" className="primary-btn w-full" onClick={() => onFolderOpen(selectedEntry.id)}>{t("explorer.detailOpenFolder")}</button>
                )}
              </div>
            </>
          ) : (
            <div className="rounded-2xl border border-dashed border-slate-300 bg-white/70 p-5 text-slate-500">{t("explorer.emptySelection")}</div>
          )}
        </div>
      </aside>
    </div>
  );
}
