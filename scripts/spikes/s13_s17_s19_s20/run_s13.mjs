/**
 * S-13 本地翻译模型资源与降档 · 本机复核 + 8GB 场景模拟。
 *
 * 做什么：
 *   1) 用与本机 S-05 相同的 llama.cpp 后端（LM Studio 内置 llama-server，CPU-only `-ngl 0`）
 *      复核 0.5B / 1.5B / 1.8B 三档模型的**冷加载耗时**与**常驻内存**（WorkingSet / Private / Peak）；
 *   2) 用 Windows Job Object（`s13_job.ps1`，C# P/Invoke）给 llama-server 设 6GB / 8GB / 低内存
 *      进程内存上限，模拟 8GB 机型与内存不足场景，记录是否加载成功、退出码与 stderr 尾部；
 *   3) 按 `resources/models.manifest.json` 的 `minRamMB` 计算 8GB / 16GB 机型的降档判定表，
 *      并记录前端（onboarding `ModelDownloadList` / settings `ModelLibrary`）当前是否读取
 *      `navigator.deviceMemory` 做动态降档提示。
 *
 * 用法（仓库根目录；不占用 Tauri 应用，只起 llama-server 子进程）：
 *   node scripts/spikes/s13_s17_s19_s20/run_s13.mjs [--skip-job] [--skip-baseline]
 *
 * 证据：原始 `.verify/spikes/S-13-S-17-S-19-S-20/raw/s13/`；
 *      聚合 `docs/spikes/data/s13_s17_s19_s20/s13_local_model_resources.json`。
 */
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { EVID, REPO, saveDocsData, saveRaw, sleep } from "./lib.mjs";

const FLAGS = new Set(process.argv.slice(2));

const LLAMA_CANDIDATES = [
  join(process.env.USERPROFILE ?? "C:/Users/zhang", ".lmstudio", "extensions", "backends", "llama.cpp-win-x86_64-avx2-2.28.2", "llama-server.exe"),
];
const ARTIFACTS = join(REPO, ".verify", "spikes", "S-05-S-06", "artifacts");
const MODELS = [
  { tier: "极小档（Qwen2.5-0.5B Q4）", id: "qwen2.5-0.5b-instruct-q4", file: join(ARTIFACTS, "qwen2.5-0.5b-instruct-q4_k_m.gguf") },
  { tier: "1.5B 候选（Qwen2.5-1.5B Q4）", id: "qwen2.5-1.5b-instruct-q4", file: join(ARTIFACTS, "qwen2.5-1.5b-instruct-q4_k_m.gguf") },
  { tier: "普通档（HY-MT1.5-1.8B Q4）", id: "hy-mt-1.8b-q4", file: join(ARTIFACTS, "HY-MT1.5-1.8B-Q4_K_M.gguf") },
];
const JOB_PS1 = join(REPO, "scripts", "spikes", "s13_s17_s19_s20", "s13_job.ps1");
const BASELINE_PORT = 39281;
const JOB_PORT_BASE = 39290;

function findLlama() {
  for (const candidate of LLAMA_CANDIDATES) if (existsSync(candidate)) return candidate;
  return null;
}

function runLlamaJob({ exe, model, limitMB, port, noMmap = false, timeout = 240000 }) {
  const started = Date.now();
  const args = [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    JOB_PS1,
    "-Exe",
    exe,
    "-Model",
    model,
    "-LimitMB",
    String(limitMB),
    "-Port",
    String(port),
    "-Threads",
    "16",
    "-Context",
    "4096",
  ];
  if (noMmap) args.push("-NoMmap");
  const result = spawnSync("powershell.exe", args, { encoding: "utf8", timeout, windowsHide: true, maxBuffer: 8 * 1024 * 1024 });
  const line = String(result.stdout ?? "")
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter((item) => item.startsWith("{"))
    .pop();
  let parsed = null;
  try {
    parsed = line ? JSON.parse(line) : null;
  } catch {
    parsed = null;
  }
  return { wallMs: Date.now() - started, exitCode: result.status, record: parsed, stderr: String(result.stderr ?? "").slice(-1000) };
}

function llamaLeftovers() {
  try {
    const text = execFileSync(
      "powershell.exe",
      ["-NoProfile", "-Command", "@(Get-Process llama-server -ErrorAction SilentlyContinue).Count"],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] },
    ).trim();
    return Number(text) || 0;
  } catch {
    return 0;
  }
}

function systemMemory() {
  try {
    const text = execFileSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-Command",
        "$cs=Get-CimInstance Win32_ComputerSystem; $os=Get-CimInstance Win32_OperatingSystem; [pscustomobject]@{totalMB=[math]::Round($cs.TotalPhysicalMemory/1MB);freeMB=[math]::Round($os.FreePhysicalMemory/1KB);os=$os.Caption;cs=$cs.Model} | ConvertTo-Json",
      ],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] },
    );
    return JSON.parse(text.trim());
  } catch {
    return null;
  }
}

function manifest() {
  const file = join(REPO, "resources", "models.manifest.json");
  return JSON.parse(readFileSync(file, "utf8"));
}

