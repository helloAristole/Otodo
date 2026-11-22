import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const versionCheckScript = resolve(repositoryRoot, "scripts", "check-version.mjs");

type ProjectFile =
  | "package.json"
  | "src-tauri/Cargo.toml"
  | "src-tauri/tauri.conf.json";

const validFiles: Record<ProjectFile, string> = {
  "package.json": JSON.stringify({ version: "1.2.3" }),
  "src-tauri/Cargo.toml":
    '[package]\nname = "otodo"\nversion = "1.2.3"\n',
  "src-tauri/tauri.conf.json": JSON.stringify({ version: "1.2.3" }),
};

function createProject(
  overrides: Partial<Record<ProjectFile, string | null>> = {},
): string {
  const projectDirectory = mkdtempSync(resolve(tmpdir(), "otodo-version-check-"));
  mkdirSync(resolve(projectDirectory, "src-tauri"));

  for (const [relativePath, defaultContent] of Object.entries(validFiles)) {
    const content =
      overrides[relativePath as ProjectFile] === undefined
        ? defaultContent
        : overrides[relativePath as ProjectFile];
    if (content !== null) {
      writeFileSync(resolve(projectDirectory, relativePath), content);
    }
  }

  return projectDirectory;
}

function runVersionCheck(projectDirectory: string, argumentsList: string[] = []) {
  return spawnSync(process.execPath, [versionCheckScript, ...argumentsList], {
    cwd: projectDirectory,
    encoding: "utf8",
  });
}

test("local check passes when all three file versions match", (t) => {
  const projectDirectory = createProject();
  t.after(() => rmSync(projectDirectory, { recursive: true, force: true }));

  const result = runVersionCheck(projectDirectory);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /1\.2\.3/);
});

test("check reports every mismatched version", (t) => {
  const projectDirectory = createProject({
    "src-tauri/Cargo.toml":
      '[package]\nname = "otodo"\nversion = "1.2.4"\n',
    "src-tauri/tauri.conf.json": JSON.stringify({ version: "1.2.5" }),
  });
  t.after(() => rmSync(projectDirectory, { recursive: true, force: true }));

  const result = runVersionCheck(projectDirectory);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /src-tauri\/Cargo\.toml.*1\.2\.4.*1\.2\.3/);
  assert.match(result.stderr, /src-tauri\/tauri\.conf\.json.*1\.2\.5.*1\.2\.3/);
});

test("CI check accepts only the exact v<version> tag", (t) => {
  const projectDirectory = createProject();
  t.after(() => rmSync(projectDirectory, { recursive: true, force: true }));

  const validResult = runVersionCheck(projectDirectory, ["--tag", "v1.2.3"]);
  assert.equal(validResult.status, 0, validResult.stderr);

  for (const invalidTag of ["1.2.3", "v1.2.4", "v1.2.3-beta"]) {
    const invalidResult = runVersionCheck(projectDirectory, [
      "--tag",
      invalidTag,
    ]);
    assert.equal(invalidResult.status, 1, `tag ${invalidTag} should fail`);
    assert.match(invalidResult.stderr, /tag.*v1\.2\.3/i);
  }
});

test("Cargo parser ignores dependency versions and supports comments and quotes", (t) => {
  const projectDirectory = createProject({
    "src-tauri/Cargo.toml": [
      "[dependencies]",
      'version = "9.9.9"',
      "",
      "[package] # application metadata",
      'name = "otodo"',
      "version = '1.2.3' # release version",
      "",
      "[package.metadata.release] # later section",
      'version = "8.8.8"',
      "",
    ].join("\n"),
  });
  t.after(() => rmSync(projectDirectory, { recursive: true, force: true }));

  const result = runVersionCheck(projectDirectory);

  assert.equal(result.status, 0, result.stderr);
});

