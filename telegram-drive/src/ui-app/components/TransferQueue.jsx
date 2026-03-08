import React, { useEffect } from "react";
import {
  dedupeTransfersForDisplay,
  fmtExpectedTime,
  fmtSize,
  fmtSpeed,
  fmtTimeStamp,
  isTerminal,
  modeLabel,
  phaseLabel,
  stateLabel,
  transferDirection,
} from "../lib/format";

export default function TransferQueue({ t, locale, open, transfers, onClose, onPause, onResume, onCancel }) {
  const items = dedupeTransfersForDisplay(transfers);

  useEffect(() => {
    if (!open) return undefined;
    const onKeyDown = (event) => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, open]);

  if (!open) {
    return <div id="queueDrawer" className="hidden" />;
  }

  return (
    <div id="queueDrawer" className="fixed inset-0 z-[120]">
      <button
        id="queueBackdrop"
        type="button"
        aria-label="Close queue"
        onClick={onClose}
        className="absolute inset-0 bg-slate-950/35 backdrop-blur-[2px]"
      />
      <section className="absolute inset-x-5 top-5 mx-auto max-w-[1720px] rounded-[30px] border border-white/70 bg-white/95 p-6 shadow-glass backdrop-blur-xl">
        <div className="mb-5 flex items-start justify-between gap-4">
          <div>
            <div className="text-xs font-semibold uppercase tracking-[0.26em] text-sky-700">{t("queue.title")}</div>
            <h3 className="mt-2 text-3xl font-semibold text-slate-950">{t("queue.subtitle")}</h3>
          </div>
          <button id="btnQueueClose" type="button" className="ghost-btn !px-4 !py-2" onClick={onClose}>
            {t("queue.close")}
          </button>
        </div>

        <div id="progressList" className="custom-scroll max-h-[58vh] space-y-4 overflow-y-auto pr-1">
          {items.length ? items.map((transfer) => {
            const percentage = transfer.bytes_total ? Math.min(100, Math.round((transfer.bytes_done / transfer.bytes_total) * 100)) : 0;
            const terminal = isTerminal(transfer.state);
            const paused = transfer.state === "Paused";
            return (
              <article key={transfer.job_id} className="rounded-[24px] border border-slate-200 bg-white p-5">
                <div className="flex items-start justify-between gap-4">
                  <div className="min-w-0">
                    <div className="truncate text-lg font-semibold text-slate-950">{transfer.file_name}</div>
                    <div className="mt-2 flex flex-wrap gap-2 text-xs font-semibold uppercase tracking-[0.16em]">
                      <span className="rounded-full bg-sky-100 px-3 py-1 text-sky-800">{transferDirection(transfer, locale)}</span>
                      <span className="rounded-full bg-slate-100 px-3 py-1 text-slate-700">{stateLabel(transfer.state, locale)}</span>
                      <span className="rounded-full bg-slate-100 px-3 py-1 text-slate-700">{phaseLabel(transfer.phase, locale)}</span>
                      {transfer.storage_mode ? <span className="rounded-full bg-slate-100 px-3 py-1 text-slate-700">{modeLabel(transfer.storage_mode, locale)}</span> : null}
                    </div>
                  </div>
                  <div className="flex shrink-0 gap-2">
                    {transfer.state === "Running" ? <button type="button" className="ghost-btn !px-4 !py-2" onClick={() => onPause(transfer.job_id)}>{t("queue.pause")}</button> : null}
                    {paused ? <button type="button" className="ghost-btn !px-4 !py-2" onClick={() => onResume(transfer.job_id)}>{t("queue.resume")}</button> : null}
                    {!terminal ? <button type="button" className="danger-btn !px-4 !py-2" onClick={() => onCancel(transfer.job_id)}>{t("queue.cancel")}</button> : null}
                  </div>
                </div>

                <div className="mt-5 h-3 overflow-hidden rounded-full bg-slate-200">
                  <div
                    className={`h-full rounded-full ${terminal ? "bg-emerald-500" : "bg-gradient-to-r from-sky-500 to-blue-700"}`}
                    style={{ width: `${percentage}%` }}
                  />
                </div>

                <div className="mt-4 grid gap-3 text-sm text-slate-600 md:grid-cols-3 xl:grid-cols-6">
                  <div><div className="text-[11px] uppercase tracking-[0.18em] text-slate-400">{t("queue.start")}</div><div>{fmtTimeStamp(transfer.started_at, locale)}</div></div>
                  <div><div className="text-[11px] uppercase tracking-[0.18em] text-slate-400">{t("queue.eta")}</div><div>{terminal ? "--" : fmtExpectedTime(transfer.eta_seconds, locale)}</div></div>
                  <div><div className="text-[11px] uppercase tracking-[0.18em] text-slate-400">{t("queue.completion")}</div><div>{terminal ? fmtTimeStamp(transfer.updated_at, locale) : "--"}</div></div>
                  <div><div className="text-[11px] uppercase tracking-[0.18em] text-slate-400">{t("queue.speed")}</div><div>{fmtSpeed(transfer.speed_bps, locale)}</div></div>
                  <div><div className="text-[11px] uppercase tracking-[0.18em] text-slate-400">{t("queue.bytes")}</div><div>{fmtSize(transfer.bytes_done, locale)} / {fmtSize(transfer.bytes_total, locale)}</div></div>
                  <div><div className="text-[11px] uppercase tracking-[0.18em] text-slate-400">{t("queue.percentage")}</div><div>{percentage}%</div></div>
                </div>
              </article>
            );
          }) : (
            <div className="rounded-[24px] border border-dashed border-slate-300 bg-white/70 p-8 text-sm text-slate-500">
              {t("queue.none")}
            </div>
          )}
        </div>
      </section>
    </div>
  );
}
