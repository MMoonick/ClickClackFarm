import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import test from "node:test";
const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
const json = (path) => JSON.parse(read(path));

test("release versions and transparent Windows window settings agree", () => {
  const base = json("src-tauri/tauri.conf.json");
  const windows = json("src-tauri/tauri.windows.conf.json");
  assert.equal(base.version, "1.0.4");
  assert.equal(json("package.json").version, base.version);
  const { shadow, url, ...shared } = windows.app.windows[0];
  assert.deepEqual(shared, base.app.windows[0]);
  assert.equal(shadow, false);
  assert.equal(url, `index.html?build=windows-${base.version}`);
  assert.deepEqual(windows.bundle.targets, ["nsis"]);
  assert.ok(existsSync(new URL("../src-tauri/icons/icon.ico", import.meta.url)));
});

test("public app has no test interface, diagnostic feature, or reset command", () => {
  for (const file of ["src/App.tsx", "src/styles.css", "src-tauri/Cargo.toml", "src-tauri/src/main.rs", "src-tauri/src/input_probe.rs", "package.json"])
    assert.doesNotMatch(read(file), /windows-input-diagnostics|offline-test|inputDiagnostics|WindowsInputDiagnostics|reset_game|Windows 输入测试|界面版本：W3|重置 Demo/);
  assert.match(read("src-tauri/src/game_backend.rs"), /const DEMO_INITIAL_COINS: u64 = 100;/);
});

test("latest hide control and announcement credit remain in the release", () => {
  assert.match(read("src/App.tsx"), /className="window-hide"[\s\S]*?onClick=\{hideMainWindow\}/);
  assert.match(read("src/App.tsx"), /getCurrentWindow\(\)\.hide\(\)/);
  assert.equal(json("src/ui-tokens.json").copy.announcementCreator, "—— 制作者 MMoonick");
  assert.match(read("src/styles.css"), /\.notice-creator\s*\{[^}]*text-align:\s*right/);
});

test("Windows release retains Raw Input keyboard and hidden native menu", () => {
  assert.match(read("src-tauri/src/windows_input.rs"), /RegisterRawInputDevices/);
  assert.doesNotMatch(read("src-tauri/src/windows_input.rs"), /WH_KEYBOARD_LL/);
  assert.match(read("src-tauri/src/window_probe.rs"), /\.hide_menu\(\)/);
});

test("Windows tray reuses the macOS icon and existing show/exit handling without notifications", () => {
  const main = read("src-tauri/src/main.rs");
  const tray = main.slice(main.indexOf("fn install_windows_tray"), main.indexOf("fn main()"));
  assert.match(main, /#\[cfg\(target_os = "windows"\)\]\s*fn install_windows_tray/);
  assert.match(main, /#\[cfg\(target_os = "windows"\)\]\s*install_windows_tray\(app\)\?/);
  assert.match(tray, /include_image!\(\s*"icons\/AppIcon.iconset\/icon_32x32.png"\s*\)/);
  const icon = readFileSync(new URL("../src-tauri/icons/AppIcon.iconset/icon_32x32.png", import.meta.url));
  assert.equal(icon.readUInt32BE(16), 32);
  assert.equal(icon.readUInt32BE(20), 32);
  assert.match(tray, /with_id\(MENU_SHOW, "显示牧场"\)/);
  assert.match(tray, /with_id\(MENU_QUIT, "退出游戏"\)/);
  assert.match(tray, /show_menu_on_left_click\(false\)/);
  assert.match(tray, /button: MouseButton::Left/);
  assert.match(tray, /button_state: MouseButtonState::Up/);
  assert.match(tray, /show_main_window\(tray.app_handle\(\)\)/);
  assert.match(main, /MENU_QUIT => app.exit\(0\)/);
  assert.match(main, /RunEvent::ExitRequested \{ \.\. \} => prepare_exit\(app\)/);
  assert.doesNotMatch(tray, /notification|balloon|message_dialog|MessageDialog/);
});
