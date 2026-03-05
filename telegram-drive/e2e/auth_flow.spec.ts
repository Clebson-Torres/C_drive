import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    let authState = "LoggedOut";
    const folders = [{ id: 1, name: "Root", parent_id: null, created_at: new Date().toISOString(), updated_at: new Date().toISOString() }];
    const calls: Array<{ cmd: string; args: any }> = [];
    (window as any).__TEST_CALLS__ = calls;

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
              return ok({ phone: "+551100000000", api_id: 37673970, api_hash: "hash" });
            case "auth_start":
              if (!args?.input?.phone || !args?.input?.api_id || !args?.input?.api_hash) {
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
            case "list_folder":
              return ok({ folders: [], files: [], total_folders: 0, total_files: 0 });
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
            case "transfer_cancel":
              return ok(null);
            case "settings_get":
              return ok({ chunk_size_bytes: 33554432, max_parallelism: 16, encrypt_chunks: true });
            case "settings_set":
              return ok(null);
            default:
              return err(`unexpected command: ${cmd}`);
          }
        },
      },
      event: {
        listen: async () => () => {},
      },
    };
  });

  await page.goto("/");
});

async function login(page: any) {
  await page.fill("#inputPhone", "+551100000000");
  await page.fill("#inputApiId", "37673970");
  await page.fill("#inputApiHash", "hash");
  await page.click("#btnAuthStart");
  await page.fill("#inputCode", "12345");
  await page.click("#btnAuthCode");
  await expect(page.locator("#driveShell")).toBeVisible();
}

test("renders login screen and no QR button", async ({ page }) => {
  await expect(page.locator("#authScreen")).toBeVisible();
  await expect(page.locator("#btnQr")).toHaveCount(0);
  await expect(page.locator("#btnApiHelp")).toBeVisible();
});

test("full login with code unlocks drive", async ({ page }) => {
  await page.fill("#inputPhone", "+551100000000");
  await page.fill("#inputApiId", "37673970");
  await page.fill("#inputApiHash", "hash");
  await page.click("#btnAuthStart");
  await expect(page.locator("#authFormCode")).toBeVisible();
  await page.fill("#inputCode", "12345");
  await page.click("#btnAuthCode");

  await expect(page.locator("#driveShell")).toBeVisible();
  await expect(page.locator("#folderTree li")).toHaveCount(1);
});

test("2fa path requires password", async ({ page }) => {
  await page.fill("#inputPhone", "+551100000000");
  await page.fill("#inputApiId", "37673970");
  await page.fill("#inputApiHash", "hash");
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
  await page.selectOption("#settingsChunkSize", "67108864");
  await expect(page.locator("#settingsChunkSummary")).toContainText("64 MiB");
  await page.fill("#settingsParallelism", "24");
  await page.click("#btnSettingsSave");

  await expect(page.locator("#driveMessage")).toContainText("Settings salvos.");

  const calls = await page.evaluate(() => (window as any).__TEST_CALLS__);
  const settingsCall = calls.find((c: any) => c.cmd === "settings_set");
  expect(settingsCall).toBeTruthy();
  expect(settingsCall.args.settings).toMatchObject({
    chunk_size_bytes: 67108864,
    max_parallelism: 24,
    encrypt_chunks: true,
  });
});

test("api help modal shows official link", async ({ page }) => {
  await page.click("#btnApiHelp");
  await expect(page.locator("#apiHelpModal")).toBeVisible();
  await expect(page.locator("#apiHelpLink")).toHaveAttribute("href", "https://my.telegram.org/apps");
});
