(() => {
  const tauri = window.__TAURI__ || {};
  const tauriInternals = window.__TAURI_INTERNALS__ || {};
  const invoke = tauri.core?.invoke || tauri.invoke || tauriInternals.invoke;
  const listen = tauri.event?.listen || tauriInternals.event?.listen;

  const state = {
    authState: "LoggedOut",
    currentFolderId: null,
    folders: [],
    transfers: new Map(),
    page: 0,
    pageSize: 100,
    currentListing: { files: [], folders: [], total_files: 0, total_folders: 0 },
    transferPoller: null,
    profile: null,
    settings: null,
  };

  const authScreen = document.getElementById("authScreen");
  const driveShell = document.getElementById("driveShell");
  const authMessage = document.getElementById("authMessage");
  const authFormStart = document.getElementById("authFormStart");
  const authFormCode = document.getElementById("authFormCode");
  const authFormPassword = document.getElementById("authFormPassword");
  const driveMessage = document.getElementById("driveMessage");
  const inputPhone = document.getElementById("inputPhone");
  const inputApiId = document.getElementById("inputApiId");
  const inputApiHash = document.getElementById("inputApiHash");
  const inputCode = document.getElementById("inputCode");
  const inputPassword = document.getElementById("inputPassword");
  const btnAuthStart = document.getElementById("btnAuthStart");
  const btnAuthCode = document.getElementById("btnAuthCode");
  const btnAuthPassword = document.getElementById("btnAuthPassword");
  const fileRows = document.getElementById("fileRows");
  const folderTree = document.getElementById("folderTree");
  const progressList = document.getElementById("progressList");
  const searchInput = document.getElementById("searchInput");
  const dropZone = document.getElementById("dropZone");
  const previewModal = document.getElementById("previewModal");
  const previewImage = document.getElementById("previewImage");
  const newFolderModal = document.getElementById("newFolderModal");
  const newFolderInput = document.getElementById("newFolderInput");
  const btnNewFolderConfirm = document.getElementById("btnNewFolderConfirm");
  const btnNewFolderCancel = document.getElementById("btnNewFolderCancel");
  const emptyState = document.getElementById("emptyState");
  const currentFolderLabel = document.getElementById("currentFolderLabel");
  const statFolderName = document.getElementById("statFolderName");
  const statEntryCount = document.getElementById("statEntryCount");
  const statTransferCount = document.getElementById("statTransferCount");
  const btnApiHelp = document.getElementById("btnApiHelp");
  const apiHelpModal = document.getElementById("apiHelpModal");
  const btnApiHelpClose = document.getElementById("btnApiHelpClose");
  const btnUserMenu = document.getElementById("btnUserMenu");
  const userMenu = document.getElementById("userMenu");
  const userAvatar = document.getElementById("userAvatar");
  const userAvatarLarge = document.getElementById("userAvatarLarge");
  const userDisplayName = document.getElementById("userDisplayName");
  const userMenuName = document.getElementById("userMenuName");
  const userMenuMeta = document.getElementById("userMenuMeta");
  const btnOpenSettings = document.getElementById("btnOpenSettings");
  const btnLogout = document.getElementById("btnLogout");
  const settingsModal = document.getElementById("settingsModal");
  const btnSettingsClose = document.getElementById("btnSettingsClose");
  const btnSettingsSave = document.getElementById("btnSettingsSave");
  const settingsChunkSize = document.getElementById("settingsChunkSize");
  const settingsChunkSummary = document.getElementById("settingsChunkSummary");
  const settingsParallelism = document.getElementById("settingsParallelism");
  const settingsEncrypt = document.getElementById("settingsEncrypt");

  const STATE_LABEL = {
    Queued: "Na fila",
    Running: "Ativo",
    Completed: "Concluído",
    Failed: "Erro",
    Cancelled: "Cancelado",
  };
  const PHASE_LABEL = {
    Queued: "Queued",
    Hashing: "Hashing",
    Uploading: "Uploading",
    Downloading: "Downloading",
    Reassembling: "Reassembling",
    Completed: "Completed",
    Failed: "Failed",
    Cancelled: "Cancelled",
  };
  const MODE_LABEL = {
    Single: "single",
    Chunked: "chunked",
  };
  const STATE_COLOR = {
    Queued: "#6e7fa5",
    Running: "#0a84ff",
    Completed: "#128a65",
    Failed: "#d1335b",
    Cancelled: "#6e7fa5",
  };

  function errText(err) {
    if (typeof err === "string") return err;
    if (err?.message) return err.message;
    try {
      return JSON.stringify(err);
    } catch {
      return String(err);
    }
  }

  async function call(cmd, args = {}) {
    if (!invoke) throw new Error("Tauri invoke API unavailable");
    const res = await invoke(cmd, args);
    if (!res.ok) throw new Error(res.error || "unknown backend error");
    return res.data;
  }

  function setMsg(el, message, kind = "error") {
    if (!el) return;
    if (!message) {
      el.classList.add("hidden");
      el.textContent = "";
      el.classList.remove("error", "ok");
      return;
    }
    el.textContent = message;
    el.classList.remove("hidden", "error", "ok");
    el.classList.add(kind);
  }

  const setAuthMsg = (m, k) => setMsg(authMessage, m, k);
  const setDriveMsg = (m, k) => setMsg(driveMessage, m, k);

  function setLoading(btn, v) {
    if (btn) btn.disabled = v;
  }

  function debounce(fn, ms) {
    let t;
    return (...a) => {
      clearTimeout(t);
      t = setTimeout(() => fn(...a), ms);
    };
  }

  function fmtSize(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
    return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
  }

  function fmtSpeed(bps) {
    return bps ? `${fmtSize(bps)}/s` : "calculando";
  }

  function fmtEta(seconds) {
    if (seconds === null || seconds === undefined) return "ETA --";
    if (seconds < 60) return `ETA ${seconds}s`;
    if (seconds < 3600) return `ETA ${Math.ceil(seconds / 60)}m`;
    return `ETA ${(seconds / 3600).toFixed(1)}h`;
  }

  function normalizeChunkSize(bytes) {
    const value = Number(bytes);
    if (value === 8 * 1024 * 1024 || value === 32 * 1024 * 1024 || value === 64 * 1024 * 1024) {
      return value;
    }
    return 32 * 1024 * 1024;
  }

  function chunkSizeLabel(bytes) {
    const normalized = normalizeChunkSize(bytes);
    return `${Math.round(normalized / 1024 / 1024)} MiB`;
  }

  function renderChunkSummary(bytes) {
    settingsChunkSummary.textContent = `Acima de 2 GiB, o app usa chunks de ${chunkSizeLabel(bytes)}.`;
  }

  function renderShellMeta() {
    const currentFolder = state.folders.find((folder) => folder.id === state.currentFolderId);
    const entriesVisible = (state.currentListing.files?.length || 0) + (state.currentListing.folders?.length || 0);
    const activeTransfers = Array.from(state.transfers.values()).filter((item) =>
      item.state === "Running" || item.state === "Queued"
    ).length;

    statFolderName.textContent = currentFolder?.name || "Root";
    statEntryCount.textContent = String(entriesVisible);
    statTransferCount.textContent = String(activeTransfers);
    currentFolderLabel.textContent = currentFolder
      ? `Pasta atual: ${currentFolder.name}.`
      : "Selecione uma pasta para navegar e sincronizar.";
  }

  function openNewFolderModal() {
    return new Promise((resolve) => {
      newFolderInput.value = "";
      newFolderModal.classList.remove("hidden");
      newFolderInput.focus();

      function cleanup() {
        newFolderModal.classList.add("hidden");
        btnNewFolderConfirm.removeEventListener("click", onConfirm);
        btnNewFolderCancel.removeEventListener("click", onCancel);
        newFolderInput.removeEventListener("keydown", onKey);
      }

      function onConfirm() {
        const name = newFolderInput.value.trim();
        cleanup();
        resolve(name || null);
      }

      function onCancel() {
        cleanup();
        resolve(null);
      }

      function onKey(event) {
        if (event.key === "Enter") onConfirm();
        if (event.key === "Escape") onCancel();
      }

      btnNewFolderConfirm.addEventListener("click", onConfirm);
      btnNewFolderCancel.addEventListener("click", onCancel);
      newFolderInput.addEventListener("keydown", onKey);
    });
  }

  function renderAuthState() {
    authFormStart.classList.toggle("hidden", state.authState !== "LoggedOut");
    authFormCode.classList.toggle("hidden", state.authState !== "AwaitingCode");
    authFormPassword.classList.toggle("hidden", state.authState !== "AwaitingPassword");
    const logged = state.authState === "LoggedIn";
    authScreen.classList.toggle("hidden", logged);
    driveShell.classList.toggle("hidden", !logged);
  }

  function applyAuthPrefill(prefill) {
    if (!prefill) return;
    if (prefill.phone) inputPhone.value = prefill.phone;
    if (prefill.api_id) inputApiId.value = String(prefill.api_id);
    if (prefill.api_hash) inputApiHash.value = prefill.api_hash;
  }

  function setAvatar(el, profile) {
    const display = profile?.display_name || "Telegram";
    const initial = display.trim().charAt(0).toUpperCase() || "T";
    el.textContent = initial;
    el.style.backgroundImage = "";
    if (profile?.avatar_path_opt) {
      el.textContent = "";
      el.style.backgroundImage = `url("${encodeURI(profile.avatar_path_opt)}")`;
    }
  }

  function renderProfile() {
    const profile = state.profile || {
      display_name: "Telegram",
      username: null,
      phone_masked: null,
      avatar_path_opt: null,
    };
    userDisplayName.textContent = profile.display_name;
    userMenuName.textContent = profile.display_name;
    userMenuMeta.textContent = [profile.username ? `@${profile.username}` : null, profile.phone_masked]
      .filter(Boolean)
      .join(" · ") || "Conta Telegram";
    setAvatar(userAvatar, profile);
    setAvatar(userAvatarLarge, profile);
  }

  function closeUserMenu() {
    userMenu.classList.add("hidden");
    btnUserMenu.setAttribute("aria-expanded", "false");
  }

  function toggleUserMenu() {
    const open = userMenu.classList.toggle("hidden");
    btnUserMenu.setAttribute("aria-expanded", String(!open));
  }

  async function loadProfile() {
    try {
      state.profile = await call("auth_profile");
    } catch {
      state.profile = null;
    }
    renderProfile();
  }

  async function loadSettings() {
    state.settings = await call("settings_get");
    state.settings.chunk_size_bytes = normalizeChunkSize(state.settings.chunk_size_bytes);
    settingsChunkSize.value = String(state.settings.chunk_size_bytes);
    settingsParallelism.value = state.settings.max_parallelism;
    settingsEncrypt.checked = !!state.settings.encrypt_chunks;
    renderChunkSummary(state.settings.chunk_size_bytes);
  }

  function openSettings() {
    settingsModal.classList.remove("hidden");
    closeUserMenu();
  }

  async function saveSettings() {
    const next = {
      ...(state.settings || {}),
      chunk_size_bytes: normalizeChunkSize(settingsChunkSize.value),
      max_parallelism: Math.max(1, Math.min(48, Number(settingsParallelism.value || 16))),
      encrypt_chunks: settingsEncrypt.checked,
    };
    await call("settings_set", { settings: next });
    state.settings = next;
    settingsModal.classList.add("hidden");
    setDriveMsg("Settings salvos.", "ok");
  }

  async function doLogout() {
    await call("auth_logout");
    state.authState = "LoggedOut";
    state.profile = null;
    state.transfers.clear();
    closeUserMenu();
    renderProgress();
    renderProfile();
    renderAuthState();
    const prefill = await call("auth_prefill");
    applyAuthPrefill(prefill);
    setAuthMsg("Sessão encerrada.", "ok");
  }

  async function refreshAuthState() {
    state.authState = await call("auth_status");
    renderAuthState();
    return state.authState;
  }

  async function submitAuthStart(evt) {
    evt.preventDefault();
    setAuthMsg("");
    setLoading(btnAuthStart, true);
    try {
      const phone = inputPhone.value.trim();
      const apiId = Number(inputApiId.value.trim());
      const apiHash = inputApiHash.value.trim();
      if (!phone || !Number.isInteger(apiId) || apiId <= 0 || !apiHash) {
        throw new Error("Preencha telefone, API ID e API hash.");
      }
      state.authState = await call("auth_start", {
        input: { phone, api_id: apiId, api_hash: apiHash },
      });
      renderAuthState();
      setAuthMsg("Código solicitado. Verifique o Telegram ou SMS.", "ok");
    } catch (e) {
      setAuthMsg(errText(e));
    } finally {
      setLoading(btnAuthStart, false);
    }
  }

  async function submitAuthCode(evt) {
    evt.preventDefault();
    setAuthMsg("");
    setLoading(btnAuthCode, true);
    try {
      const code = inputCode.value.trim();
      if (!code) throw new Error("Informe o código recebido.");
      state.authState = await call("auth_verify_code", { code });
      renderAuthState();
      if (state.authState === "LoggedIn") {
        setAuthMsg("Login concluído.", "ok");
        await bootstrapDrive();
      } else {
        setAuthMsg("Conta com 2FA. Informe a senha.", "ok");
      }
    } catch (e) {
      setAuthMsg(errText(e));
    } finally {
      setLoading(btnAuthCode, false);
    }
  }

  async function submitAuthPassword(evt) {
    evt.preventDefault();
    setAuthMsg("");
    setLoading(btnAuthPassword, true);
    try {
      const password = inputPassword.value;
      if (!password) throw new Error("Informe a senha 2FA.");
      state.authState = await call("auth_verify_password", { password });
      renderAuthState();
      if (state.authState === "LoggedIn") {
        setAuthMsg("Login concluído.", "ok");
        await bootstrapDrive();
      }
    } catch (e) {
      setAuthMsg(errText(e));
    } finally {
      setLoading(btnAuthPassword, false);
    }
  }

  function renderFolders() {
    folderTree.innerHTML = "";
    state.folders.forEach((folder) => {
      const li = document.createElement("li");
      if (folder.id === state.currentFolderId) li.classList.add("active");

      const label = document.createElement("span");
      label.textContent = folder.name;
      label.className = "folder-label";
      label.onclick = async () => {
        state.currentFolderId = folder.id;
        renderFolders();
        await loadListing();
      };
      li.appendChild(label);

      if (folder.parent_id !== null && folder.parent_id !== undefined) {
        const del = document.createElement("button");
        del.textContent = "✕";
        del.className = "action-btn del-btn";
        del.title = "Apagar pasta";
        del.onclick = async (event) => {
          event.stopPropagation();
          if (!confirm(`Apagar pasta "${folder.name}" e todo seu conteúdo?`)) return;
          try {
            await call("delete_folder", { folderId: folder.id, folder_id: folder.id });
            if (state.currentFolderId === folder.id) {
              state.currentFolderId = state.folders.find((item) => !item.parent_id)?.id ?? null;
            }
            await loadFolders();
            await loadListing();
            setDriveMsg(`Pasta "${folder.name}" apagada.`, "ok");
          } catch (err) {
            setDriveMsg(errText(err));
          }
        };
        li.appendChild(del);
      }

      folderTree.appendChild(li);
    });
    renderShellMeta();
  }

  function renderFiles(data) {
    state.currentListing = data;
    fileRows.innerHTML = "";
    emptyState.classList.toggle("hidden", data.files.length > 0);

    data.files.forEach((file) => {
      const tr = document.createElement("tr");
      const isImage = file.mime_type?.startsWith("image/");
      const nameCell = isImage
        ? `<span class="file-link" data-preview="${file.id}">${file.name}</span>`
        : file.name;
      tr.innerHTML = `
        <td>${nameCell}</td>
        <td>${fmtSize(file.size)}</td>
        <td>${file.mime_type}</td>
        <td>${new Date(file.created_at).toLocaleString()}</td>
        <td>
          <div class="action-stack">
            <button class="action-btn dl-btn" data-file-id="${file.id}" data-file-name="${file.name}" title="Baixar arquivo">⬇</button>
            <button class="action-btn del-btn" data-file-id="${file.id}" data-file-name="${file.name}" title="Apagar arquivo">✕</button>
          </div>
        </td>
      `;
      fileRows.appendChild(tr);
    });

    document.querySelectorAll("[data-preview]").forEach((el) => {
      el.addEventListener("click", async () => {
        const id = Number(el.getAttribute("data-preview"));
        try {
          const preview = await call("preview_image", { fileId: id, file_id: id });
          previewImage.src = preview.local_path;
          previewModal.classList.remove("hidden");
        } catch (e) {
          setDriveMsg(errText(e));
        }
      });
    });

    document.querySelectorAll(".dl-btn").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const id = Number(btn.getAttribute("data-file-id"));
        const name = btn.getAttribute("data-file-name");
        try {
          const destDir = await call("pick_folder_native");
          if (!destDir) return;
          const destPath = destDir.replace(/[\\/]+$/, "") + "/" + name;
          setDriveMsg(`Baixando "${name}"...`, "ok");
          await call("download_file", {
            fileId: id,
            file_id: id,
            destinationPath: destPath,
            destination_path: destPath,
          });
          await refreshTransferSnapshot();
        } catch (e) {
          setDriveMsg(errText(e));
        }
      });
    });

    document.querySelectorAll(".del-btn").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const id = Number(btn.getAttribute("data-file-id"));
        const name = btn.getAttribute("data-file-name");
        if (!name) return;
        if (!confirm(`Apagar arquivo "${name}"?`)) return;
        try {
          await call("delete_file", { fileId: id, file_id: id });
          await loadListing();
          setDriveMsg(`"${name}" apagado.`, "ok");
        } catch (err) {
          setDriveMsg(errText(err));
        }
      });
    });

    renderShellMeta();
  }

  function renderProgress() {
    progressList.innerHTML = "";
    const items = Array.from(state.transfers.values()).sort((a, b) => {
      const order = { Running: 0, Queued: 1, Failed: 2, Completed: 3, Cancelled: 4 };
      return (order[a.state] ?? 9) - (order[b.state] ?? 9);
    });
    statTransferCount.textContent = String(
      items.filter((item) => item.state === "Running" || item.state === "Queued").length
    );

    if (items.length === 0) {
      progressList.innerHTML = '<p class="empty-progress">Nenhuma transferência em andamento.</p>';
      return;
    }

    items.forEach((transfer) => {
      const pct = transfer.bytes_total > 0
        ? Math.min(100, (transfer.bytes_done / transfer.bytes_total) * 100)
        : 0;
      const color = STATE_COLOR[transfer.state] ?? "#6e7fa5";
      const label = STATE_LABEL[transfer.state] ?? transfer.state;
      const phase = PHASE_LABEL[transfer.phase] ?? transfer.phase ?? "Running";
      const mode = transfer.storage_mode ? (MODE_LABEL[transfer.storage_mode] ?? transfer.storage_mode) : "n/a";
      const isActive = transfer.state === "Running" || transfer.state === "Queued";

      const item = document.createElement("article");
      item.className = "progress-item";
      item.innerHTML = `
        <div class="progress-meta">
          <span class="progress-filename" title="${transfer.file_name}">${transfer.file_name}</span>
          <span class="progress-state" style="color:${color}">${label}</span>
          ${isActive ? `<button class="action-btn del-btn" data-cancel-job="${transfer.job_id}">✕</button>` : ""}
        </div>
        <div class="progress-track">
          <div class="progress-value" style="width:${pct}%;background:${color}"></div>
        </div>
        <div class="progress-submeta">
          <span class="phase-chip">${phase}</span>
          <span class="mode-chip">${mode}</span>
          <span>${fmtSize(transfer.bytes_done)} / ${fmtSize(transfer.bytes_total)}</span>
          <span>${fmtSpeed(transfer.speed_bps)}</span>
          <span>${fmtEta(transfer.eta_seconds)}</span>
          <span>${pct.toFixed(0)}%</span>
        </div>
        ${transfer.error ? `<div class="progress-error">${transfer.error}</div>` : ""}
      `;
      progressList.appendChild(item);
    });

    document.querySelectorAll("[data-cancel-job]").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const jobId = btn.getAttribute("data-cancel-job");
        try {
          await call("transfer_cancel", { jobId, job_id: jobId });
        } catch {
          // ignore
        }
      });
    });
  }

  async function refreshTransferSnapshot() {
    try {
      const snap = await call("transfers_snapshot");
      const seen = new Set();
      if (snap?.length) {
        snap.forEach((item) => {
          seen.add(item.job_id);
          state.transfers.set(item.job_id, item);
        });
      }
      for (const [jobId, transfer] of state.transfers.entries()) {
        const terminal = transfer.state === "Completed" || transfer.state === "Failed" || transfer.state === "Cancelled";
        if (!seen.has(jobId) && !terminal) {
          state.transfers.delete(jobId);
        }
      }
      renderProgress();
      syncTransferPolling();
    } catch {
      // non critical
    }
  }

  function hasActiveTransfers() {
    return Array.from(state.transfers.values()).some((item) =>
      item.state === "Running" || item.state === "Queued"
    );
  }

  function syncTransferPolling() {
    if (hasActiveTransfers()) {
      if (!state.transferPoller) {
        state.transferPoller = window.setInterval(() => {
          refreshTransferSnapshot().catch(() => {});
        }, 1200);
      }
      return;
    }

    if (state.transferPoller) {
      window.clearInterval(state.transferPoller);
      state.transferPoller = null;
    }
  }

  async function loadFolders() {
    state.folders = await call("folder_tree");
    if (!state.currentFolderId && state.folders.length) {
      state.currentFolderId = state.folders[0].id;
    }
    renderFolders();
  }

  async function loadListing() {
    if (!state.currentFolderId) return;
    const data = await call("list_folder", {
      folderId: state.currentFolderId,
      folder_id: state.currentFolderId,
      page: state.page,
      pageSize: state.pageSize,
      page_size: state.pageSize,
      sort: null,
      direction: null,
    });
    renderFiles(data);
  }

  async function doSearch(text) {
    if (!text.trim()) {
      await loadListing();
      return;
    }
    const res = await call("search", {
      query: text,
      folderIdOpt: state.currentFolderId,
      folder_id_opt: state.currentFolderId,
      page: 0,
      pageSize: 100,
      page_size: 100,
    });
    renderFiles(res);
  }

  async function createFolder() {
    if (!state.currentFolderId) {
      setDriveMsg("Selecione uma pasta pai primeiro.");
      return;
    }
    const name = await openNewFolderModal();
    if (!name) return;
    try {
      await call("create_folder", {
        parentId: state.currentFolderId,
        parent_id: state.currentFolderId,
        name,
      });
      await loadFolders();
      await loadListing();
      setDriveMsg(`Pasta "${name}" criada.`, "ok");
    } catch (e) {
      setDriveMsg(errText(e));
    }
  }

  async function uploadPaths(paths) {
    if (!paths.length) return;
    if (!state.currentFolderId) throw new Error("Selecione uma pasta antes de enviar arquivos.");
    await call("upload_files", {
      folderId: state.currentFolderId,
      folder_id: state.currentFolderId,
      paths,
    });
    setDriveMsg(`Upload iniciado para ${paths.length} arquivo(s).`, "ok");
    await refreshTransferSnapshot();
  }

  async function uploadFromNativeFilePicker() {
    const paths = await call("pick_files_native");
    if (!paths?.length) {
      setDriveMsg("Seleção cancelada.");
      return;
    }
    await uploadPaths(paths);
  }

  async function uploadFromNativeFolderPicker() {
    if (!state.currentFolderId) throw new Error("Selecione uma pasta de destino.");
    const selected = await call("pick_folder_native");
    if (!selected) {
      setDriveMsg("Seleção cancelada.");
      return;
    }
    await call("upload_folder", {
      folderId: state.currentFolderId,
      folder_id: state.currentFolderId,
      directoryPath: selected,
      directory_path: selected,
    });
    setDriveMsg("Upload de pasta iniciado.", "ok");
    await refreshTransferSnapshot();
  }

  function bindEvents() {
    authFormStart.addEventListener("submit", submitAuthStart);
    authFormCode.addEventListener("submit", submitAuthCode);
    authFormPassword.addEventListener("submit", submitAuthPassword);

    document.getElementById("btnFolder").onclick = () => createFolder().catch((e) => setDriveMsg(errText(e)));
    document.getElementById("btnUpload").onclick = () => uploadFromNativeFilePicker().catch((e) => setDriveMsg(errText(e)));
    document.getElementById("btnUploadFolder").onclick = () =>
      uploadFromNativeFolderPicker().catch((e) => setDriveMsg(errText(e)));
    document.getElementById("closePreview").onclick = () => previewModal.classList.add("hidden");
    btnApiHelp.onclick = () => apiHelpModal.classList.remove("hidden");
    btnApiHelpClose.onclick = () => apiHelpModal.classList.add("hidden");
    btnUserMenu.onclick = () => toggleUserMenu();
    btnOpenSettings.onclick = () => openSettings();
    btnSettingsClose.onclick = () => settingsModal.classList.add("hidden");
    btnSettingsSave.onclick = () => saveSettings().catch((e) => setDriveMsg(errText(e)));
    btnLogout.onclick = () => doLogout().catch((e) => setDriveMsg(errText(e)));
    settingsChunkSize.onchange = () => renderChunkSummary(settingsChunkSize.value);

    searchInput.addEventListener("input", debounce((event) => doSearch(event.target.value), 250));
    document.addEventListener("click", (event) => {
      if (!userMenu.classList.contains("hidden") && !event.target.closest(".user-menu-shell")) {
        closeUserMenu();
      }
    });

    ["dragenter", "dragover"].forEach((ev) =>
      dropZone.addEventListener(ev, (event) => {
        event.preventDefault();
        event.stopPropagation();
        dropZone.classList.add("drag-over");
      })
    );
    ["dragleave", "drop"].forEach((ev) =>
      dropZone.addEventListener(ev, (event) => {
        event.preventDefault();
        event.stopPropagation();
        dropZone.classList.remove("drag-over");
      })
    );
    dropZone.addEventListener("drop", async (event) => {
      const paths = Array.from(event.dataTransfer?.files || [])
        .map((file) => file.path || "")
        .filter(Boolean);
      if (!paths.length) {
        setDriveMsg("Drop sem paths locais; use Upload nativo.");
        return;
      }
      await uploadPaths(paths).catch((err) => setDriveMsg(errText(err)));
    });

    if (listen) {
      const onTransfer = (evt) => {
        const payload = evt.payload;
        state.transfers.set(payload.job_id, payload);
        renderProgress();
        syncTransferPolling();
        if (payload.state === "Completed") {
          setDriveMsg(`"${payload.file_name}" concluído.`, "ok");
          loadListing().catch(() => {});
        } else if (payload.state === "Failed") {
          setDriveMsg(`Erro em "${payload.file_name}": ${payload.error || "erro desconhecido"}`);
        }
      };
      Promise.resolve(listen("transfer_progress", onTransfer)).catch(() => {});
      Promise.resolve(listen("transfer_state_changed", onTransfer)).catch(() => {});
      Promise.resolve(listen("auth_state_changed", async (evt) => {
        state.authState = evt.payload;
        renderAuthState();
        if (state.authState === "LoggedIn") {
          await bootstrapDrive();
        }
      })).catch(() => {});
    }
  }

  async function bootstrapDrive() {
    setDriveMsg("");
    await loadProfile();
    await loadSettings();
    await loadFolders();
    await loadListing();
    await refreshTransferSnapshot();
  }

  async function init() {
    bindEvents();
    const prefill = await call("auth_prefill").catch(() => null);
    applyAuthPrefill(prefill);
    await refreshAuthState();
    if (state.authState === "LoggedIn") {
      await bootstrapDrive();
    } else {
      renderAuthState();
      renderProfile();
    }
  }

  init().catch((err) => {
    console.error(err);
    setAuthMsg(errText(err));
  });
})();
