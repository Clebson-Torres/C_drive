import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    let authState = "LoggedOut";
    const folders = [
      {
        id: 1,
        name: "Root",
        parent_id: null,
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
      },
    ];

    const ok = (data: any) => ({ ok: true, data, error: null });
    const err = (message: string) => ({ ok: false, data: null, error: message });

    (window as any).__TAURI__ = {
      core: {
        invoke: async (cmd: string, args: any) => {
          switch (cmd) {
            case "auth_status":
              return ok(authState);
            case "auth_start":
              authState = "AwaitingCode";
              return ok(authState);
            case "auth_verify_code":
              authState = "LoggedIn";
              return ok(authState);
            case "folder_tree":
              return ok(folders);
            case "list_folder":
              return ok({ folders: [], files: [], total_folders: 0, total_files: 0 });
            case "pick_files_native":
              return ok(["C:/tmp/x.txt"]);
            case "pick_folder_native":
              return ok("C:/tmp/folder-A");
            case "upload_files":
              if (!Object.prototype.hasOwnProperty.call(args || {}, "folder_id")) {
                return err("missing required key folder_id");
              }
              return ok([1]);
            case "upload_folder":
              if (!Object.prototype.hasOwnProperty.call(args || {}, "folder_id")) {
                return err("missing required key folder_id");
              }
              if (!Object.prototype.hasOwnProperty.call(args || {}, "directory_path")) {
                return err("missing required key directory_path");
              }
              return ok([10, 11]);
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
  await page.fill("#inputPhone", "+551100000000");
  await page.fill("#inputApiId", "37673970");
  await page.fill("#inputApiHash", "hash");
  await page.click("#btnAuthStart");
  await page.fill("#inputCode", "12345");
  await page.click("#btnAuthCode");
  await expect(page.locator("#driveShell")).toBeVisible();
});

test("click upload works even with snake_case backend expectation", async ({ page }) => {
  await page.click("#btnUpload");
  await expect(page.locator("#driveMessage")).toContainText("Upload iniciado");
});

test("click upload folder works even with snake_case backend expectation", async ({ page }) => {
  await page.click("#btnUploadFolder");
  await expect(page.locator("#driveMessage")).toContainText("Upload de pasta iniciado.");
});