/** 8GB / 16GB 机型的推荐降档表（口径：minRamMB ≤ 系统总内存 × 50%，给 OS/应用留一半）。 */
function downgradeTable(models) {
  const machines = [
    { name: "8GB 机型（模拟）", totalMB: 8192 },
    { name: "16GB 机型（本机 15.78GiB）", totalMB: 16144 },
  ];
  return machines.map((machine) => {
    const budget = Math.round(machine.totalMB * 0.5);
    return {
      machine: machine.name,
      totalMB: machine.totalMB,
      budgetMB: budget,
      recommendations: models.map((model) => {
        const id = model.id;
        const minRamMB = Number(model.minRamMB ?? 0);
        const allowed = minRamMB <= budget;
        let action = "允许";
        if (!allowed) action = "降档/禁止";
        else if (minRamMB > budget * 0.75) action = "允许但提示内存偏紧";
        return { id, minRamMB, allowed, action };
      }),
    };
  });
}

function frontendGap() {
  // 以源码检索为静态证据：前端是否有 navigator.deviceMemory / minRamMB 阈值判断。
  const dirs = ["src"];
  const needles = ["navigator.deviceMemory", "deviceMemory", "minRamMB", "降档", "内存不足"];
  const hits = [];
  for (const dir of dirs) {
    const base = join(REPO, dir);
    if (!existsSync(base)) continue;
    const stack = [base];
    while (stack.length > 0) {
      const current = stack.pop();
      let entries = [];
      try {
        entries = readdirSync(current, { withFileTypes: true });
      } catch {
        continue;
      }
      for (const entry of entries) {
        const full = join(current, entry.name);
        if (entry.isDirectory()) stack.push(full);
        else if (/\.(ts|tsx)$/.test(entry.name)) {
          const text = readFileSync(full, "utf8");
          for (const needle of needles) {
            if (text.includes(needle)) hits.push({ file: full.replace(`${REPO}\\`, ""), needle });
          }
        }
      }
    }
  }
  const modelDownloadList = join(REPO, "src", "windows", "main", "onboarding", "components", "ModelDownloadList.tsx");
  const modelLibrary = join(REPO, "src", "windows", "main", "settings", "components", "ModelLibrary.tsx");
  const read = (file) => (existsSync(file) ? readFileSync(file, "utf8") : "");
  const dl = read(modelDownloadList);
  const lib = read(modelLibrary);
  return {
    grepHits: hits,
    deviceMemoryRead: hits.some((hit) => /deviceMemory/.test(hit.needle)),
    minRamCheckedInFrontend: hits.some((hit) => hit.needle === "minRamMB"),
    downloadListShowsStaticMinRam: /需内存 \{model\.minRamMB\} MB/.test(dl),
    libraryShowsStaticMinRam: /需内存 \{model\.minRamMB\} MB/.test(lib),
    files: [modelDownloadList, modelLibrary],
  };
}