test("Cargo parser does not treat value lines as table headers", (t) => {
  const projectDirectory = createProject({
    "src-tauri/Cargo.toml": [
      "[package]",
      'name = "otodo"',
      "keywords = [",
      '  "calendar",',
      "]",
      'description = "[not a table header]"',
      'version = "1.2.3" # release version',
      "",
    ].join("\n"),
  });
  t.after(() => rmSync(projectDirectory, { recursive: true, force: true }));

  const result = runVersionCheck(projectDirectory);

  assert.equal(result.status, 0, result.stderr);
});

test("missing files are all reported in one run", (t) => {
  const projectDirectory = createProject({
    "package.json": null,
    "src-tauri/Cargo.toml": null,
    "src-tauri/tauri.conf.json": null,
  });
  t.after(() => rmSync(projectDirectory, { recursive: true, force: true }));

  const result = runVersionCheck(projectDirectory);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /package\.json.*无法读取/);
  assert.match(result.stderr, /src-tauri\/Cargo\.toml.*无法读取/);
  assert.match(result.stderr, /src-tauri\/tauri\.conf\.json.*无法读取/);
});

test("invalid JSON files are both reported in one run", (t) => {
  const projectDirectory = createProject({
    "package.json": "{",
    "src-tauri/tauri.conf.json": "{",
  });
  t.after(() => rmSync(projectDirectory, { recursive: true, force: true }));

  const result = runVersionCheck(projectDirectory);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /package\.json.*JSON 格式无效/);
  assert.match(result.stderr, /src-tauri\/tauri\.conf\.json.*JSON 格式无效/);
});

for (const invalidVersion of [
  {
    label: "missing",
    value: { dependencies: { example: { version: "9.9.9" } } },
  },
  { label: "blank", value: { version: "   " } },
  { label: "null", value: { version: null } },
  { label: "non-string", value: { version: 123 } },
]) {
  test(`JSON version rejects ${invalidVersion.label} values`, (t) => {
    const projectDirectory = createProject({
      "package.json": JSON.stringify(invalidVersion.value),
    });
    t.after(() => rmSync(projectDirectory, { recursive: true, force: true }));

    const result = runVersionCheck(projectDirectory);

    assert.equal(result.status, 1);
    assert.match(result.stderr, /package\.json.*version/);
    assert.doesNotMatch(result.stderr, /TypeError/);
  });
}

test("Cargo file requires a package table", (t) => {
  const projectDirectory = createProject({
    "src-tauri/Cargo.toml": '[dependencies]\nversion = "1.2.3"\n',
  });
  t.after(() => rmSync(projectDirectory, { recursive: true, force: true }));

  const result = runVersionCheck(projectDirectory);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /Cargo\.toml.*缺少 \[package\] 段/);
});

test("Cargo package table requires a nonblank version", (t) => {
  const projectDirectory = createProject({
    "src-tauri/Cargo.toml":
      '[package]\nname = "otodo"\nversion = "   " # invalid\n',
  });
  t.after(() => rmSync(projectDirectory, { recursive: true, force: true }));

  const result = runVersionCheck(projectDirectory);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /Cargo\.toml.*version.*空白/);
});

test("Cargo package table reports a missing version", (t) => {
  const projectDirectory = createProject({
    "src-tauri/Cargo.toml": '[package]\nname = "otodo"\n',
  });
  t.after(() => rmSync(projectDirectory, { recursive: true, force: true }));

  const result = runVersionCheck(projectDirectory);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /Cargo\.toml.*缺少 version/);
});

for (const invalidArguments of [
  { label: "missing tag value", values: ["--tag"] },
  { label: "blank tag value", values: ["--tag", ""] },
  { label: "unknown argument", values: ["--unknown"] },
]) {
  test(`CLI rejects ${invalidArguments.label}`, (t) => {
    const projectDirectory = createProject();
    t.after(() => rmSync(projectDirectory, { recursive: true, force: true }));

    const result = runVersionCheck(projectDirectory, invalidArguments.values);

    assert.equal(result.status, 1);
    assert.match(result.stderr, /用法/);
  });
}
