import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__TEST_LOCALE__ = "pt-BR";
    let authState = "LoggedOut";
    const folders = [{ id: 1, name: "Saved Messages", parent_id: null, created_at: new Date().toISOString(), updated_at: new Date().toISOString() }];
    const calls: Array<{ cmd: string; args: any }> = [];
    const listeners = new Map<string, Array<(event: any) => void>>();
    (window as any).__TEST_CALLS__ = calls;
    (window as any).__emitTauriEvent = (name: string, payload: any) => {
      for (const cb of listeners.get(name) || []) cb({ payload });
    };
    const emit = (name: string, payload: any) => {
      for (const cb of listeners.get(name) || []) cb({ payload });
    };

    const ok = (data: any) => ({ ok: true, data, error: null });
    const err = (message: string) => ({ ok: false, data: null, error: message });

    (window as any).__TAURI__ = {
      core: {
        invoke: async (cmd: string, args: any) => {
          calls.push({ cmd, args });
          switch (cmd) {
            case "auth_status":
              return ok(authState);
            case "auth_prefill":
              return ok({ phone: "+551100000000" });
            case "auth_start":
              if (!args?.input?.phone) {
                return err("invalid auth input");
              }
              authState = "AwaitingCode";
              return ok(authState);
            case "auth_verify_code":
              if (args?.code === "00000") {
                authState = "AwaitingPassword";
                return ok(authState);
              }
              if (args?.code === "12345") {
                authState = "LoggedIn";
                return ok(authState);
              }
              return err("invalid login code");
            case "auth_verify_password":
              if (args?.password === "password123") {
                authState = "LoggedIn";
                return ok(authState);
              }
              return err("invalid 2FA password");
            case "auth_profile":
              return ok({
                display_name: "Telegram User",
                username: "telegram_user",
                phone_masked: "***0000",
                avatar_path_opt: null,
              });
            case "auth_logout":
              authState = "LoggedOut";
              return ok(authState);
            case "folder_tree":
              return ok(folders);
            case "sync_saved_messages_index":
              return ok(3);
            case "list_folder":
              return ok({
                folders: [],
                files: [{
                  id: 88,
                  name: "archive.bin",
                  size: 3 * 1024 * 1024 * 1024,
                  hash: "abc123",
                  folder_id: 1,
                  mime_type: "application/octet-stream",
                  created_at: new Date().toISOString(),
                  updated_at: new Date().toISOString(),
                  original_path: null,
                  storage_mode: "Chunked",
                  telegram_file_id: null,
                  origin: "Savedrive",
                }],
                total_folders: 0,
                total_files: 1,
              });
            case "search":
              return ok({ folders: [], files: [], total_folders: 0, total_files: 0 });
            case "create_folder":
              if (!Object.prototype.hasOwnProperty.call(args || {}, "parentId")) {
                return err("missing parentId");
              }
              if (!args?.name) {
                return err("missing folder name");
              }
              return ok({
                id: 2,
                name: args.name,
                parent_id: args.parentId ?? null,
                created_at: new Date().toISOString(),
                updated_at: new Date().toISOString(),
              });
            case "pick_files_native":
              return ok(["C:/tmp/file-1.txt", "C:/tmp/file-2.jpg"]);
            case "upload_files":
              if (!Object.prototype.hasOwnProperty.call(args || {}, "folderId")) {
                return err("missing folderId");
              }
              if (!Array.isArray(args?.paths) || args.paths.length === 0) {
                return err("missing paths");
              }
              return ok([101, 102]);
            case "pick_folder_native":
              return ok("C:/tmp/folder-A");
            case "pick_save_file_native":
              return ok("C:/tmp/downloads/archive.bin");
            case "upload_folder":
              if (!Object.prototype.hasOwnProperty.call(args || {}, "folderId")) {
                return err("missing folderId");
              }
              if (!args?.directoryPath) {
                return err("missing directoryPath");
              }
              return ok([501, 502, 503]);
            case "preview_image":
              return ok({ local_path: "C:/tmp/preview.png", mime_type: "image/png" });
            case "download_file":
              emit("transfer_progress", {
                job_id: "download-88",
                file_name: "archive.bin",
                state: "Queued",
                phase: "Queued",
                storage_mode: "Chunked",
                bytes_done: 0,
                bytes_total: 3 * 1024 * 1024 * 1024,
                error: null,
                speed_bps: 0,
                eta_seconds: null,
                started_at: new Date().toISOString(),
                updated_at: new Date().toISOString(),
              });
              emit("transfer_state_changed", {
                job_id: "download-88",
                file_name: "archive.bin",
                state: "Queued",
                phase: "Queued",
                storage_mode: "Chunked",
                bytes_done: 0,
                bytes_total: 3 * 1024 * 1024 * 1024,
                error: null,
                speed_bps: 0,
                eta_seconds: null,
                started_at: new Date().toISOString(),
                updated_at: new Date().toISOString(),
              });
              return ok({
                cache_state: "Pending",
                cache_mode: args?.cacheMode === "enabled" ? "Enabled" : "Disabled",
                message: "Download enfileirado.",
              });
            case "transfer_cancel":
              return ok(null);
            case "transfer_pause":
              return ok(null);
            case "transfer_resume":
              return ok(null);
            case "settings_get":
              return ok({
                chunk_size_bytes: 134217728,
                max_parallelism: 16,
                encrypt_chunks: true,
                download_cache_default_mode: "Threshold",
                download_cache_threshold_bytes: 2147483648,
                download_cache_write_mode: "Background",
              });
            case "settings_set":
              return ok(null);
            default:
              return err(`unexpected command: ${cmd}`);
          }
        },
      },
      event: {
        listen: async (name: string, cb: (event: any) => void) => {
          const existing = listeners.get(name) || [];
          existing.push(cb);
          listeners.set(name, existing);
          return () => {
            listeners.set(
              name,
              (listeners.get(name) || []).filter((item) => item !== cb)
            );
          };
        },
      },
    };
  });

  await page.goto("/");
});