async function main() {
  const exe = findLlama();
  if (!exe) throw new Error("未找到 llama-server.exe（LM Studio 内置后端）");
  const mem = systemMemory();
  const manifestData = manifest();
  const models = Array.isArray(manifestData) ? manifestData : manifestData.models ?? [];
  const evidence = {
    kind: "local-model-resource-recheck + job-object-low-memory-simulation",
    date: new Date().toISOString(),
    env: {
      os: "Windows 11 26200",
      cpu: "i7-12650H (10C16T)",
      ram: mem,
      gpu: "RTX 4060 Laptop（未使用：llama.cpp -ngl 0 纯 CPU）",
      llamaServer: { path: exe, version: "LM Studio 内置 llama.cpp avx2 2.28.2（同 S-05）", args: "-c 4096 -t 16 --host 127.0.0.1 -ngl 0 --no-webui" },
      modelsDir: ARTIFACTS,
    },
    baseline: [],
    jobScenarios: [],
    downgradeTable: null,
    frontend: null,
    leftovers: null,
  };

  if (!FLAGS.has("--skip-baseline")) {
    for (let index = 0; index < MODELS.length; index += 1) {
      const model = MODELS[index];
      if (!existsSync(model.file)) {
        evidence.baseline.push({ ...model, error: "模型文件不存在" });
        continue;
      }
      const run = runLlamaJob({ exe, model: model.file, limitMB: 0, port: BASELINE_PORT + index });
      const row = {
        tier: model.tier,
        id: model.id,
        file: model.file,
        fileMB: Math.round(readFileSyncSize(model.file) / (1024 * 1024)),
        wallMs: run.wallMs,
        ...(run.record ?? {}),
      };
      evidence.baseline.push(row);
      console.log(
        `[s13] baseline ${model.tier}: loaded=${row.loaded} loadMs=${row.loadMs} ws=${row.workingSetMB}MB private=${row.privateMB}MB peak=${row.peakWorkingSetMB}MB`,
      );
      await sleep(800);
    }
    saveRaw("s13", "baseline-memory.json", evidence.baseline);
  }

  if (!FLAGS.has("--skip-job")) {
    const standard = MODELS[2];
    const tiny = MODELS[0];
    const scenarios = [
      { name: "1.8B + 6GB 上限（8GB 机型宽松场景）", model: standard, limitMB: 6144, expectLoaded: true },
      { name: "1.8B + 8GB 上限（16GB 机型下限场景）", model: standard, limitMB: 8192, expectLoaded: true },
      { name: "1.8B + 2.5GB 上限（8GB 机型被其它应用占用）", model: standard, limitMB: 2560, expectLoaded: true },
      { name: "1.8B + 1.5GB 上限（内存严重不足）", model: standard, limitMB: 1536, expectLoaded: false },
      { name: "1.8B + 1.0GB 上限（内存不足，必失败）", model: standard, limitMB: 1024, expectLoaded: false },
      { name: "0.5B + 512MB 上限（极小档内存不足）", model: tiny, limitMB: 512, expectLoaded: false },
    ];
    for (let index = 0; index < scenarios.length; index += 1) {
      const scenario = scenarios[index];
      const run = runLlamaJob({
        exe,
        model: scenario.model.file,
        limitMB: scenario.limitMB,
        port: JOB_PORT_BASE + index,
        timeout: 180000,
        noMmap: true,
      });
      const row = {
        name: scenario.name,
        model: scenario.model.id,
        limitMB: scenario.limitMB,
        expectLoaded: scenario.expectLoaded,
        wallMs: run.wallMs,
        ...(run.record ?? {}),
      };
      row.behavior = row.loaded ? "区间内正常加载" : row.exitCode !== null && row.exitCode !== 0 ? "进程失败退出（有退出码）" : "加载超时/被清理";
      row.pass = scenario.expectLoaded ? row.loaded === true : row.loaded === false;
      evidence.jobScenarios.push(row);
      console.log(
        `[s13] job ${scenario.name}: loaded=${row.loaded} exitCode=${row.exitCode} wall=${row.wallMs}ms pass=${row.pass}`,
      );
      await sleep(800);
      const leftover = llamaLeftovers();
      if (leftover > 0) {
        console.warn(`[s13] 清理 llama-server 残留 ${leftover} 个`);
        execFileSync("powershell.exe", ["-NoProfile", "-Command", "Get-Process llama-server -ErrorAction SilentlyContinue | Stop-Process -Force"], {
          stdio: "ignore",
        });
      }
    }
    saveRaw("s13", "job-object-scenarios.json", evidence.jobScenarios);
  }

  evidence.downgradeTable = downgradeTable(models.filter((model) => model.kind === "gguf"));
  evidence.frontend = frontendGap();
  evidence.leftovers = { llamaServer: llamaLeftovers(), snipAsk: 0 };
  saveRaw("s13", "resources-full.json", evidence);

  const baselineByTier = Object.fromEntries(evidence.baseline.map((row) => [row.id ?? row.tier, row]));
  saveDocsData("s13_local_model_resources.json", {
    kind: evidence.kind,
    date: evidence.date,
    env: evidence.env,
    baseline: evidence.baseline.map((row) => ({
      tier: row.tier,
      id: row.id ?? null,
      fileMB: row.fileMB ?? null,
      loadMs: row.loadMs ?? null,
      workingSetMB: row.workingSetMB ?? null,
      privateMB: row.privateMB ?? null,
      peakWorkingSetMB: row.peakWorkingSetMB ?? null,
      loaded: row.loaded ?? null,
    })),
    jobScenarios: evidence.jobScenarios.map((row) => ({
      name: row.name,
      limitMB: row.limitMB,
      loaded: row.loaded,
      exitCode: row.exitCode,
      wallMs: row.wallMs,
      behavior: row.behavior,
      stderrTail: (row.stderrTail ?? []).slice(-6),
      pass: row.pass,
    })),
    downgradeTable: evidence.downgradeTable,
    frontend: {
      deviceMemoryRead: evidence.frontend.deviceMemoryRead,
      minRamCheckedInFrontend: evidence.frontend.minRamCheckedInFrontend,
      downloadListShowsStaticMinRam: evidence.frontend.downloadListShowsStaticMinRam,
      libraryShowsStaticMinRam: evidence.frontend.libraryShowsStaticMinRam,
      grepHits: evidence.frontend.grepHits.slice(0, 20),
    },
    leftovers: evidence.leftovers,
  });
  console.log(
    `[s13] 完成：baseline=${evidence.baseline.length} 条，job 场景=${evidence.jobScenarios.filter((row) => row.pass).length}/${evidence.jobScenarios.length} 通过；前端动态降档=${evidence.frontend.deviceMemoryRead}；llama-server 残留=${evidence.leftovers.llamaServer}`,
  );
  void baselineByTier;
}

function readFileSyncSize(file) {
  return existsSync(file) ? statSync(file).size : 0;
}

main().catch((error) => {
  console.error(`[s13] 失败：${error.message}`);
  saveRaw("s13", "failure.json", { at: new Date().toISOString(), message: String(error.message ?? error), stack: String(error.stack ?? "") });
  process.exit(1);
});
