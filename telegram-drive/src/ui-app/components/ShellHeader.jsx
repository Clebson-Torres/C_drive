import React, { useEffect, useState } from "react";

function SavedriveMark() {
  return (
    <span className="inline-flex h-11 w-11 items-center justify-center rounded-[18px] bg-gradient-to-br from-sky-500 via-cyan-400 to-blue-700 text-lg font-bold text-white shadow-lg shadow-sky-500/25">
      S
    </span>
  );
}

function avatarInitial(profile) {
  return (profile?.display_name || "S").trim().charAt(0).toUpperCase();
}

export default function ShellHeader({
  t,
  search,
  onSearchChange,
  onUpload,
  onUploadFolder,
  onNewFolder,
  profile,
  profileAvatarUrl,
  activeTransfers,
  queueOpen,
  onToggleQueue,
  userMenuOpen,
  onToggleUserMenu,
}) {
  const [avatarFailed, setAvatarFailed] = useState(false);

  useEffect(() => {
    setAvatarFailed(false);
  }, [profileAvatarUrl]);

  const showAvatarImage = Boolean(profileAvatarUrl) && !avatarFailed;

  return (
    <header className="relative z-30">
      <div className="grid gap-4 rounded-[28px] border border-white/70 bg-white/60 px-5 py-4 shadow-glass backdrop-blur-xl xl:grid-cols-[auto_auto_minmax(440px,1fr)_auto] xl:items-center">
        <div className="flex items-center gap-3">
          <SavedriveMark />
          <div className="min-w-0">
            <div className="text-[11px] font-semibold uppercase tracking-[0.28em] text-sky-700">Savedrive</div>
            <div className="truncate text-sm text-slate-500">{profile?.display_name || "Saved Messages shell"}</div>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <button id="btnUpload" className="primary-btn !px-4 !py-3" type="button" onClick={onUpload}>{t("header.upload")}</button>
          <button id="btnUploadFolder" className="ghost-btn !px-4 !py-3" type="button" onClick={onUploadFolder}>{t("header.uploadFolder")}</button>
          <button id="btnFolder" className="ghost-btn !px-4 !py-3" type="button" onClick={onNewFolder}>{t("header.newFolder")}</button>
        </div>

        <label className="block min-w-0">
          <span className="mb-2 block text-center text-xs font-semibold uppercase tracking-[0.26em] text-slate-500">
            {t("header.searchLabel")}
          </span>
          <input
            id="searchInput"
            value={search}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder={t("header.searchPlaceholder")}
            className="h-14 w-full rounded-2xl border border-slate-200 bg-white/90 px-5 text-base text-slate-900 outline-none transition focus:border-sky-400 focus:ring-4 focus:ring-sky-100"
          />
        </label>

        <div className="flex items-center justify-end gap-3">
          <button
            id="btnQueueToggle"
            type="button"
            onClick={onToggleQueue}
            className={`relative flex h-14 items-center gap-3 rounded-2xl border px-4 text-left shadow-sm transition ${
              queueOpen ? "border-sky-400 bg-sky-50 text-sky-900" : "border-slate-200 bg-white/85 text-slate-800 hover:bg-slate-100"
            }`}
          >
            <span className="text-sm font-semibold">{t("queue.title")}</span>
            {activeTransfers ? (
              <span
                id="queueBadge"
                className="inline-flex min-w-[1.75rem] items-center justify-center rounded-full bg-sky-600 px-2 py-1 text-xs font-semibold text-white"
              >
                {activeTransfers}
              </span>
            ) : null}
          </button>

          <div className="relative">
            <button
              id="btnUserMenu"
              type="button"
              onClick={onToggleUserMenu}
              className="flex min-w-[210px] items-center justify-between gap-3 rounded-[24px] border border-slate-200 bg-white/85 px-4 py-3 text-left shadow-sm"
            >
              <div className="flex items-center gap-3">
                {showAvatarImage ? (
                  <img
                    id="userAvatar"
                    src={profileAvatarUrl}
                    alt="Avatar"
                    onError={() => setAvatarFailed(true)}
                    className="h-11 w-11 rounded-full object-cover"
                  />
                ) : (
                  <div
                    id="userAvatarFallback"
                    className="flex h-11 w-11 items-center justify-center rounded-full bg-gradient-to-br from-sky-500 to-blue-700 text-base font-semibold text-white"
                  >
                    {avatarInitial(profile)}
                  </div>
                )}
                <div className="min-w-0">
                  <div id="userDisplayName" className="truncate text-base font-semibold text-slate-900">
                    {profile?.display_name || "Savedrive"}
                  </div>
                  <div className="truncate text-sm text-emerald-700">
                    {profile ? t("header.loggedIn") : t("header.loading")}
                  </div>
                </div>
              </div>
              <span className={`text-slate-400 transition ${userMenuOpen ? "rotate-180" : ""}`}>⌄</span>
            </button>
          </div>
        </div>
      </div>
    </header>
  );
}