async function login(page: any) {
  await page.fill("#inputPhone", "+551100000000");
  await page.click("#btnAuthStart");
  await page.fill("#inputCode", "12345");
  await page.click("#btnAuthCode");
  await expect(page.locator("#driveShell")).toBeVisible();
}

test("renders simplified login screen without QR or API fields", async ({ page }) => {
  await expect(page.locator("#authScreen")).toBeVisible();
  await expect(page.locator("#btnQr")).toHaveCount(0);
  await expect(page.locator("#inputApiId")).toHaveCount(0);
  await expect(page.locator("#inputApiHash")).toHaveCount(0);
});

test("full login with code unlocks drive", async ({ page }) => {
  await page.fill("#inputPhone", "+551100000000");
  await page.click("#btnAuthStart");
  await expect(page.locator("#authFormCode")).toBeVisible();
  await page.fill("#inputCode", "12345");
  await page.click("#btnAuthCode");

  await expect(page.locator("#driveShell")).toBeVisible();
  await expect(page.locator("#folderTree li")).toHaveCount(1);
  await expect(page.locator("#folderTree")).toContainText("Saved Messages");
});

test("2fa path requires password", async ({ page }) => {
  await page.fill("#inputPhone", "+551100000000");
  await page.click("#btnAuthStart");

  await page.fill("#inputCode", "00000");
  await page.click("#btnAuthCode");
  await expect(page.locator("#authFormPassword")).toBeVisible();

  await page.fill("#inputPassword", "password123");
  await page.click("#btnAuthPassword");
  await expect(page.locator("#driveShell")).toBeVisible();
});

test("create folder sends native tauri payload and shows success message", async ({ page }) => {
  await login(page);
  await page.click("#btnFolder");
  await page.fill("#newFolderInput", "Projetos");
  await page.click("#btnNewFolderConfirm");

  await expect(page.locator("#driveMessage")).toContainText('Pasta "Projetos" criada.');

  const calls = await page.evaluate(() => (window as any).__TEST_CALLS__);
  const createCall = calls.find((c: any) => c.cmd === "create_folder");
  expect(createCall).toBeTruthy();
  expect(createCall.args).toMatchObject({ parentId: 1, parent_id: 1, name: "Projetos" });
});

