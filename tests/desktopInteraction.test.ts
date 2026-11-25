import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const appSource = readFileSync(resolve(repositoryRoot, "src", "App.tsx"), "utf8");

test("window position lock does not disable todo card dragging", () => {
  const handlerStart = appSource.indexOf("const onTodoDragStart");
  const handlerEnd = appSource.indexOf("const onTodoDrop", handlerStart);

  assert.notEqual(handlerStart, -1);
  assert.ok(handlerEnd > handlerStart);

  const dragStartHandler = appSource.slice(handlerStart, handlerEnd);
  assert.doesNotMatch(dragStartHandler, /\blocked\b/);
});

test("clicking the settings button again closes the panel", () => {
  const buttonStart = appSource.indexOf('className={`settings-btn ${settingsOpen ? "active" : ""}`}');
  const buttonEnd = appSource.indexOf("<IconSettings />", buttonStart);

  assert.notEqual(buttonStart, -1);
  assert.ok(buttonEnd > buttonStart);

  const settingsButton = appSource.slice(buttonStart, buttonEnd);
  assert.match(settingsButton, /onMouseDown=\{\(e\) => e\.stopPropagation\(\)\}/);
  assert.match(settingsButton, /setSettingsOpen\(\(v\) => !v\)/);
});

test("updates are checked only from the settings button", () => {
  const invocations = appSource.match(/\bcheckForUpdates\(\)/g) ?? [];

  assert.equal(invocations.length, 1);
  assert.match(
    appSource,
    /onClick=\{\(\) => \{ void checkForUpdates\(\); \}\}/,
  );
  assert.match(appSource, /"检查更新"/);
  assert.match(appSource, /当前已是最新版本/);
});
