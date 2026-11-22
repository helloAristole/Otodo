import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test, { type TestContext } from "node:test";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const workflowPath = resolve(
  repositoryRoot,
  ".github",
  "workflows",
  "release.yml",
);
const checksumScript = resolve(
  repositoryRoot,
  "scripts",
  "create-release-checksums.ps1",
);

function findPowerShell(): string | undefined {
  const candidates =
    process.platform === "win32"
      ? ["pwsh.exe", "powershell.exe"]
      : ["pwsh", "powershell"];

  for (const candidate of candidates) {
    const result = spawnSync(
      candidate,
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "$PSVersionTable.PSVersion.ToString()",
      ],
      { encoding: "utf8" },
    );
    if (result.status === 0) {
      return candidate;
    }
  }

  return undefined;
}

const powershellExecutable = findPowerShell();

function sha256(content: string): string {
  return createHash("sha256").update(content).digest("hex");
}

function requirePowerShell(t: TestContext): string | undefined {
  if (powershellExecutable) {
    return powershellExecutable;
  }

  t.skip("当前平台未找到 pwsh 或 Windows PowerShell，跳过 ps1 执行用例");
  return undefined;
}

function runChecksumScript(
  executable: string,
  bundleRoot: string,
  outputPath: string,
  workingDirectory?: string,
) {
  return spawnSync(
    executable,
    [
      "-NoProfile",
      "-NonInteractive",
      "-File",
      checksumScript,
      "-BundleRoot",
      bundleRoot,
      "-OutputPath",
      outputPath,
    ],
    { cwd: workingDirectory, encoding: "utf8" },
  );
}

test("workflow passes tag through environment instead of expression interpolation", () => {
  const workflow = readFileSync(workflowPath, "utf8");

  assert.match(
    workflow,
    /node scripts\/check-version\.mjs --tag \$env:GITHUB_REF_NAME/,
  );
  assert.doesNotMatch(
    workflow,
    /run:\s*[^\r\n]*\$\{\{\s*github\.ref_name\s*\}\}/,
  );
});

test("workflow uploads checksum to an explicit repository after generation", () => {
  const workflow = readFileSync(workflowPath, "utf8");
  const generationPosition = workflow.indexOf(
    "scripts/create-release-checksums.ps1",
  );
  const uploadPosition = workflow.indexOf("gh release upload");

  assert.notEqual(generationPosition, -1);
  assert.ok(uploadPosition > generationPosition);
  assert.match(workflow, /--repo \$env:REPOSITORY/);
});

test("workflow keeps release draft until checksum is uploaded, then publishes it", () => {
  const workflow = readFileSync(workflowPath, "utf8");
  const draftPosition = workflow.indexOf("releaseDraft: true");
  const uploadPosition = workflow.indexOf("gh release upload");
  const publishPosition = workflow.indexOf("gh release edit");

  assert.notEqual(draftPosition, -1);
  assert.ok(uploadPosition > draftPosition);
  assert.ok(publishPosition > uploadPosition);
  assert.match(workflow, /gh release edit[\s\S]*--draft=false/);
});

test("tauri release build uses locked cargo dependencies", () => {
  const workflow = readFileSync(workflowPath, "utf8");

  assert.match(workflow, /args:\s*"-- --locked"/);
});