test("upload file uses native picker and calls upload_files with folderId", async ({ page }) => {
  await login(page);
  await page.click("#btnUpload");

  await expect(page.locator("#driveMessage")).toContainText("Upload iniciado para 2 arquivo(s).");
  const calls = await page.evaluate(() => (window as any).__TEST_CALLS__);
  const uploadCall = calls.find((c: any) => c.cmd === "upload_files");
  expect(uploadCall).toBeTruthy();
  expect(uploadCall.args).toMatchObject({
    folderId: 1,
    folder_id: 1,
    paths: ["C:/tmp/file-1.txt", "C:/tmp/file-2.jpg"],
  });
});

test("upload folder uses native picker and calls upload_folder with directoryPath", async ({ page }) => {
  await login(page);
  await page.click("#btnUploadFolder");

  await expect(page.locator("#driveMessage")).toContainText("Upload de pasta iniciado.");
  const calls = await page.evaluate(() => (window as any).__TEST_CALLS__);
  const uploadFolderCall = calls.find((c: any) => c.cmd === "upload_folder");
  expect(uploadFolderCall).toBeTruthy();
  expect(uploadFolderCall.args).toMatchObject({
    folderId: 1,
    folder_id: 1,
    directoryPath: "C:/tmp/folder-A",
    directory_path: "C:/tmp/folder-A",
  });
});

test("user menu exposes settings and logout", async ({ page }) => {
  await login(page);
  await page.click("#btnUserMenu");
  await expect(page.locator("#userMenu")).toBeVisible();
  await page.click("#btnOpenSettings");
  await expect(page.locator("#settingsModal")).toBeVisible();
  await page.click("#btnSettingsClose");
  await page.click("#btnUserMenu");
  await page.click("#btnLogout");
  await expect(page.locator("#authScreen")).toBeVisible();
});

test("settings modal lets user choose chunk size profile", async ({ page }) => {
  await login(page);
  await page.click("#btnUserMenu");
  await page.click("#btnOpenSettings");
  await expect(page.locator("#settingsModal")).toBeVisible();
  await page.selectOption("#settingsChunkSize", "268435456");
  await expect(page.locator("#settingsChunkSummary")).toContainText("256 MiB");
  await page.fill("#settingsParallelism", "24");
  await page.click("#btnSettingsSave");

  await expect(page.locator("#driveMessage")).toContainText("Settings salvos.");

  const calls = await page.evaluate(() => (window as any).__TEST_CALLS__);
  const settingsCall = calls.find((c: any) => c.cmd === "settings_set");
  expect(settingsCall).toBeTruthy();
  expect(settingsCall.args.settings).toMatchObject({
    chunk_size_bytes: 268435456,
    max_parallelism: 24,
    encrypt_chunks: true,
    download_cache_default_mode: "Threshold",
    download_cache_threshold_bytes: 2147483648,
    download_cache_write_mode: "Background",
  });
});

test("download modal lets user override cache policy", async ({ page }) => {
  await login(page);
  await page.click(".dl-btn");
  await expect(page.locator("#downloadModal")).toBeVisible();
  await expect(page.locator("#downloadCacheSummary")).toContainText("acima de 2.00 GB");
  await page.selectOption("#downloadCacheMode", "enabled");
  await page.click("#btnDownloadConfirm");
  await expect(page.locator("#driveMessage")).toContainText("Download enfileirado.");
  await expect(page.locator("#queueDrawer")).toBeVisible();
  await expect(page.locator("#progressList")).toContainText("archive.bin");
  await expect(page.locator(".dl-btn").first()).toBeDisabled();

  const calls = await page.evaluate(() => (window as any).__TEST_CALLS__);
  const downloadCall = calls.find((c: any) => c.cmd === "download_file");
  expect(downloadCall).toBeTruthy();
  expect(downloadCall.args).toMatchObject({
    fileId: 88,
    file_id: 88,
    destinationPath: "C:/tmp/downloads/archive.bin",
    destination_path: "C:/tmp/downloads/archive.bin",
    cacheMode: "enabled",
    cache_mode: "enabled",
  });
});

