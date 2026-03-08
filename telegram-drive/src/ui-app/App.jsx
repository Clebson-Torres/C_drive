import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import LoginScreen from "./components/LoginScreen";
import ShellHeader from "./components/ShellHeader";
import Explorer from "./components/Explorer";
import TransferQueue from "./components/TransferQueue";
import SettingsModal from "./components/SettingsModal";
import DownloadModal from "./components/DownloadModal";
import NewFolderModal from "./components/NewFolderModal";
import PreviewModal from "./components/PreviewModal";
import { call, debugLog, listenEvent, listenFileDropEvent, toAssetUrl } from "./lib/tauri";
import { defaultDownloadCacheModeForSize, errText } from "./lib/format";
import { createI18n, detectLocale } from "./lib/i18n";

const EMPTY_LISTING = { files: [], folders: [], total_files: 0, total_folders: 0 };
const DEFAULT_SETTINGS = {
  chunk_size_bytes: 128 * 1024 * 1024,
  max_parallelism: 16,
  encrypt_chunks: true,
  download_cache_default_mode: "Threshold",
  download_cache_threshold_bytes: 2 * 1024 * 1024 * 1024,
  download_cache_write_mode: "Background",
};

function useMessageState() {
  const [message, setMessage] = useState(null);
  const show = useCallback((text, kind = "error") => setMessage(text ? { text, kind } : null), []);
  return [message, show];
}

function detailsFromEntry(entry) {
  if (!entry) return null;
  return {
    name: entry.name,
    kind: entry.kind,
    size: entry.size,
    storageMode: typeof entry.storage_mode === "string" ? entry.storage_mode.toLowerCase() : entry.storage_mode,
    mimeType: entry.mime_type,
    updatedAt: entry.updated_at || entry.created_at,
    folderId: entry.folder_id,
  };
}

function extractDroppedPaths(payload) {
  if (Array.isArray(payload)) return payload.filter(Boolean);
  if (Array.isArray(payload?.paths)) return payload.paths.filter(Boolean);
  if (typeof payload?.path === "string") return [payload.path];
  if (typeof payload === "string") return [payload];
  return [];
}

function extractDomDroppedPaths(dataTransfer) {
  if (!dataTransfer) return [];

  const fromFiles = Array.from(dataTransfer.files || [])
    .map((file) => file?.path)
    .filter(Boolean);
  if (fromFiles.length) return fromFiles;

  const fromItems = Array.from(dataTransfer.items || [])
    .map((item) => item?.getAsFile?.())
    .map((file) => file?.path)
    .filter(Boolean);
  if (fromItems.length) return fromItems;

  return [];
}