test("checksum script writes flat sorted asset names with UTF-8 and LF", (t) => {
  const executable = requirePowerShell(t);
  if (!executable) {
    return;
  }

  const temporaryDirectory = mkdtempSync(
    resolve(tmpdir(), "otodo-release[quality]-"),
  );
  const bundleRoot = resolve(temporaryDirectory, "bundle[1]");
  const outputPath = resolve(bundleRoot, "SHA256SUMS.txt");
  t.after(() => rmSync(temporaryDirectory, { recursive: true, force: true }));

  mkdirSync(resolve(bundleRoot, "nsis"), { recursive: true });
  mkdirSync(resolve(bundleRoot, "msi"), { recursive: true });
  mkdirSync(resolve(bundleRoot, "updater"), { recursive: true });
  writeFileSync(resolve(bundleRoot, "nsis", "Zeta.exe"), "exe-content");
  writeFileSync(resolve(bundleRoot, "msi", "Alpha.msi"), "msi-content");
  writeFileSync(resolve(bundleRoot, "nsis", "ignored.msi"), "ignored");
  writeFileSync(resolve(bundleRoot, "msi", "ignored.exe"), "ignored");
  writeFileSync(resolve(bundleRoot, "updater", "ignored.exe"), "ignored");

  const result = runChecksumScript(executable, bundleRoot, outputPath);

  assert.equal(result.status, 0, result.stderr);
  const checksum = readFileSync(outputPath);
  const expected = [
    `${sha256("msi-content")}  Alpha.msi`,
    `${sha256("exe-content")}  Zeta.exe`,
    "",
  ].join("\n");
  assert.deepEqual(checksum, Buffer.from(expected, "utf8"));
});

test("checksum script resolves relative output from current directory and creates parents", (t) => {
  const executable = requirePowerShell(t);
  if (!executable) {
    return;
  }

  const temporaryDirectory = mkdtempSync(resolve(tmpdir(), "otodo-release-"));
  const bundleRoot = resolve(temporaryDirectory, "bundle");
  const relativeOutputPath = "generated/checks/SHA256SUMS.txt";
  const expectedOutputPath = resolve(temporaryDirectory, relativeOutputPath);
  t.after(() => rmSync(temporaryDirectory, { recursive: true, force: true }));

  mkdirSync(resolve(bundleRoot, "nsis"), { recursive: true });
  writeFileSync(resolve(bundleRoot, "nsis", "Otodo.exe"), "installer");

  const result = runChecksumScript(
    executable,
    bundleRoot,
    relativeOutputPath,
    temporaryDirectory,
  );

  assert.equal(result.status, 0, result.stderr);
  assert.match(readFileSync(expectedOutputPath, "utf8"), /  Otodo\.exe\n$/);
});

test("checksum script fails clearly when bundle directory is missing", (t) => {
  const executable = requirePowerShell(t);
  if (!executable) {
    return;
  }

  const temporaryDirectory = mkdtempSync(resolve(tmpdir(), "otodo-release-"));
  const bundleRoot = resolve(temporaryDirectory, "missing");
  const outputPath = resolve(temporaryDirectory, "SHA256SUMS.txt");
  t.after(() => rmSync(temporaryDirectory, { recursive: true, force: true }));

  const result = runChecksumScript(executable, bundleRoot, outputPath);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /bundle.*不存在/i);
});

test("checksum script fails clearly when expected artifacts are absent", (t) => {
  const executable = requirePowerShell(t);
  if (!executable) {
    return;
  }

  const temporaryDirectory = mkdtempSync(resolve(tmpdir(), "otodo-release-"));
  const bundleRoot = resolve(temporaryDirectory, "bundle");
  const outputPath = resolve(bundleRoot, "SHA256SUMS.txt");
  t.after(() => rmSync(temporaryDirectory, { recursive: true, force: true }));

  mkdirSync(resolve(bundleRoot, "nsis"), { recursive: true });
  mkdirSync(resolve(bundleRoot, "msi"), { recursive: true });
  writeFileSync(resolve(bundleRoot, "nsis", "not-an-installer.txt"), "ignored");

  const result = runChecksumScript(executable, bundleRoot, outputPath);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /nsis\/\*\.exe.*msi\/\*\.msi/i);
});

test("checksum script guards against case-insensitive asset name collisions", () => {
  const script = readFileSync(checksumScript, "utf8");

  // 允许的 nsis/*.exe 与 msi/*.msi 扩展名不同，文件系统中无法自然构造重名用例；
  // 因此这里固定字典比较器和明确错误文案这两个静态契约。
  assert.match(script, /OrdinalIgnoreCase/);
  assert.match(script, /重复/);
});
