import React, { useEffect, useState } from "react";

function SavedriveMark() {
  return (
    <span className="inline-flex h-12 w-12 items-center justify-center rounded-[18px] bg-gradient-to-br from-sky-500 via-cyan-400 to-blue-700 text-xl font-bold text-white shadow-lg shadow-sky-500/30">
      S
    </span>
  );
}

function avatarInitial(profile) {
  return (profile?.display_name || "S").trim().charAt(0).toUpperCase();
}

export default function ShellHeader({ t, search, onSearchChange, onUpload, onUploadFolder, onNewFolder, profile, profileAvatarUrl, currentFolderName, entryCount, activeTransfers, userMenuOpen, onToggleUserMenu, onOpenSettings, onLogout }) {
  const [avatarFailed, setAvatarFailed] = useState(false);

  useEffect(() => {
    setAvatarFailed(false);
  }, [profileAvatarUrl]);

  const showAvatarImage = Boolean(profileAvatarUrl) && !avatarFailed;

  return (
    <header className="relative z-20 space-y-5">
      <div className="grid gap-4 rounded-[32px] border border-white/70 bg-white/55 p-5 shadow-glass backdrop-blur-xl xl:grid-cols-[auto_minmax(420px,1fr)_auto] xl:items-center">
        <div className="flex flex-wrap items-center gap-3">
          <div className="mr-3 flex items-center gap-3 rounded-2xl bg-white/80 px-3 py-2 shadow-sm">
            <SavedriveMark />
            <div>
              <div className="text-[11px] font-semibold uppercase tracking-[0.28em] text-sky-700">Savedrive</div>
              <div className="text-sm text-slate-500">Saved Messages desktop shell</div>
            </div>
          </div>
          <button id="btnUpload" className="primary-btn" type="button" onClick={onUpload}>{t("header.upload")}</button>
          <button id="btnUploadFolder" className="ghost-btn" type="button" onClick={onUploadFolder}>{t("header.uploadFolder")}</button>
          <button id="btnFolder" className="ghost-btn" type="button" onClick={onNewFolder}>{t("header.newFolder")}</button>
        </div>
        <div className="mx-auto w-full max-w-3xl xl:px-6">
          <label className="block">
            <span className="mb-2 block text-center text-xs font-semibold uppercase tracking-[0.26em] text-slate-500">{t("header.searchLabel")}</span>
            <input
              id="searchInput"
              value={search}
              onChange={(event) => onSearchChange(event.target.value)}
              placeholder={t("header.searchPlaceholder")}
              className="h-14 w-full rounded-2xl border border-slate-200 bg-white/85 px-5 text-base text-slate-900 outline-none transition focus:border-sky-400 focus:ring-4 focus:ring-sky-100"
            />
          </label>
        </div>
        <div className="relative justify-self-end">
          <button id="btnUserMenu" type="button" onClick={onToggleUserMenu} className="flex min-w-[220px] items-center justify-between gap-3 rounded-[28px] border border-slate-200 bg-white/80 px-4 py-3 text-left shadow-sm">
            <div className="flex items-center gap-3">
              {showAvatarImage ? (
                <img id="userAvatar" src={profileAvatarUrl} alt="Avatar" onError={() => setAvatarFailed(true)} className="h-12 w-12 rounded-full object-cover" />
              ) : (
                <div id="userAvatarFallback" className="flex h-12 w-12 items-center justify-center rounded-full bg-gradient-to-br from-sky-500 to-blue-700 text-lg font-semibold text-white">{avatarInitial(profile)}</div>
              )}
              <div>
                <div id="userDisplayName" className="text-base font-semibold text-slate-900">{profile?.display_name || "Savedrive"}</div>
                <div className="text-sm text-emerald-700">{profile ? t("header.loggedIn") : t("header.loading")}</div>
              </div>
            </div>
            <span className="text-slate-500">?</span>
          </button>
        </div>
      </div>

      <div className="grid gap-3 rounded-[28px] border border-white/70 bg-white/45 p-4 shadow-glass backdrop-blur-xl md:grid-cols-3">
        <div>
          <div className="text-xs font-semibold uppercase tracking-[0.26em] text-sky-700">{t("header.workspace")}</div>
          <div id="currentFolderLabel" className="mt-2 text-2xl font-semibold text-slate-950">{currentFolderName}</div>
        </div>
        <div className="rounded-2xl border border-slate-200 bg-white/70 px-4 py-3"><div className="text-xs uppercase tracking-[0.24em] text-slate-500">{t("header.visibleEntries")}</div><div id="statEntryCount" className="mt-2 text-xl font-semibold text-slate-900">{entryCount}</div></div>
        <div className="rounded-2xl border border-slate-200 bg-white/70 px-4 py-3"><div className="text-xs uppercase tracking-[0.24em] text-slate-500">{t("header.activeTransfers")}</div><div id="statTransferCount" className="mt-2 text-xl font-semibold text-slate-900">{activeTransfers}</div></div>
      </div>
    </header>
  );
}