export default function App() {
  const locale = useMemo(() => detectLocale(), []);
  const { t } = useMemo(() => createI18n(locale), [locale]);

  const [authState, setAuthState] = useState("LoggedOut");
  const [phone, setPhone] = useState("");
  const [code, setCode] = useState("");
  const [password, setPassword] = useState("");
  const [busyAction, setBusyAction] = useState(null);
  const [authMessage, setAuthMessage] = useMessageState();
  const [driveMessage, setDriveMessage] = useMessageState();
  const [folders, setFolders] = useState([]);
  const [currentFolderId, setCurrentFolderId] = useState(1);
  const [listing, setListing] = useState(EMPTY_LISTING);
  const [search, setSearch] = useState("");
  const [profile, setProfile] = useState(null);
  const [settings, setSettings] = useState(DEFAULT_SETTINGS);
  const [selectedEntry, setSelectedEntry] = useState(null);
  const [transfers, setTransfers] = useState(new Map());
  const [newFolderOpen, setNewFolderOpen] = useState(false);
  const [newFolderName, setNewFolderName] = useState("");
  const [downloadFile, setDownloadFile] = useState(null);
  const [downloadCacheMode, setDownloadCacheMode] = useState("default");
  const [previewPath, setPreviewPath] = useState(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [userMenuOpen, setUserMenuOpen] = useState(false);
  const [queueOpen, setQueueOpen] = useState(false);
  const [dragActive, setDragActive] = useState(false);
  const [profileAvatarBroken, setProfileAvatarBroken] = useState(false);
  const [pendingDownloadIds, setPendingDownloadIds] = useState(new Set());
  const dropSignatureRef = useRef("");
  const dropSignatureUntilRef = useRef(0);

  const activeTransfers = useMemo(
    () => [...transfers.values()].filter((transfer) => ["Running", "Queued", "Paused"].includes(transfer.state)).length,
    [transfers]
  );

  const selectionDetails = detailsFromEntry(selectedEntry);
  const profileAvatarUrl = useMemo(() => (profileAvatarBroken ? null : toAssetUrl(profile?.avatar_path_opt)), [profile?.avatar_path_opt, profileAvatarBroken]);
  const previewUrl = useMemo(() => toAssetUrl(previewPath), [previewPath]);

  useEffect(() => {
    setProfileAvatarBroken(false);
  }, [profile?.avatar_path_opt]);

  const upsertTransfer = useCallback((transfer) => {
    setTransfers((current) => {
      const next = new Map(current);
      next.set(transfer.job_id, transfer);
      return next;
    });
  }, []);

  const refreshListing = useCallback(async (folderId = currentFolderId, query = search) => {
    try {
      const data = query.trim()
        ? await call("search", { query, folderIdOpt: folderId, folder_id_opt: folderId, page: 0, pageSize: 100, page_size: 100 })
        : await call("list_folder", { folderId: folderId, folder_id: folderId, page: 0, pageSize: 100, page_size: 100 });
      setListing(data || EMPTY_LISTING);
      setSelectedEntry((current) => {
        if (!current) return null;
        const exists = [...(data?.folders || []), ...(data?.files || [])].some((entry) => entry.id === current.id);
        return exists ? current : null;
      });
    } catch (error) {
      setDriveMessage(errText(error));
    }
  }, [currentFolderId, search, setDriveMessage]);

  const refreshFolders = useCallback(async () => {
    try {
      const tree = await call("folder_tree");
      setFolders(tree || []);
      if (tree?.length && !tree.some((folder) => folder.id === currentFolderId)) {
        setCurrentFolderId(tree[0].id);
      }
      return tree || [];
    } catch (error) {
      setDriveMessage(errText(error));
      return [];
    }
  }, [currentFolderId, setDriveMessage]);

  const refreshProfile = useCallback(async () => {
    try {
      const data = await call("auth_profile");
      setProfile(data);
    } catch {
      setProfile(null);
    }
  }, []);

  const refreshSettings = useCallback(async () => {
    try {
      const data = await call("settings_get");
      setSettings({ ...DEFAULT_SETTINGS, ...data });
    } catch {
      setSettings(DEFAULT_SETTINGS);
    }
  }, []);

  const refreshTransfers = useCallback(async () => {
    try {
      const snapshot = await call("transfers_snapshot");
      const next = new Map();
      for (const transfer of snapshot || []) next.set(transfer.job_id, transfer);
      setTransfers(next);
    } catch {
      // no-op
    }
  }, []);

  const syncSavedMessages = useCallback(async () => {
    try {
      await call("sync_saved_messages_index");
    } catch (error) {
      setDriveMessage(t("misc.syncSavedMessagesFailed", { error: errText(error) }));
    }
  }, [setDriveMessage, t]);

  const hydrateAuthenticatedShell = useCallback(async (folderId = currentFolderId, query = search) => {
    await syncSavedMessages();
    const tree = await refreshFolders();
    const resolvedFolderId = tree.some((folder) => folder.id === folderId) ? folderId : (tree[0]?.id ?? folderId);
    if (resolvedFolderId !== currentFolderId) {
      setCurrentFolderId(resolvedFolderId);
    }
    await Promise.all([
      refreshListing(resolvedFolderId, query),
      refreshProfile(),
      refreshSettings(),
      refreshTransfers(),
    ]);
  }, [currentFolderId, refreshFolders, refreshListing, refreshProfile, refreshSettings, refreshTransfers, search, syncSavedMessages]);

  const bootstrap = useCallback(async () => {
    try {
      const prefill = await call("auth_prefill");
      if (prefill?.phone) {
        setPhone(prefill.phone);
      }
    } catch {}
    try {
      const state = await call("auth_status");
      setAuthState(state);
      if (state === "LoggedIn") {
        await hydrateAuthenticatedShell();
      }
    } catch (error) {
      setAuthMessage(t("misc.initializedFailed", { error: errText(error) }));
    }
  }, [hydrateAuthenticatedShell, setAuthMessage, t]);

  useEffect(() => {
    bootstrap();
  }, [bootstrap]);

  useEffect(() => {
    if (authState === "LoggedIn") {
      refreshListing(currentFolderId, search);
    }
  }, [authState, currentFolderId, refreshListing, search]);

  useEffect(() => {
    const timer = setTimeout(() => {
      if (authState === "LoggedIn") {
        refreshListing(currentFolderId, search);
      }
    }, 250);
    return () => clearTimeout(timer);
  }, [search, authState, currentFolderId, refreshListing]);

  const triggerUpload = useCallback(async (paths) => {
    if (!paths?.length) return;
    try {
      await debugLog("dragdrop", "trigger_upload", { currentFolderId, count: paths.length, sample: paths.slice(0, 3) });
      await call("upload_files", { folderId: currentFolderId, folder_id: currentFolderId, paths });
      await debugLog("dragdrop", "trigger_upload_ok", { count: paths.length });
      setDriveMessage(t("misc.uploadStarted", { count: paths.length }), "ok");
      await refreshTransfers();
    } catch (error) {
      await debugLog("dragdrop", "trigger_upload_error", { error: errText(error) });
      setDriveMessage(errText(error));
    }
  }, [currentFolderId, refreshTransfers, setDriveMessage, t]);

  const handleDroppedPaths = useCallback(async (paths) => {
    setDragActive(false);
    const normalized = (paths || []).filter(Boolean);
    if (!normalized.length) {
      debugLog("dragdrop", "handle_dropped_paths_empty");
      setDriveMessage(t("misc.dragDropNoPath"));
      return;
    }
    const signature = normalized.slice().sort().join("|");
    if (dropSignatureRef.current === signature && Date.now() < dropSignatureUntilRef.current) {
      debugLog("dragdrop", "handle_dropped_paths_deduped", { signature, count: normalized.length });
      return;
    }
    debugLog("dragdrop", "handle_dropped_paths", { signature, count: normalized.length, sample: normalized.slice(0, 3) });
    dropSignatureRef.current = signature;
    dropSignatureUntilRef.current = Date.now() + 1500;
    await triggerUpload(normalized);
  }, [setDriveMessage, t, triggerUpload]);

  const handleDomDragEnter = useCallback((event) => {
    event.preventDefault();
    setDragActive(true);
  }, []);

  const handleDomDragOver = useCallback((event) => {
    event.preventDefault();
    setDragActive(true);
  }, []);

  const handleDomDragLeave = useCallback((event) => {
    event.preventDefault();
    const currentTarget = event.currentTarget;
    const relatedTarget = event.relatedTarget;
    if (currentTarget && relatedTarget instanceof Node && currentTarget.contains(relatedTarget)) {
      return;
    }
    setDragActive(false);
  }, []);

  const handleDomDrop = useCallback(async (event) => {
    event.preventDefault();
    const paths = extractDomDroppedPaths(event.dataTransfer);
    await debugLog("dragdrop", "dom_drop", {
      fileCount: event.dataTransfer?.files?.length || 0,
      itemCount: event.dataTransfer?.items?.length || 0,
      extractedCount: paths.length,
      sample: paths.slice(0, 3),
    });
    if (!paths.length) {
      setDragActive(false);
      setDriveMessage(t("misc.dragDropNoPath"));
      return;
    }
    await handleDroppedPaths(paths);
  }, [handleDroppedPaths, setDriveMessage, t]);

  useEffect(() => {
    let mounted = true;
    const unsubs = [];
    (async () => {
      const handlers = {
        transfer_progress: ({ payload }) => mounted && upsertTransfer(payload),
        transfer_state_changed: ({ payload }) => mounted && upsertTransfer(payload),
        auth_state_changed: ({ payload }) => mounted && setAuthState(payload),
        index_changed: async () => mounted && refreshListing(currentFolderId, search),
        download_cache_state_changed: ({ payload }) => mounted && payload?.message && setDriveMessage(payload.message, payload.state === "failed" ? "error" : "ok"),
        "tauri://drag-enter": () => {
          if (!mounted) return;
          debugLog("dragdrop", "legacy_drag_enter");
          setDragActive(true);
        },
        "tauri://drag-over": () => {
          if (!mounted) return;
          debugLog("dragdrop", "legacy_drag_over");
          setDragActive(true);
        },
        "tauri://drag-leave": () => {
          if (!mounted) return;
          debugLog("dragdrop", "legacy_drag_leave");
          setDragActive(false);
        },
        "tauri://drag-drop": ({ payload }) => {
          if (!mounted) return;
          const paths = extractDroppedPaths(payload);
          debugLog("dragdrop", "legacy_drag_drop", { extractedCount: paths.length, sample: paths.slice(0, 3), payload });
          handleDroppedPaths(paths);
        },
      };
      for (const [name, handler] of Object.entries(handlers)) {
        unsubs.push(await listenEvent(name, handler));
      }
      const nativeUnlisten = await listenFileDropEvent((event) => {
        if (!mounted) return;
        const payload = event?.payload || {};
        const paths = extractDroppedPaths(payload.paths || payload);
        debugLog("dragdrop", "tauri_dragdrop_event", {
          type: payload.type,
          extractedCount: paths.length,
          sample: paths.slice(0, 3),
          payload,
        });
        switch (payload.type) {
          case "enter":
          case "over":
            setDragActive(true);
            break;
          case "leave":
            setDragActive(false);
            break;
          case "drop":
            handleDroppedPaths(paths);
            break;
          default:
            break;
        }
      });
      await debugLog("dragdrop", "native_dragdrop_listener_registered", { registered: typeof nativeUnlisten === "function" });
      unsubs.push(nativeUnlisten);
    })();

    const injectTransfer = (event) => {
      const entries = event.detail || {};
      Object.values(entries).forEach(upsertTransfer);
    };
    window.addEventListener("__inject_transfer__", injectTransfer);

    return () => {
      mounted = false;
      window.removeEventListener("__inject_transfer__", injectTransfer);
      unsubs.forEach((unsub) => typeof unsub === "function" && unsub());
    };
  }, [currentFolderId, handleDroppedPaths, refreshListing, search, setDriveMessage, upsertTransfer]);

  useEffect(() => {
    if (!activeTransfers) return;
    const interval = setInterval(() => {
      refreshTransfers();
    }, 1200);
    return () => clearInterval(interval);
  }, [activeTransfers, refreshTransfers]);

  useEffect(() => {
    const preventDefault = (event) => {
      event.preventDefault();
    };
    window.addEventListener("dragenter", preventDefault);
    window.addEventListener("dragover", preventDefault);
    window.addEventListener("dragleave", preventDefault);
    window.addEventListener("drop", preventDefault);
    return () => {
      window.removeEventListener("dragenter", preventDefault);
      window.removeEventListener("dragover", preventDefault);
      window.removeEventListener("dragleave", preventDefault);
      window.removeEventListener("drop", preventDefault);
    };
  }, []);

  const hasActiveDownloadForFile = useCallback((file) => {
    if (!file) return false;
    if (pendingDownloadIds.has(file.id)) return true;
    return [...transfers.values()].some((transfer) =>
      String(transfer.job_id || "").startsWith("download-")
      && ["Queued", "Running", "Paused"].includes(transfer.state)
      && transfer.file_name === file.name
    );
  }, [pendingDownloadIds, transfers]);

  const handleStartAuth = async () => {
    try {
      setBusyAction("start");
      setAuthMessage(null);
      const nextState = await call("auth_start", { input: { phone } });
      setAuthState(nextState);
    } catch (error) {
      setAuthMessage(errText(error));
    } finally {
      setBusyAction(null);
    }
  };

  const handleVerifyCode = async () => {
    try {
      setBusyAction("code");
      setAuthMessage(null);
      const nextState = await call("auth_verify_code", { code });
      setAuthState(nextState);
      if (nextState === "LoggedIn") {
        await hydrateAuthenticatedShell(currentFolderId, search);
      }
    } catch (error) {
      setAuthMessage(errText(error));
    } finally {
      setBusyAction(null);
    }
  };

  const handleVerifyPassword = async () => {
    try {
      setBusyAction("password");
      setAuthMessage(null);
      const nextState = await call("auth_verify_password", { password });
      setAuthState(nextState);
      if (nextState === "LoggedIn") {
        await hydrateAuthenticatedShell(currentFolderId, search);
      }
    } catch (error) {
      setAuthMessage(errText(error));
    } finally {
      setBusyAction(null);
    }
  };

  const handleLogout = async () => {
    try {
      await call("auth_logout");
      setAuthState("LoggedOut");
      setUserMenuOpen(false);
      setDriveMessage(null);
      setTransfers(new Map());
    } catch (error) {
      setDriveMessage(errText(error));
    }
  };

  const handleCreateFolder = async () => {
    try {
      const created = await call("create_folder", { parentId: currentFolderId, parent_id: currentFolderId, name: newFolderName });
      setNewFolderOpen(false);
      setNewFolderName("");
      setDriveMessage(t("newFolder.success", { name: created.name }), "ok");
      await Promise.all([refreshFolders(), refreshListing(currentFolderId, search)]);
    } catch (error) {
      setDriveMessage(errText(error));
    }
  };

  const handleUpload = async () => {
    try {
      const paths = await call("pick_files_native");
      await triggerUpload(paths);
    } catch (error) {
      setDriveMessage(errText(error));
    }
  };

  const handleUploadFolder = async () => {
    try {
      const directoryPath = await call("pick_folder_native");
      if (!directoryPath) return;
      await call("upload_folder", { folderId: currentFolderId, folder_id: currentFolderId, directoryPath, directory_path: directoryPath });
      setDriveMessage(t("misc.folderUploadStarted"), "ok");
      await refreshTransfers();
    } catch (error) {
      setDriveMessage(errText(error));
    }
  };

  const handlePreview = async (file) => {
    try {
      const preview = await call("preview_image", { fileId: file.id, file_id: file.id });
      setPreviewPath(preview.local_path);
    } catch (error) {
      setDriveMessage(errText(error));
    }
  };

  const handleDownloadConfirm = async () => {
    if (!downloadFile || hasActiveDownloadForFile(downloadFile)) return;
    setPendingDownloadIds((current) => new Set(current).add(downloadFile.id));
    setQueueOpen(true);
    try {
      const destinationPath = await call("pick_save_file_native", {
        suggestedName: downloadFile.name,
        suggested_name: downloadFile.name,
      });
      if (!destinationPath) return;
      const response = await call("download_file", {
        fileId: downloadFile.id,
        file_id: downloadFile.id,
        destinationPath,
        destination_path: destinationPath,
        cacheMode: downloadCacheMode,
        cache_mode: downloadCacheMode,
      });
      setDownloadFile(null);
      setDriveMessage(response.message || t("download.success"), "ok");
      await refreshTransfers();
    } catch (error) {
      setDriveMessage(errText(error));
    } finally {
      setPendingDownloadIds((current) => {
        const next = new Set(current);
        if (downloadFile?.id) next.delete(downloadFile.id);
        return next;
      });
    }
  };

  const handleSaveSettings = async () => {
    try {
      await call("settings_set", { settings });
      setSettingsOpen(false);
      setDriveMessage(t("settings.saved"), "ok");
    } catch (error) {
      setDriveMessage(errText(error));
    }
  };

  const updateTransferAction = async (command, jobId) => {
    try {
      await call(command, { jobId: jobId, job_id: jobId });
    } catch (error) {
      setDriveMessage(errText(error));
    }
  };

  const downloadSummary = useMemo(() => {
    if (!downloadFile) return "";
    const threshold = (settings.download_cache_threshold_bytes / 1024 / 1024 / 1024).toFixed(2);
    const defaultMode = defaultDownloadCacheModeForSize(downloadFile.size, settings.download_cache_threshold_bytes);
    if (downloadCacheMode === "default") {
      return defaultMode === "enabled"
        ? t("download.defaultEnabled", { threshold: `${threshold} GB` })
        : t("download.defaultDisabled", { threshold: `${threshold} GB` });
    }
    if (downloadCacheMode === "enabled") {
      return t("download.enabled");
    }
    return t("download.disabled");
  }, [downloadCacheMode, downloadFile, settings.download_cache_threshold_bytes, t]);

  useEffect(() => {
    if (downloadFile) {
      setDownloadCacheMode("default");
    }
  }, [downloadFile]);

  return (
    <div className="min-h-screen px-4 py-4 sm:px-6 sm:py-6">
      {authState === "LoggedIn" ? (
        <main id="driveShell" className="mx-auto flex min-h-screen max-w-[1880px] flex-col gap-6">
          <ShellHeader
            t={t}
            search={search}
            onSearchChange={setSearch}
            onUpload={handleUpload}
            onUploadFolder={handleUploadFolder}
            onNewFolder={() => setNewFolderOpen(true)}
            profile={profile}
            profileAvatarUrl={profileAvatarUrl}
            activeTransfers={activeTransfers}
            queueOpen={queueOpen}
            onToggleQueue={() => setQueueOpen((value) => !value)}
            userMenuOpen={userMenuOpen}
            onToggleUserMenu={() => setUserMenuOpen((value) => !value)}
          />

          {driveMessage ? <div id="driveMessage" className={`rounded-2xl border px-4 py-3 text-sm ${driveMessage.kind === "ok" ? "border-emerald-200 bg-emerald-50 text-emerald-700" : "border-rose-200 bg-rose-50 text-rose-700"}`}>{driveMessage.text}</div> : <div id="driveMessage" className="hidden" />}

          <Explorer
            t={t}
            locale={locale}
            folders={folders}
            listing={listing}
            selectedEntry={selectedEntry}
            details={selectionDetails}
            currentFolderId={currentFolderId}
            dragActive={dragActive}
            onFolderOpen={setCurrentFolderId}
            onEntrySelect={setSelectedEntry}
            onDragEnter={handleDomDragEnter}
            onDragOver={handleDomDragOver}
            onDragLeave={handleDomDragLeave}
            onDrop={handleDomDrop}
            isDownloadBusy={hasActiveDownloadForFile}
            onFileDownload={(file) => {
              if (hasActiveDownloadForFile(file)) {
                setQueueOpen(true);
                setDriveMessage(t("misc.downloadAlreadyQueued"), "ok");
                return;
              }
              setDownloadFile(file);
            }}
            onFilePreview={handlePreview}
          />

          <TransferQueue
            t={t}
            locale={locale}
            open={queueOpen}
            transfers={[...transfers.values()]}
            onClose={() => setQueueOpen(false)}
            onPause={(jobId) => updateTransferAction("transfer_pause", jobId)}
            onResume={(jobId) => updateTransferAction("transfer_resume", jobId)}
            onCancel={(jobId) => updateTransferAction("transfer_cancel", jobId)}
          />

          {userMenuOpen ? (
            <div id="userMenu" className="fixed right-6 top-28 z-[200] w-80 rounded-[28px] border border-white/70 bg-white/95 p-5 shadow-glass backdrop-blur-xl">
              <div className="flex items-center gap-4">
                {profileAvatarUrl ? (
                  <img id="userAvatarLarge" src={profileAvatarUrl} alt="Avatar" onError={() => setProfileAvatarBroken(true)} className="h-16 w-16 rounded-full object-cover" />
                ) : (
                  <div id="userAvatarLargeFallback" className="flex h-16 w-16 items-center justify-center rounded-full bg-gradient-to-br from-sky-500 to-blue-700 text-2xl font-semibold text-white">
                    {(profile?.display_name || "S").trim().charAt(0).toUpperCase()}
                  </div>
                )}
                <div>
                  <div id="userMenuName" className="text-lg font-semibold text-slate-950">{profile?.display_name || "Savedrive"}</div>
                  <div id="userMenuMeta" className="text-sm text-slate-600">{profile?.username ? `@${profile.username}` : profile?.phone_masked || t("misc.sessionActive")}</div>
                </div>
              </div>
              <div className="mt-5 grid gap-3">
                <button id="btnOpenSettings" type="button" className="ghost-btn w-full" onClick={() => { setSettingsOpen(true); setUserMenuOpen(false); }}>{t("header.settings")}</button>
                <button id="btnLogout" type="button" className="danger-btn w-full" onClick={handleLogout}>{t("header.logout")}</button>
              </div>
            </div>
          ) : null}
        </main>
      ) : (
        <LoginScreen
          t={t}
          authState={authState}
          message={authMessage}
          phone={phone}
          code={code}
          password={password}
          busyAction={busyAction}
          onPhoneChange={setPhone}
          onCodeChange={setCode}
          onPasswordChange={setPassword}
          onStart={handleStartAuth}
          onVerifyCode={handleVerifyCode}
          onVerifyPassword={handleVerifyPassword}
        />
      )}

      <NewFolderModal t={t} open={newFolderOpen} value={newFolderName} onChange={setNewFolderName} onConfirm={handleCreateFolder} onCancel={() => setNewFolderOpen(false)} />
      <DownloadModal t={t} locale={locale} open={Boolean(downloadFile)} file={downloadFile} settings={settings} cacheMode={downloadCacheMode} summary={downloadSummary} submitting={downloadFile ? pendingDownloadIds.has(downloadFile.id) : false} onChangeMode={setDownloadCacheMode} onConfirm={handleDownloadConfirm} onCancel={() => setDownloadFile(null)} />
      <SettingsModal t={t} locale={locale} open={settingsOpen} settings={settings} onChunkSizeChange={(value) => setSettings((current) => ({ ...current, chunk_size_bytes: value }))} onParallelismChange={(value) => setSettings((current) => ({ ...current, max_parallelism: value }))} onEncryptChange={(value) => setSettings((current) => ({ ...current, encrypt_chunks: value }))} onSave={handleSaveSettings} onClose={() => setSettingsOpen(false)} />
      <PreviewModal t={t} open={Boolean(previewUrl)} path={previewUrl} onClose={() => setPreviewPath(null)} />
    </div>
  );
}
