import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const VERSION_FILES = {
  packageJson: "package.json",
  cargoToml: "src-tauri/Cargo.toml",
  tauriConfig: "src-tauri/tauri.conf.json",
};

const TOML_KEY = String.raw`(?:"(?:[^"\\]|\\.)*"|'[^']*'|[A-Za-z0-9_-]+)`;
const TOML_TABLE_PATH = String.raw`${TOML_KEY}(?:\s*\.\s*${TOML_KEY})*`;
const TOML_TABLE_HEADER = new RegExp(
  String.raw`^\s*(?:\[\s*${TOML_TABLE_PATH}\s*\]|\[\[\s*${TOML_TABLE_PATH}\s*\]\])\s*(?:#.*)?$`,
);
const PACKAGE_TABLE_HEADER = /^\s*\[\s*package\s*\]\s*(?:#.*)?$/;

function readTextFile(projectDirectory, relativePath) {
  try {
    return readFileSync(resolve(projectDirectory, relativePath), "utf8");
  } catch (error) {
    const reason = error.code ? `${error.code}: ${error.message}` : error.message;
    throw new Error(`无法读取文件（${reason}）`);
  }
}

function readJsonVersion(projectDirectory, relativePath) {
  const content = readTextFile(projectDirectory, relativePath);
  let parsed;

  try {
    parsed = JSON.parse(content);
  } catch (error) {
    throw new Error(`JSON 格式无效（${error.message}）`);
  }

  if (
    parsed === null ||
    typeof parsed !== "object" ||
    !Object.hasOwn(parsed, "version")
  ) {
    throw new Error("缺少 version");
  }

  if (typeof parsed.version !== "string") {
    const actualType = parsed.version === null ? "null" : typeof parsed.version;
    throw new Error(`version 必须是字符串（当前为 ${actualType}）`);
  }

  if (parsed.version.trim().length === 0) {
    throw new Error("version 不能为空或只包含空白");
  }

  return parsed.version;
}

function readCargoVersion(projectDirectory) {
  const content = readTextFile(projectDirectory, VERSION_FILES.cargoToml);
  const lines = content.split(/\r?\n/);
  const packageStart = lines.findIndex((line) =>
    PACKAGE_TABLE_HEADER.test(line),
  );

  if (packageStart < 0) {
    throw new Error("缺少 [package] 段");
  }

  for (const line of lines.slice(packageStart + 1)) {
    if (TOML_TABLE_HEADER.test(line)) {
      break;
    }

    if (!/^\s*version\s*=/.test(line)) {
      continue;
    }

    const match = line.match(
      /^\s*version\s*=\s*(?:"([^"]*)"|'([^']*)')\s*(?:#.*)?$/,
    );
    if (!match) {
      throw new Error("[package] 中的 version 必须是字符串");
    }

    const version = match[1] ?? match[2];
    if (version.trim().length === 0) {
      throw new Error("[package] 中的 version 不能为空或只包含空白");
    }

    return version;
  }

  throw new Error("[package] 中缺少 version");
}

function errorMessage(error) {
  if (error instanceof Error) {
    return error.message;
  }

  return String(error);
}

export function checkVersionConsistency(projectDirectory, tag) {
  const errors = [];
  const versions = {};

  const readers = [
    [
      VERSION_FILES.packageJson,
      () => readJsonVersion(projectDirectory, VERSION_FILES.packageJson),
      "packageJson",
    ],
    [VERSION_FILES.cargoToml, () => readCargoVersion(projectDirectory), "cargoToml"],
    [
      VERSION_FILES.tauriConfig,
      () => readJsonVersion(projectDirectory, VERSION_FILES.tauriConfig),
      "tauriConfig",
    ],
  ];

  for (const [relativePath, readVersion, key] of readers) {
    try {
      versions[key] = readVersion();
    } catch (error) {
      errors.push(`- ${relativePath}: ${errorMessage(error)}`);
    }
  }

  const expectedVersion = versions.packageJson;
  if (expectedVersion) {
    for (const [key, relativePath] of [
      ["cargoToml", VERSION_FILES.cargoToml],
      ["tauriConfig", VERSION_FILES.tauriConfig],
    ]) {
      const actualVersion = versions[key];
      if (actualVersion && actualVersion !== expectedVersion) {
        errors.push(
          `- ${relativePath}: ${actualVersion}（应与 package.json ${expectedVersion} 一致）`,
        );
      }
    }

    if (tag !== undefined && tag !== `v${expectedVersion}`) {
      errors.push(`- CI tag: ${tag}（必须严格等于 v${expectedVersion}）`);
    }
  }

  return {
    errors,
    version: expectedVersion,
  };
}

function parseTag(argumentsList) {
  if (argumentsList.length === 0) {
    return undefined;
  }

  if (
    argumentsList.length !== 2 ||
    argumentsList[0] !== "--tag" ||
    argumentsList[1].trim().length === 0
  ) {
    throw new Error("用法: node scripts/check-version.mjs [--tag <tag>]");
  }

  return argumentsList[1];
}

function main() {
  let tag;
  try {
    tag = parseTag(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
    return;
  }

  const result = checkVersionConsistency(process.cwd(), tag);
  if (result.errors.length > 0) {
    console.error(`版本检查失败:\n${result.errors.join("\n")}`);
    process.exitCode = 1;
    return;
  }

  const tagMessage = tag === undefined ? "" : `，CI tag ${tag}`;
  console.log(`版本检查通过: ${result.version}${tagMessage}`);
}

const isExecutedDirectly =
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href;

if (isExecutedDirectly) {
  main();
}