test("queue drawer toggles from header and workspace strip is removed", async ({ page }) => {
  await login(page);
  await expect(page.locator("#currentFolderLabel")).toHaveCount(0);
  await expect(page.locator("#btnQueueToggle")).toBeVisible();
  await page.click("#btnQueueToggle");
  await expect(page.locator("#queueDrawer")).toBeVisible();
  await page.click("#btnQueueClose");
  await expect(page.locator("#queueDrawer")).toBeHidden();
});

test("queue shows transfer direction labels", async ({ page }) => {
  await login(page);
  await page.click("#btnQueueToggle");
  await page.evaluate(() => {
    const event = new CustomEvent("__inject_transfer__", {
      detail: {
        upload: {
          job_id: "upload-1",
          file_name: "archive.bin",
          state: "Running",
          phase: "Uploading",
          storage_mode: "Chunked",
          bytes_done: 1024,
          bytes_total: 2048,
          error: null,
          speed_bps: 100,
          eta_seconds: 30,
          started_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        },
        download: {
          job_id: "download-1",
          file_name: "image.png",
          state: "Completed",
          phase: "Completed",
          storage_mode: "Single",
          bytes_done: 2048,
          bytes_total: 2048,
          error: null,
          speed_bps: 0,
          eta_seconds: null,
          started_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        },
      },
    });
    window.dispatchEvent(event);
  });
  await expect(page.locator("#progressList")).toContainText("Upload");
  await expect(page.locator("#progressList")).toContainText("Download");
});

test("queue supports pause and continue actions", async ({ page }) => {
  await login(page);
  await page.click("#btnQueueToggle");
  await page.evaluate(() => {
    window.dispatchEvent(new CustomEvent("__inject_transfer__", {
      detail: {
        upload: {
          job_id: "upload-77",
          file_name: "huge.iso",
          state: "Running",
          phase: "Uploading",
          storage_mode: "Chunked",
          bytes_done: 1024,
          bytes_total: 4096,
          error: null,
          speed_bps: 100,
          eta_seconds: 30,
          started_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        },
      },
    }));
  });

  await page.getByRole("button", { name: "Pausar" }).click();
  let calls = await page.evaluate(() => (window as any).__TEST_CALLS__);
  expect(calls.some((c: any) => c.cmd === "transfer_pause")).toBeTruthy();

  await page.evaluate(() => {
    (window as any).__emitTauriEvent("transfer_state_changed", {
      job_id: "upload-77",
      file_name: "huge.iso",
      state: "Paused",
      phase: "Uploading",
      storage_mode: "Chunked",
      bytes_done: 1024,
      bytes_total: 4096,
      error: null,
      speed_bps: 0,
      eta_seconds: null,
      started_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    });
  });

  await expect(page.locator("#progressList")).toContainText("Pausado");
  await page.getByRole("button", { name: "Continuar" }).click();
  calls = await page.evaluate(() => (window as any).__TEST_CALLS__);
  expect(calls.some((c: any) => c.cmd === "transfer_resume")).toBeTruthy();
});

test("selection details update when choosing a file", async ({ page }) => {
  await login(page);
  await page.locator("#fileRows tr").first().click();
  await expect(page.locator("#selectionDetails")).toContainText("archive.bin");
  await expect(page.locator("#selectionDetails")).toContainText("chunked");
});

test("native drag-drop uploads paths emitted by tauri", async ({ page }) => {
  await login(page);
  await page.evaluate(() => {
    (window as any).__emitTauriEvent("tauri://drag-drop", {
      paths: ["C:/tmp/dropped.iso"],
    });
  });

  await expect(page.locator("#driveMessage")).toContainText("Upload iniciado para 1 arquivo");
  const calls = await page.evaluate(() => (window as any).__TEST_CALLS__);
  const uploadCall = calls.find((c: any) => c.cmd === "upload_files" && c.args.paths?.includes("C:/tmp/dropped.iso"));
  expect(uploadCall).toBeTruthy();
});

