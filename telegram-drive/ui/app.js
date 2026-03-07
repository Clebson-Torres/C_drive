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
    selectedEntry: null,
    nativeDragActive: false,
  };

  const authScreen = document.getElementById("authScreen");
  const driveShell = document.getElementById("driveShell");
  const authMessage = document.getElementById("authMessage");
  const authFormStart = document.getElementById("authFormStart");
  const authFormCode = document.getElementById("authFormCode");
  const authFormPassword = document.getElementById("authFormPassword");
  const driveMessage = document.getElementById("driveMessage");
  const inputPhone = document.getElementById("inputPhone");
  const inputCode = document.getElementById("inputCode");
  const inputPassword = document.getElementById("inputPassword");
  const btnAuthStart = document.getElementById("btnAuthStart");
  const btnAuthCode = document.getElementById("btnAuthCode");
  const btnAuthPassword = document.getElementById("btnAuthPassword");
  const fileRows = document.getElementById("fileRows");
  const folderTree = document.getElementById("folderTree");
  const folderTiles = document.getElementById("folderTiles");
  const dropzoneCard = document.getElementById("dropzoneCard");
  const progressList = document.getElementById("progressList");
  const searchInput = document.getElementById("searchInput");
  const dropZone = document.getElementById("dropZone");
  const previewModal = document.getElementById("previewModal");
  const previewImage = document.getElementById("previewImage");
  const newFolderModal = document.getElementById("newFolderModal");
  const newFolderInput = document.getElementById("newFolderInput");
  const btnNewFolderConfirm = document.getElementById("btnNewFolderConfirm");
  const btnNewFolderCancel = document.getElementById("btnNewFolderCancel");
  const downloadModal = document.getElementById("downloadModal");
  const downloadTargetSummary = document.getElementById("downloadTargetSummary");
  const downloadCacheMode = document.getElementById("downloadCacheMode");
  const downloadCacheSummary = document.getElementById("downloadCacheSummary");
  const btnDownloadConfirm = document.getElementById("btnDownloadConfirm");
  const btnDownloadCancel = document.getElementById("btnDownloadCancel");
  const emptyState = document.getElementById("emptyState");
  const currentFolderLabel = document.getElementById("currentFolderLabel");
  const statFolderName = document.getElementById("statFolderName");
  const statEntryCount = document.getElementById("statEntryCount");
  const statTransferCount = document.getElementById("statTransferCount");
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
  const settingsDownloadCacheSummary = document.getElementById("settingsDownloadCacheSummary");
  const settingsDownloadCacheWriteSummary = document.getElementById("settingsDownloadCacheWriteSummary");
  const selectionDetails = document.getElementById("selectionDetails");

  const STATE_LABEL = {
    Queued: "Na fila",
    Running: "Ativo",
    Paused: "Pausado",
    Completed: "Concluído",
    Failed: "Erro",
    Cancelled: "Cancelado",
  };

  const PHASE_LABEL = {
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
  };

  const MODE_LABEL = {
    Single: "single",
    Chunked: "chunked",
  };

  const STATE_COLOR = {
    Queued: "#6e7fa5",
    Running: "#0a84ff",
    Paused: "#9a6b00",
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

  const setAuthMsg = (message, kind) => setMsg(authMessage, message, kind);
  const setDriveMsg = (message, kind) => setMsg(driveMessage, message, kind);

  function setLoading(btn, value) {
    if (btn) btn.disabled = value;
  }

  function debounce(fn, ms) {
    let handle;
    return (...args) => {
      clearTimeout(handle);
      handle = setTimeout(() => fn(...args), ms);
    };
  }

  function fmtSize(bytes) {
    const value = Number(bytes || 0);
    if (value < 1024) return `${value} B`;
    if (value < 1024 ** 2) return `${(value / 1024).toFixed(1)} KB`;
    if (value < 1024 ** 3) return `${(value / 1024 ** 2).toFixed(1)} MB`;
    return `${(value / 1024 ** 3).toFixed(2)} GB`;
  }

  function fmtSpeed(bps) {
    return bps ? `${fmtSize(bps)}/s` : "calculando";
  }

  function fmtTimeStamp(value) {
    if (!value) return "--";
    const parsed = new Date(value);
    if (Number.isNaN(parsed.getTime())) return "--";
    return parsed.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function fmtExpectedTime(seconds) {
    if (seconds === null || seconds === undefined) return "--";
    const parsed = new Date(Date.now() + Number(seconds) * 1000);
    if (Number.isNaN(parsed.getTime())) return "--";
    return parsed.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function normalizeChunkSize(bytes) {
    const value = Number(bytes);
    const allowed = [64, 128, 256].map((mib) => mib * 1024 * 1024);
    return allowed.includes(value) ? value : 128 * 1024 * 1024;
  }

  function chunkSizeLabel(bytes) {
    return `${Math.round(normalizeChunkSize(bytes) / 1024 / 1024)} MiB`;
  }

  function normalizeDownloadCacheThreshold(bytes) {
    const value = Number(bytes);
    return Number.isFinite(value) && value > 0 ? value : 2 * 1024 * 1024 * 1024;
  }

  function defaultDownloadCacheModeForSize(bytes) {
    const threshold = normalizeDownloadCacheThreshold(state.settings?.download_cache_threshold_bytes);
    return Number(bytes) > threshold ? "enabled" : "disabled";
  }

  function renderChunkSummary(bytes) {
    settingsChunkSummary.textContent =
      `Entre 2 GiB e 20 GiB, o app usa chunks de ${chunkSizeLabel(bytes)}. Acima disso, eleva para 256 MiB.`;
  }

  function renderSettingsCacheSummary() {
    const threshold = normalizeDownloadCacheThreshold(state.settings?.download_cache_threshold_bytes);
    settingsDownloadCacheSummary.textContent =
      `Downloads acima de ${fmtSize(threshold)} entram no cache automaticamente.`;
    settingsDownloadCacheWriteSummary.textContent = "Persistência de cache ocorre em background.";
  }

  function renderDownloadCacheSummary(fileSize, selectedMode) {
    const threshold = normalizeDownloadCacheThreshold(state.settings?.download_cache_threshold_bytes);
    const defaultMode = defaultDownloadCacheModeForSize(fileSize);

    if (selectedMode === "default") {
      const label = defaultMode === "enabled" ? "com cache" : "sem cache";
      downloadCacheSummary.textContent =
        `Padrão atual: ${label}. O cache automático entra apenas acima de ${fmtSize(threshold)}.`;
      return;
    }
    if (selectedMode === "enabled") {
      downloadCacheSummary.textContent =
        "O download será entregue primeiro e o cache será persistido em background.";
      return;
    }
    downloadCacheSummary.textContent = "Este download não preencherá o cache local nesta execução.";
  }

  function transferDirection(transfer) {
    return String(transfer?.job_id || "").startsWith("download-") ? "Download" : "Upload";
  }

  function isTerminal(stateValue) {
    return stateValue === "Completed" || stateValue === "Failed" || stateValue === "Cancelled";
  }

  function isActiveState(stateValue) {
    return stateValue === "Running" || stateValue === "Queued" || stateValue === "Paused";
  }

  function dedupeTransfersForDisplay(items) {
    const grouped = new Map();
    for (const transfer of items) {
      const key = `${transferDirection(transfer)}:${transfer.file_name}:${isActiveState(transfer.state) ? "active" : transfer.state}`;
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
    return Array.from(grouped.values());
  }

  function removeConflictingActiveDownloads(incoming) {
    if (transferDirection(incoming) !== "Download" || !isActiveState(incoming.state)) return;
    for (const [jobId, transfer] of state.transfers.entries()) {
      if (jobId === incoming.job_id) continue;
      if (transferDirection(transfer) !== "Download") continue;
      if (transfer.file_name !== incoming.file_name) continue;
      if (!isActiveState(transfer.state)) continue;
      state.transfers.delete(jobId);
    }
  }

  function renderShellMeta() {
    const currentFolder = state.folders.find((folder) => folder.id === state.currentFolderId);
    const entriesVisible = (state.currentListing.files?.length || 0) + (state.currentListing.folders?.length || 0);
    const activeTransfers = Array.from(state.transfers.values()).filter((item) => isActiveState(item.state)).length;

    statFolderName.textContent = currentFolder?.name || "Root";
    statEntryCount.textContent = String(entriesVisible);
    statTransferCount.textContent = String(activeTransfers);
    currentFolderLabel.textContent = currentFolder
      ? `Pasta atual: ${currentFolder.name}. Clique para selecionar e use duplo clique para abrir.`
      : "Selecione uma pasta para navegar e sincronizar.";
  }

  function renderSelectionDetails() {
    if (!selectionDetails) return;
    if (!state.selectedEntry) {
      selectionDetails.innerHTML = '<p class="selection-empty">Selecione uma pasta ou arquivo para ver detalhes.</p>';
      return;
    }

    const { kind, data } = state.selectedEntry;
    const rows = [];

    if (kind === "folder") {
      const childFolders = (state.currentListing.folders || []).filter((folder) => folder.parent_id === data.id).length;
      const childFiles = (state.currentListing.files || []).filter((file) => file.folder_id === data.id).length;
      rows.push(["Tipo", "Pasta"]);
      rows.push(["ID", String(data.id)]);
      rows.push(["Atualizada", new Date(data.updated_at).toLocaleString()]);
      rows.push(["Subpastas visíveis", String(childFolders)]);
      rows.push(["Arquivos visíveis", String(childFiles)]);
    } else {
      rows.push(["Tipo", data.mime_type || "application/octet-stream"]);
      rows.push(["Tamanho", fmtSize(data.size)]);
      rows.push(["Modo", MODE_LABEL[data.storage_mode] || data.storage_mode || "n/a"]);
      rows.push(["Hash", data.hash]);
      rows.push(["Atualizado", new Date(data.updated_at).toLocaleString()]);
    }

    selectionDetails.innerHTML = `
      <h4 class="selection-title">${data.name}</h4>
      <p class="selection-subtitle">${kind === "folder" ? "Seleção de pasta" : "Seleção de arquivo"}</p>
      <div class="selection-grid">
        ${rows
          .map(
            ([label, value]) => `
              <article class="selection-item">
                <span>${label}</span>
                <strong>${value}</strong>
              </article>`
          )
          .join("")}
      </div>
    `;
  }

  function selectEntry(kind, data) {
    state.selectedEntry = { kind, data };
    renderFolderTiles();
    renderFiles(state.currentListing);
    renderFolders();
    renderSelectionDetails();
  }

  async function openFolder(folder) {
    state.currentFolderId = folder.id;
    state.selectedEntry = { kind: "folder", data: folder };
    renderFolders();
    await loadListing();
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

  function openDownloadModal({ name, size, destinationPath }) {
    return new Promise((resolve) => {
      downloadTargetSummary.textContent = `${name} -> ${destinationPath}`;
      downloadCacheMode.value = "default";
      renderDownloadCacheSummary(size, downloadCacheMode.value);
      downloadModal.classList.remove("hidden");

      function cleanup() {
        downloadModal.classList.add("hidden");
        btnDownloadConfirm.removeEventListener("click", onConfirm);
        btnDownloadCancel.removeEventListener("click", onCancel);
        downloadCacheMode.removeEventListener("change", onChange);
      }

      function onConfirm() {
        const value = downloadCacheMode.value;
        cleanup();
        resolve(value);
      }

      function onCancel() {
        cleanup();
        resolve(null);
      }

      function onChange() {
        renderDownloadCacheSummary(size, downloadCacheMode.value);
      }

      btnDownloadConfirm.addEventListener("click", onConfirm);
      btnDownloadCancel.addEventListener("click", onCancel);
      downloadCacheMode.addEventListener("change", onChange);
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
    if (prefill?.phone) inputPhone.value = prefill.phone;
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
    const hidden = userMenu.classList.toggle("hidden");
    btnUserMenu.setAttribute("aria-expanded", String(!hidden));
  }

  function renderFolderTiles() {
    folderTiles.innerHTML = "";
    const folders = state.currentListing.folders || [];

    if (!folders.length) {
      folderTiles.innerHTML = `
        <article class="folder-tile-empty">
          <strong>No folders here</strong>
          <p>Crie uma nova pasta para organizar este workspace.</p>
        </article>
      `;
      return;
    }

    folders.forEach((folder) => {
      const selected =
        state.selectedEntry?.kind === "folder" && state.selectedEntry.data.id === folder.id;
      const tile = document.createElement("article");
      tile.className = `folder-tile${selected ? " selected" : ""}`;
      tile.setAttribute("data-folder-id", String(folder.id));
      tile.innerHTML = `
        <div class="folder-icon" aria-hidden="true"></div>
        <strong>${folder.name}</strong>
        <p>Um clique seleciona. Duplo clique abre a pasta.</p>
      `;
      tile.addEventListener("click", () => selectEntry("folder", folder));
      tile.addEventListener("dblclick", () => {
        openFolder(folder).catch((err) => setDriveMsg(errText(err)));
      });
      folderTiles.appendChild(tile);
    });
  }

  function renderFolders() {
    folderTree.innerHTML = "";
    state.folders.forEach((folder) => {
      const selected =
        state.selectedEntry?.kind === "folder" && state.selectedEntry.data.id === folder.id;
      const current = folder.id === state.currentFolderId;
      const li = document.createElement("li");
      if (selected || current) li.classList.add("active");

      const label = document.createElement("span");
      label.className = "folder-label";
      label.textContent = folder.name;
      label.addEventListener("click", () => selectEntry("folder", folder));
      label.addEventListener("dblclick", () => {
        openFolder(folder).catch((err) => setDriveMsg(errText(err)));
      });
      li.appendChild(label);

      if (folder.parent_id !== null && folder.parent_id !== undefined) {
        const del = document.createElement("button");
        del.type = "button";
        del.className = "action-btn del-btn";
        del.textContent = "Excluir";
        del.title = "Apagar pasta";
        del.addEventListener("click", async (event) => {
          event.stopPropagation();
          if (!confirm(`Apagar pasta "${folder.name}" e todo seu conteúdo?`)) return;
          try {
            await call("delete_folder", { folderId: folder.id, folder_id: folder.id });
            if (state.currentFolderId === folder.id) {
              state.currentFolderId =
                state.folders.find((item) => item.parent_id === null)?.id ?? null;
            }
            if (state.selectedEntry?.kind === "folder" && state.selectedEntry.data.id === folder.id) {
              state.selectedEntry = null;
            }
            await loadFolders();
            await loadListing();
            setDriveMsg(`Pasta "${folder.name}" apagada.`, "ok");
          } catch (err) {
            setDriveMsg(errText(err));
          }
        });
        li.appendChild(del);
      }

      folderTree.appendChild(li);
    });
  }

  function renderFiles(data = state.currentListing) {
    state.currentListing = data;
    fileRows.innerHTML = "";
    emptyState.classList.toggle("hidden", data.files.length > 0 || data.folders.length > 0);
    renderFolderTiles();

    data.files.forEach((file) => {
      const selected =
        state.selectedEntry?.kind === "file" && state.selectedEntry.data.id === file.id;
      const tr = document.createElement("tr");
      if (selected) tr.classList.add("file-row-selected");
      const isImage = file.mime_type?.startsWith("image/");
      tr.innerHTML = `
        <td>${isImage ? `<span class="file-link" data-preview="${file.id}">${file.name}</span>` : file.name}</td>
        <td>${fmtSize(file.size)}</td>
        <td>${file.mime_type}</td>
        <td>${new Date(file.created_at).toLocaleString()}</td>
        <td>
          <div class="action-stack">
            <button class="action-btn dl-btn" data-file-id="${file.id}" data-file-name="${file.name}" data-file-size="${file.size}" type="button" title="Baixar arquivo">Baixar</button>
            <button class="action-btn del-btn" data-file-id="${file.id}" data-file-name="${file.name}" type="button" title="Apagar arquivo">Excluir</button>
          </div>
        </td>
      `;
      tr.addEventListener("click", () => selectEntry("file", file));
      fileRows.appendChild(tr);
    });

    document.querySelectorAll("[data-preview]").forEach((el) => {
      el.addEventListener("click", async (event) => {
        event.stopPropagation();
        const id = Number(el.getAttribute("data-preview"));
        try {
          const preview = await call("preview_image", { fileId: id, file_id: id });
          previewImage.src = preview.local_path;
          previewModal.classList.remove("hidden");
        } catch (err) {
          setDriveMsg(errText(err));
        }
      });
    });

    document.querySelectorAll(".dl-btn").forEach((btn) => {
      btn.addEventListener("click", async (event) => {
        event.stopPropagation();
        const id = Number(btn.getAttribute("data-file-id"));
        const name = btn.getAttribute("data-file-name");
        const size = Number(btn.getAttribute("data-file-size") || 0);

        if (
          Array.from(state.transfers.values()).some(
            (item) =>
              transferDirection(item) === "Download" &&
              item.file_name === name &&
              isActiveState(item.state)
          )
        ) {
          setDriveMsg(`Já existe um download ativo para "${name}".`);
          return;
        }

        btn.disabled = true;
        try {
          const destDir = await call("pick_folder_native");
          if (!destDir) return;
          const destPath = destDir.replace(/[\\/]+$/, "") + "/" + name;
          const cacheMode = await openDownloadModal({ name, size, destinationPath: destPath });
          if (!cacheMode) return;
          setDriveMsg(`Baixando "${name}"...`, "ok");
          const result = await call("download_file", {
            fileId: id,
            file_id: id,
            destinationPath: destPath,
            destination_path: destPath,
            cacheMode: cacheMode,
            cache_mode: cacheMode,
          });
          setDriveMsg(result?.message || `Download de "${name}" concluído.`, "ok");
          await refreshTransferSnapshot();
        } catch (err) {
          setDriveMsg(errText(err));
        } finally {
          btn.disabled = false;
        }
      });
    });

    document.querySelectorAll(".del-btn").forEach((btn) => {
      btn.addEventListener("click", async (event) => {
        event.stopPropagation();
        const id = Number(btn.getAttribute("data-file-id"));
        const name = btn.getAttribute("data-file-name");
        if (!name) return;
        if (!confirm(`Apagar arquivo "${name}"?`)) return;
        try {
          await call("delete_file", { fileId: id, file_id: id });
          if (state.selectedEntry?.kind === "file" && state.selectedEntry.data.id === id) {
            state.selectedEntry = null;
          }
          await loadListing();
          setDriveMsg(`"${name}" apagado.`, "ok");
        } catch (err) {
          setDriveMsg(errText(err));
        }
      });
    });

    renderShellMeta();
    renderSelectionDetails();
  }

  function renderProgress() {
    progressList.innerHTML = "";
    const items = dedupeTransfersForDisplay(Array.from(state.transfers.values())).sort((a, b) => {
      const order = {
        Running: 0,
        Paused: 1,
        Queued: 2,
        Failed: 3,
        Completed: 4,
        Cancelled: 5,
      };
      return (order[a.state] ?? 9) - (order[b.state] ?? 9);
    });

    statTransferCount.textContent = String(
      items.filter((item) => isActiveState(item.state)).length
    );

    if (!items.length) {
      progressList.innerHTML = '<p class="empty-progress">Nenhuma transferência em andamento.</p>';
      return;
    }

    items.forEach((transfer) => {
      const pct = transfer.bytes_total > 0
        ? Math.min(100, (transfer.bytes_done / transfer.bytes_total) * 100)
        : 0;
      const color = STATE_COLOR[transfer.state] ?? "#6e7fa5";
      const label = STATE_LABEL[transfer.state] ?? transfer.state;
      const phase = PHASE_LABEL[transfer.phase] ?? transfer.phase ?? "Ativo";
      const mode = transfer.storage_mode
        ? MODE_LABEL[transfer.storage_mode] ?? transfer.storage_mode
        : "n/a";
      const direction = transferDirection(transfer);
      const isActive = isActiveState(transfer.state);
      const started = fmtTimeStamp(transfer.started_at);
      const expected = isTerminal(transfer.state)
        ? "--"
        : fmtExpectedTime(transfer.eta_seconds);
      const completed = isTerminal(transfer.state)
        ? fmtTimeStamp(transfer.updated_at)
        : "--";

      const item = document.createElement("article");
      item.className = "progress-item";
      item.innerHTML = `
        <div class="progress-meta">
          <span class="progress-filename" title="${transfer.file_name}">${transfer.file_name}</span>
          <div class="progress-control-row">
            <span class="progress-state" style="color:${color}">${label}</span>
            ${isActive ? `<button class="action-btn pause-btn" data-pause-job="${transfer.job_id}" type="button">${transfer.state === "Paused" ? "Continuar" : "Pausar"}</button>` : ""}
            ${isActive ? `<button class="action-btn del-btn" data-cancel-job="${transfer.job_id}" type="button">Cancelar</button>` : ""}
          </div>
        </div>
        <div class="progress-track">
          <div class="progress-value" style="width:${pct}%;background:${color}"></div>
        </div>
        <div class="progress-timings">
          <span class="time-chip">Início ${started}</span>
          <span class="time-chip">Previsto ${expected}</span>
          <span class="time-chip">Conclusão ${completed}</span>
        </div>
        <div class="progress-submeta">
          <span class="phase-chip">${phase}</span>
          <span class="mode-chip">${direction}</span>
          <span class="mode-chip">${mode}</span>
          <span>${fmtSize(transfer.bytes_done)} / ${fmtSize(transfer.bytes_total)}</span>
          <span>${fmtSpeed(transfer.speed_bps)}</span>
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
          await call("transfer_cancel", { jobId: jobId, job_id: jobId });
        } catch {
          // ignored
        }
      });
    });

    document.querySelectorAll("[data-pause-job]").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const jobId = btn.getAttribute("data-pause-job");
        const transfer = state.transfers.get(jobId);
        if (!transfer) return;
        try {
          if (transfer.state === "Paused") {
            await call("transfer_resume", { jobId: jobId, job_id: jobId });
          } else {
            await call("transfer_pause", { jobId: jobId, job_id: jobId });
          }
        } catch (err) {
          setDriveMsg(errText(err));
        }
      });
    });
  }

  function upsertTransfer(payload) {
    if (!payload?.job_id) return;
    removeConflictingActiveDownloads(payload);
    state.transfers.set(payload.job_id, payload);
    renderProgress();
    syncTransferPolling();
  }

  async function refreshTransferSnapshot() {
    try {
      const snapshot = await call("transfers_snapshot");
      const seen = new Set();
      if (snapshot?.length) {
        snapshot.forEach((item) => {
          seen.add(item.job_id);
          removeConflictingActiveDownloads(item);
          state.transfers.set(item.job_id, item);
        });
      }

      for (const [jobId, transfer] of state.transfers.entries()) {
        if (!seen.has(jobId) && !isTerminal(transfer.state)) {
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
    return Array.from(state.transfers.values()).some(
      (item) => item.state === "Running" || item.state === "Queued"
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
    if (!state.selectedEntry && state.currentFolderId) {
      const current = state.folders.find((folder) => folder.id === state.currentFolderId);
      if (current) state.selectedEntry = { kind: "folder", data: current };
    }
    renderFolders();
    renderSelectionDetails();
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
    const result = await call("search", {
      query: text,
      folderIdOpt: state.currentFolderId,
      folder_id_opt: state.currentFolderId,
      page: 0,
      pageSize: 100,
      page_size: 100,
    });
    renderFiles(result);
  }

  async function createFolder() {
    if (!state.currentFolderId) {
      setDriveMsg("Selecione uma pasta pai primeiro.");
      return;
    }
    const name = await openNewFolderModal();
    if (!name) return;
    try {
      const folder = await call("create_folder", {
        parentId: state.currentFolderId,
        parent_id: state.currentFolderId,
        name,
      });
      await loadFolders();
      await loadListing();
      selectEntry("folder", folder);
      setDriveMsg(`Pasta "${name}" criada.`, "ok");
    } catch (err) {
      setDriveMsg(errText(err));
    }
  }

  async function uploadPaths(paths) {
    if (!paths.length) return;
    if (!state.currentFolderId) {
      throw new Error("Selecione uma pasta antes de enviar arquivos.");
    }
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

  function setNativeDragActive(value) {
    state.nativeDragActive = value;
    dropZone.classList.toggle("drag-over", value);
    dropzoneCard?.classList.toggle("drag-over", value);
  }

  async function handleDroppedPaths(paths) {
    const validPaths = (paths || []).filter(Boolean);
    if (!validPaths.length) {
      setDriveMsg("Nenhum path local foi recebido no drop.");
      return;
    }
    await uploadPaths(validPaths);
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
    state.settings.download_cache_threshold_bytes = normalizeDownloadCacheThreshold(
      state.settings.download_cache_threshold_bytes
    );
    settingsChunkSize.value = String(state.settings.chunk_size_bytes);
    settingsParallelism.value = state.settings.max_parallelism;
    settingsEncrypt.checked = !!state.settings.encrypt_chunks;
    renderChunkSummary(state.settings.chunk_size_bytes);
    renderSettingsCacheSummary();
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
      download_cache_default_mode:
        state.settings?.download_cache_default_mode ?? "Threshold",
      download_cache_threshold_bytes: normalizeDownloadCacheThreshold(
        state.settings?.download_cache_threshold_bytes
      ),
      download_cache_write_mode:
        state.settings?.download_cache_write_mode ?? "Background",
    };
    await call("settings_set", { settings: next });
    state.settings = next;
    settingsModal.classList.add("hidden");
    renderChunkSummary(next.chunk_size_bytes);
    renderSettingsCacheSummary();
    setDriveMsg("Settings salvos.", "ok");
  }

  async function doLogout() {
    await call("auth_logout");
    state.authState = "LoggedOut";
    state.profile = null;
    state.transfers.clear();
    state.selectedEntry = null;
    closeUserMenu();
    renderProgress();
    renderProfile();
    renderAuthState();
    renderSelectionDetails();
    const prefill = await call("auth_prefill");
    applyAuthPrefill(prefill);
    setAuthMsg("Sessão encerrada.", "ok");
  }

  async function refreshAuthState() {
    state.authState = await call("auth_status");
    renderAuthState();
    return state.authState;
  }

  async function submitAuthStart(event) {
    event.preventDefault();
    setAuthMsg("");
    setLoading(btnAuthStart, true);
    try {
      const phone = inputPhone.value.trim();
      if (!phone) throw new Error("Preencha o telefone.");
      state.authState = await call("auth_start", { input: { phone } });
      renderAuthState();
      setAuthMsg("Código solicitado. Verifique o Telegram ou SMS.", "ok");
    } catch (err) {
      setAuthMsg(errText(err));
    } finally {
      setLoading(btnAuthStart, false);
    }
  }

  async function submitAuthCode(event) {
    event.preventDefault();
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
    } catch (err) {
      setAuthMsg(errText(err));
    } finally {
      setLoading(btnAuthCode, false);
    }
  }

  async function submitAuthPassword(event) {
    event.preventDefault();
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
    } catch (err) {
      setAuthMsg(errText(err));
    } finally {
      setLoading(btnAuthPassword, false);
    }
  }

  function bindEvents() {
    authFormStart.addEventListener("submit", submitAuthStart);
    authFormCode.addEventListener("submit", submitAuthCode);
    authFormPassword.addEventListener("submit", submitAuthPassword);

    document.getElementById("btnFolder").onclick = () =>
      createFolder().catch((err) => setDriveMsg(errText(err)));
    document.getElementById("btnUpload").onclick = () =>
      uploadFromNativeFilePicker().catch((err) => setDriveMsg(errText(err)));
    document.getElementById("btnUploadFolder").onclick = () =>
      uploadFromNativeFolderPicker().catch((err) => setDriveMsg(errText(err)));
    document.getElementById("closePreview").onclick = () =>
      previewModal.classList.add("hidden");
    btnDownloadCancel.onclick = () => downloadModal.classList.add("hidden");
    btnUserMenu.onclick = () => toggleUserMenu();
    btnOpenSettings.onclick = () => openSettings();
    btnSettingsClose.onclick = () => settingsModal.classList.add("hidden");
    btnSettingsSave.onclick = () =>
      saveSettings().catch((err) => setDriveMsg(errText(err)));
    btnLogout.onclick = () => doLogout().catch((err) => setDriveMsg(errText(err)));
    settingsChunkSize.onchange = () => renderChunkSummary(settingsChunkSize.value);

    searchInput.addEventListener(
      "input",
      debounce((event) => doSearch(event.target.value), 250)
    );

    document.addEventListener("click", (event) => {
      if (
        !userMenu.classList.contains("hidden") &&
        !event.target.closest(".user-menu-shell")
      ) {
        closeUserMenu();
      }
    });

    ["dragenter", "dragover"].forEach((name) => {
      dropZone.addEventListener(name, (event) => {
        event.preventDefault();
        event.stopPropagation();
        setNativeDragActive(true);
      });
    });

    ["dragleave", "drop"].forEach((name) => {
      dropZone.addEventListener(name, (event) => {
        event.preventDefault();
        event.stopPropagation();
        if (!state.nativeDragActive) setNativeDragActive(false);
      });
    });

    dropZone.addEventListener("drop", async (event) => {
      const paths = Array.from(event.dataTransfer?.files || [])
        .map((file) => file.path || "")
        .filter(Boolean);
      if (!paths.length) {
        setDriveMsg("Drop sem paths locais do WebView. Aguardando paths nativos do Tauri.");
        return;
      }
      setNativeDragActive(false);
      await handleDroppedPaths(paths).catch((err) => setDriveMsg(errText(err)));
    });

    if (listen) {
      const onTransfer = (evt) => {
        const payload = evt.payload;
        upsertTransfer(payload);
        if (payload.state === "Completed") {
          setDriveMsg(`"${payload.file_name}" concluído.`, "ok");
          loadListing().catch(() => {});
        } else if (payload.state === "Failed") {
          setDriveMsg(`Erro em "${payload.file_name}": ${payload.error || "erro desconhecido"}`);
        }
      };

      Promise.resolve(listen("transfer_progress", onTransfer)).catch(() => {});
      Promise.resolve(listen("transfer_state_changed", onTransfer)).catch(() => {});
      Promise.resolve(
        listen("download_cache_state_changed", (evt) => {
          const payload = evt.payload;
          if (payload?.state === "Completed") {
            setDriveMsg(`Cache de "${payload.file_name}" concluído.`, "ok");
          } else if (payload?.state === "Failed") {
            setDriveMsg(
              payload.message || `Falha ao persistir cache de "${payload.file_name}".`
            );
          }
        })
      ).catch(() => {});
      Promise.resolve(
        listen("auth_state_changed", async (evt) => {
          state.authState = evt.payload;
          renderAuthState();
          if (state.authState === "LoggedIn") {
            await bootstrapDrive();
          }
        })
      ).catch(() => {});
      Promise.resolve(listen("tauri://drag-enter", () => setNativeDragActive(true))).catch(
        () => {}
      );
      Promise.resolve(listen("tauri://drag-over", () => setNativeDragActive(true))).catch(
        () => {}
      );
      Promise.resolve(listen("tauri://drag-leave", () => setNativeDragActive(false))).catch(
        () => {}
      );
      Promise.resolve(
        listen("tauri://drag-drop", async (evt) => {
          setNativeDragActive(false);
          await handleDroppedPaths(evt.payload?.paths || []).catch((err) =>
            setDriveMsg(errText(err))
          );
        })
      ).catch(() => {});
    }

    window.addEventListener("__inject_transfer__", (event) => {
      Object.values(event.detail || {}).forEach((item) => {
        if (item?.job_id) {
          upsertTransfer(item);
        }
      });
    });
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
      renderSelectionDetails();
    }
  }

  init().catch((err) => {
    console.error(err);
    setAuthMsg(errText(err));
  });
})();
