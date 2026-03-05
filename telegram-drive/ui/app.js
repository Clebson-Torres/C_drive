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

  function errText(err) {
    if (typeof err === "string") return err;
    if (err && typeof err.message === "string") return err.message;
    try {
      return JSON.stringify(err);
    } catch (_) {
      return String(err);
    }
  }

  async function call(cmd, args = {}) {
    if (!invoke) throw new Error("Tauri invoke API unavailable");
    const res = await invoke(cmd, args);
    if (!res.ok) throw new Error(res.error || "unknown backend error");
    return res.data;
  }

  function setAuthMessage(message, kind = "error") {
    if (!message) {
      authMessage.classList.add("hidden");
      authMessage.textContent = "";
      authMessage.classList.remove("error", "ok");
      return;
    }
    authMessage.textContent = message;
    authMessage.classList.remove("hidden", "error", "ok");
    authMessage.classList.add(kind);
  }

  function setDriveMessage(message, kind = "error") {
    if (!driveMessage) return;
    if (!message) {
      driveMessage.classList.add("hidden");
      driveMessage.textContent = "";
      driveMessage.classList.remove("error", "ok");
      return;
    }
    driveMessage.textContent = message;
    driveMessage.classList.remove("hidden", "error", "ok");
    driveMessage.classList.add(kind);
  }

  function setLoading(button, loading) {
    if (!button) return;
    button.disabled = loading;
  }

  function renderAuthState() {
    authFormStart.classList.toggle("hidden", state.authState !== "LoggedOut");
    authFormCode.classList.toggle("hidden", state.authState !== "AwaitingCode");
    authFormPassword.classList.toggle("hidden", state.authState !== "AwaitingPassword");

    const logged = state.authState === "LoggedIn";
    authScreen.classList.toggle("hidden", logged);
    driveShell.classList.toggle("hidden", !logged);
  }

  async function refreshAuthState() {
    state.authState = await call("auth_status");
    renderAuthState();
    return state.authState;
  }

  function fmtSize(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  function debounce(fn, ms) {
    let timer;
    return (...args) => {
      clearTimeout(timer);
      timer = setTimeout(() => fn(...args), ms);
    };
  }

  function renderFolders() {
    folderTree.innerHTML = "";
    state.folders.forEach((f) => {
      const li = document.createElement("li");
      li.textContent = f.name;
      if (f.id === state.currentFolderId) li.classList.add("active");
      li.onclick = async () => {
        state.currentFolderId = f.id;
        renderFolders();
        await loadListing();
      };
      folderTree.appendChild(li);
    });
  }

  function renderFiles(data) {
    fileRows.innerHTML = "";
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
      `;
      fileRows.appendChild(tr);
    });

    Array.from(document.querySelectorAll("[data-preview]")).forEach((el) => {
      el.addEventListener("click", async () => {
        const id = Number(el.getAttribute("data-preview"));
        const preview = await call("preview_image", { fileId: id, file_id: id });
        previewImage.src = preview.local_path;
        previewModal.classList.remove("hidden");
      });
    });
  }

  function renderProgress() {
    progressList.innerHTML = "";
    Array.from(state.transfers.values()).forEach((t) => {
      const item = document.createElement("div");
      item.className = "progress-item";
      const pct = t.bytes_total > 0 ? Math.min(100, (t.bytes_done / t.bytes_total) * 100) : 0;
      item.innerHTML = `
        <div class="progress-meta">
          <span>${t.file_name} (${t.state})</span>
          <span>${pct.toFixed(0)}%</span>
        </div>
        <div class="progress-track"><div class="progress-value" style="width:${pct}%"></div></div>
      `;
      progressList.appendChild(item);
    });
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
    const name = prompt("Folder name");
    if (!name) return;
    await call("create_folder", {
      parentId: state.currentFolderId,
      parent_id: state.currentFolderId,
      name,
    });
    await loadFolders();
    await loadListing();
    setDriveMessage(`Pasta "${name}" criada.`, "ok");
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
    await loadListing();
    setDriveMessage(`Upload iniciado para ${paths.length} arquivo(s).`, "ok");
  }

  async function uploadFromNativeFilePicker() {
    const paths = await call("pick_files_native");
    if (!paths.length) {
      setDriveMessage("Seleção cancelada.", "error");
      return;
    }
    await uploadPaths(paths);
  }

  async function uploadFromNativeFolderPicker() {
    if (!state.currentFolderId) {
      throw new Error("Selecione uma pasta de destino antes do upload.");
    }
    const selected = await call("pick_folder_native");
    if (!selected) {
      setDriveMessage("Seleção de pasta cancelada.", "error");
      return;
    }
    await call("upload_folder", {
      folderId: state.currentFolderId,
      folder_id: state.currentFolderId,
      directoryPath: selected,
      directory_path: selected,
    });
    await loadListing();
    setDriveMessage("Upload de pasta iniciado.", "ok");
  }

  async function submitAuthStart(evt) {
    evt.preventDefault();
    setAuthMessage("");
    setLoading(btnAuthStart, true);
    try {
      const phone = inputPhone.value.trim();
      const apiId = Number(inputApiId.value.trim());
      const apiHash = inputApiHash.value.trim();
      if (!phone || !Number.isInteger(apiId) || apiId <= 0 || !apiHash) {
        throw new Error("Preencha telefone, API ID e API hash corretamente.");
      }

      const nextState = await call("auth_start", {
        input: { phone, api_id: apiId, api_hash: apiHash },
      });
      state.authState = nextState;
      renderAuthState();
      setAuthMessage("Codigo enviado. Verifique Telegram/SMS.", "ok");
    } catch (err) {
      setAuthMessage(errText(err), "error");
    } finally {
      setLoading(btnAuthStart, false);
    }
  }

  async function submitAuthCode(evt) {
    evt.preventDefault();
    setAuthMessage("");
    setLoading(btnAuthCode, true);
    try {
      const code = inputCode.value.trim();
      if (!code) throw new Error("Informe o codigo recebido.");
      const nextState = await call("auth_verify_code", { code });
      state.authState = nextState;
      renderAuthState();
      if (nextState === "LoggedIn") {
        setAuthMessage("Login concluido.", "ok");
        await bootstrapDrive();
      } else {
        setAuthMessage("Conta com 2FA. Informe a senha.", "ok");
      }
    } catch (err) {
      setAuthMessage(errText(err), "error");
    } finally {
      setLoading(btnAuthCode, false);
    }
  }

  async function submitAuthPassword(evt) {
    evt.preventDefault();
    setAuthMessage("");
    setLoading(btnAuthPassword, true);
    try {
      const password = inputPassword.value;
      if (!password) throw new Error("Informe a senha 2FA.");
      const nextState = await call("auth_verify_password", { password });
      state.authState = nextState;
      renderAuthState();
      if (nextState === "LoggedIn") {
        setAuthMessage("Login concluido.", "ok");
        await bootstrapDrive();
      }
    } catch (err) {
      setAuthMessage(errText(err), "error");
    } finally {
      setLoading(btnAuthPassword, false);
    }
  }

  function bindEvents() {
    authFormStart.addEventListener("submit", submitAuthStart);
    authFormCode.addEventListener("submit", submitAuthCode);
    authFormPassword.addEventListener("submit", submitAuthPassword);

    document.getElementById("btnFolder").onclick = () =>
      createFolder().catch((e) => setDriveMessage(errText(e), "error"));
    document.getElementById("btnUpload").onclick = () =>
      uploadFromNativeFilePicker().catch((e) => setDriveMessage(errText(e), "error"));
    document.getElementById("btnUploadFolder").onclick = () =>
      uploadFromNativeFolderPicker().catch((e) => setDriveMessage(errText(e), "error"));

    document.getElementById("closePreview").onclick = () => previewModal.classList.add("hidden");
    searchInput.addEventListener("input", debounce((e) => doSearch(e.target.value), 250));

    ["dragenter", "dragover"].forEach((evt) => {
      dropZone.addEventListener(evt, (e) => {
        e.preventDefault();
        e.stopPropagation();
        dropZone.classList.add("drag-over");
      });
    });

    ["dragleave", "drop"].forEach((evt) => {
      dropZone.addEventListener(evt, (e) => {
        e.preventDefault();
        e.stopPropagation();
        dropZone.classList.remove("drag-over");
      });
    });

    dropZone.addEventListener("drop", async (e) => {
      try {
        const files = Array.from(e.dataTransfer?.files || []);
        const paths = files.map((f) => f.path || "").filter(Boolean);
        if (!paths.length) {
          throw new Error("Drop sem paths locais; use Upload nativo.");
        }
        await uploadPaths(paths);
      } catch (err) {
        setDriveMessage(errText(err), "error");
      }
    });

    if (listen) {
      listen("transfer_progress", (evt) => {
        const p = evt.payload;
        state.transfers.set(p.job_id, p);
        renderProgress();
      });

      listen("transfer_state_changed", (evt) => {
        const p = evt.payload;
        state.transfers.set(p.job_id, p);
        renderProgress();
      });
    }
  }

  async function bootstrapDrive() {
    setDriveMessage("");
    await loadFolders();
    await loadListing();
  }

  async function init() {
    bindEvents();
    await refreshAuthState();
    if (state.authState === "LoggedIn") {
      await bootstrapDrive();
    } else {
      renderAuthState();
    }
  }

  init().catch((err) => {
    console.error(err);
    setAuthMessage(errText(err), "error");
  });
})();