test("native drag-drop also supports raw array payload and dedupes duplicate dispatch", async ({ page }) => {
  await login(page);
  await page.evaluate(() => {
    (window as any).__emitTauriEvent("tauri://drag-drop", ["C:/tmp/raw-drop.iso"]);
    (window as any).__emitTauriEvent("tauri://drag-drop", { paths: ["C:/tmp/raw-drop.iso"] });
  });

  await expect(page.locator("#driveMessage")).toContainText("Upload iniciado para 1 arquivo");
  const calls = await page.evaluate(() => (window as any).__TEST_CALLS__);
  const uploadCalls = calls.filter((c: any) => c.cmd === "upload_files" && c.args.paths?.includes("C:/tmp/raw-drop.iso"));
  expect(uploadCalls).toHaveLength(1);
});

test("dom drag-drop reads dataTransfer files and dispatches a single upload", async ({ page }) => {
  await login(page);
  await page.evaluate(() => {
    const dropZone = document.querySelector("#dropZone");
    const event = new Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(event, "dataTransfer", {
      value: {
        files: [{ path: "C:/tmp/dom-drop.iso", name: "dom-drop.iso" }],
        items: [],
      },
    });
    dropZone?.dispatchEvent(event);
  });

  await expect(page.locator("#driveMessage")).toContainText("Upload iniciado para 1 arquivo");
  const calls = await page.evaluate(() => (window as any).__TEST_CALLS__);
  const uploadCalls = calls.filter((c: any) => c.cmd === "upload_files" && c.args.paths?.includes("C:/tmp/dom-drop.iso"));
  expect(uploadCalls).toHaveLength(1);
});

test("locale follows system/browser locale with english fallback set", async ({ browser }) => {
  const context = await browser.newContext({ locale: "en-US" });
  const page = await context.newPage();
  await page.addInitScript(() => {
    (window as any).__TEST_LOCALE__ = "en-US";
    let authState = "LoggedOut";
    const folders = [{ id: 1, name: "Saved Messages", parent_id: null, created_at: new Date().toISOString(), updated_at: new Date().toISOString() }];
    const ok = (data: any) => ({ ok: true, data, error: null });
    (window as any).__TAURI__ = {
      core: {
        invoke: async (cmd: string, args: any) => {
          switch (cmd) {
            case "auth_status":
              return ok(authState);
            case "auth_prefill":
              return ok({ phone: "+551100000000" });
            case "auth_start":
              authState = "AwaitingCode";
              return ok(authState);
            case "auth_verify_code":
              authState = "LoggedIn";
              return ok(authState);
            case "auth_profile":
              return ok({ display_name: "Telegram User", username: "telegram_user", phone_masked: "***0000", avatar_path_opt: null });
            case "folder_tree":
              return ok(folders);
            case "sync_saved_messages_index":
              return ok(0);
            case "list_folder":
              return ok({ folders: [], files: [], total_folders: 0, total_files: 0 });
            case "search":
              return ok({ folders: [], files: [], total_folders: 0, total_files: 0 });
            case "settings_get":
              return ok({
                chunk_size_bytes: 134217728,
                max_parallelism: 16,
                encrypt_chunks: true,
                download_cache_default_mode: "Threshold",
                download_cache_threshold_bytes: 2147483648,
                download_cache_write_mode: "Background",
              });
            default:
              return ok(null);
          }
        },
      },
      event: {
        listen: async () => () => {},
      },
    };
  });
  await page.goto("/");
  await expect(page.locator("#btnAuthStart")).toContainText("Continue");
  await page.fill("#inputPhone", "+551100000000");
  await page.click("#btnAuthStart");
  await page.fill("#inputCode", "12345");
  await page.click("#btnAuthCode");
  await expect(page.locator("#btnUpload")).toContainText("Upload");
  await page.click("#btnQueueToggle");
  await expect(page.locator("#progressList")).toContainText("No active transfers.");
  await context.close();
});
