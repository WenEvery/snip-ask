#!/usr/bin/env node
/**
 * S-05 本地翻译链路时延基准（llama.cpp llama-server + HY-MT1.5-1.8B Q4_K_M）。
 *
 * 流程：
 *   1. 启动 llama-server.exe -m <gguf> -c 4096 -t <threads> --port 39281 --host 127.0.0.1
 *   2. 轮询 GET /health 直到就绪，记录「进程启动 → /health 就绪」的冷加载耗时
 *   3. 用与 bench_translate.py 相同的 200 字中文样例（resources/prompts/translate.md）
 *      调 POST /v1/chat/completions，warmup 后测 20 次
 *   4. 记录客户端总耗时 / 首字节 / llama.cpp 服务端 timings（prompt_ms、predicted_ms、tok/s）
 *   5. 通过 PowerShell 读取 llama-server 进程 WorkingSet / PrivateMemory / PeakWorkingSet
 *   6. 关闭进程，输出 JSON 到 .verify/spikes/S-05-S-06/raw/
 *
 * DONE 阈值（技术验证清单 §2）：本地 1.8B Q4 200 字 ≤ 8s（首次加载另计）。
 *
 * 说明：
 * - 优先使用本机已有的 llama.cpp 预编译 llama-server.exe（LM Studio backend，等价 llama.cpp
 *   官方 server 前端）；也支持 LLAMA_SERVER_EXE / --exe 指定 GitHub release 解压后的 exe。
 * - idleUnloadSec=600 属于应用层策略（SnipAsk 设置项），llama-server 本身无空闲卸载；
 *   本脚本只能记录「常驻内存」与「未卸载」事实，不能测卸载行为（文档中注明）。
 *
 * 用法：
 *   node scripts/spikes/s05_s06/bench_local_translate.mjs --runs 20 --threads 16
 *   node scripts/spikes/s05_s06/bench_local_translate.mjs --self-check   # 只做前置检查
 *
 * 退出码：0 = 已完成真实测量；2 = BLOCKED（exe/gguf 缺失或加载失败）。
 */

import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { freemem, cpus, platform, release } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { performance } from 'node:perf_hooks';

const SCRIPT_VERSION = '1.0.0';
const THRESHOLD_MS = 8000;
const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(SCRIPT_DIR, '..', '..', '..');
const RAW_DIR = join(REPO_ROOT, '.verify', 'spikes', 'S-05-S-06', 'raw');
const ARTIFACT_DIR = join(REPO_ROOT, '.verify', 'spikes', 'S-05-S-06', 'artifacts');
const PROMPT_FILE = join(REPO_ROOT, 'resources', 'prompts', 'translate.md');

/** 与 bench_translate.py 完全一致的 200 字中文样例（含 160 汉字 + 标点/空格/ASCII 标识符）。 */
const SAMPLE_TEXT =
  '框选 AI 是一款屏幕取词与截图问答工具：按下 Ctrl+Q 唤起选框，框住屏幕上的文字或图像，' +
  '松开鼠标后弹出动作菜单。你可以翻译选中段落、提取文字并搜索、识别图片内容，或向 AI 提问。' +
  '所有动作都在结果窗口中展示，支持复制、重新执行与追问。云端模型需要配置 API Key，' +
  '本地模型下载完成后即可离线使用。首次使用建议先完成引导，选择常用语言与默认动作。' +
  '若遇到快捷键冲突，可在设置中修改或暂停热键。';
if (SAMPLE_TEXT.length !== 200) throw new Error(`SAMPLE_TEXT 必须为 200 字符，当前 ${SAMPLE_TEXT.length}`);

const EXIT_OK = 0;
const EXIT_BLOCKED = 2;

// ---------------------------------------------------------------- utils ----
function parseArgs(argv) {
  const out = {
    exe: process.env.LLAMA_SERVER_EXE || '',
    model: process.env.LLAMA_MODEL_GGUF || '',
    port: 39281,
    host: '127.0.0.1',
    ctx: 4096,
    threads: Math.min(16, Math.max(1, cpus().length)),
    gpuLayers: 0,
    runs: 20,
    warmup: 2,
    maxTokens: 1024,
    temperature: 0.2,
    topP: 0.95,
    topK: 0,
    repeatPenalty: 1.0,
    timeout: 120000,
    out: '',
    selfCheck: false,
    logFile: '',
    promptVariant: 'product',
    text: '',
  };
  const take = (name, key, kind = 'string') => {
    const i = argv.indexOf(name);
    if (i >= 0 && i + 1 < argv.length) out[key] = kind === 'number' ? Number(argv[i + 1]) : argv[i + 1];
  };
  ['--exe', '--model', '--port', '--host', '--ctx', '--threads', '--gpu-layers', '--runs', '--warmup', '--max-tokens', '--temperature', '--top-p', '--top-k', '--repeat-penalty', '--timeout', '--out', '--log-file', '--prompt-variant', '--text'].forEach(
    (flag, i) => {
      const key = {
        '--exe': 'exe',
        '--model': 'model',
        '--port': 'port',
        '--host': 'host',
        '--ctx': 'ctx',
        '--threads': 'threads',
        '--gpu-layers': 'gpuLayers',
        '--runs': 'runs',
        '--warmup': 'warmup',
        '--max-tokens': 'maxTokens',
        '--temperature': 'temperature',
        '--top-p': 'topP',
        '--top-k': 'topK',
        '--repeat-penalty': 'repeatPenalty',
        '--timeout': 'timeout',
        '--out': 'out',
        '--log-file': 'logFile',
        '--prompt-variant': 'promptVariant',
        '--text': 'text',
      }[flag];
      const num = ['port', 'ctx', 'threads', 'gpuLayers', 'runs', 'warmup', 'maxTokens', 'temperature', 'topP', 'topK', 'repeatPenalty', 'timeout'].includes(key);
      take(flag, key, num ? 'number' : 'string');
    },
  );
  out.selfCheck = argv.includes('--self-check');
  return out;
}

function findCandidateExe() {
  const env = process.env.LLAMA_SERVER_EXE;
  if (env && existsSync(env)) return env;
  const local = join(ARTIFACT_DIR, 'llama-b12110-bin-win-cpu-x64', 'llama-server.exe');
  if (existsSync(local)) return local;
  // 本机已有的 llama.cpp 预编译（LM Studio backend，CPU 版优先，便于复现）
  const lmRoot = join(process.env.USERPROFILE || 'C:', '.lmstudio', 'extensions', 'backends');
  const prefs = [
    'llama.cpp-win-x86_64-avx2-2.28.2',
    'llama.cpp-win-x86_64-nvidia-cuda12-avx2-2.67.0',
    'llama.cpp-win-x86_64-nvidia-cuda-avx2-2.28.2',
  ];
  for (const p of prefs) {
    const candidate = join(lmRoot, p, 'llama-server.exe');
    if (existsSync(candidate)) return candidate;
  }
  return '';
}

function findCandidateModel() {
  const env = process.env.LLAMA_MODEL_GGUF;
  if (env && existsSync(env)) return env;
  const candidates = [
    join(ARTIFACT_DIR, 'HY-MT1.5-1.8B-Q4_K_M.gguf'),
    join(ARTIFACT_DIR, 'hy-mt-1.5-1.8b-q4_k_m.gguf'),
  ];
  for (const c of candidates) if (existsSync(c)) return c;
  return '';
}

function renderTranslatePrompt(text, targetLang = 'en', sourceLang = 'zh-CHS', glossary = '') {
  const raw = readFileSync(PROMPT_FILE, 'utf8');
  const marker = raw.indexOf('以下为发送给模型的正文');
  let body = marker >= 0 ? raw.slice(marker) : raw;
  const nl = body.indexOf('\n');
  if (nl >= 0) body = body.slice(nl + 1);
  const rendered = body
    .replaceAll('{text}', text)
    .replaceAll('{targetLang}', targetLang)
    .replaceAll('{sourceLang}', sourceLang)
    .replaceAll('{glossary}', glossary);
  for (const v of ['text', 'targetLang', 'sourceLang', 'glossary']) {
    if (rendered.includes(`{${v}}`)) throw new Error(`提示词未替换变量 {${v}}`);
  }
  return rendered.trim();
}

function renderVariantPrompt(variant, text) {
  if (variant === 'product') return renderTranslatePrompt(text);
  if (variant === 'minimal') {
    return `将下面的中文翻译成英文，只输出译文：\n\n${text}`;
  }
  if (variant === 'minimal-fewshot') {
    return (
      '将下面的中文翻译成英文，只输出译文：\n\n' +
      '示例：\n中文：今天天气不错。\n英文：The weather is nice today.\n\n' +
      `现在翻译：\n${text}`
    );
  }
  throw new Error(`未知 prompt variant：${variant}（支持 product / minimal / minimal-fewshot）`);
}

function cjkRatio(text) {
  const chars = [...text];
  if (!chars.length) return 0;
  const cjk = chars.filter((c) => /[\u4e00-\u9fff]/.test(c)).length;
  return cjk / chars.length;
}

function sha256File(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function percentile(values, p) {
  if (!values.length) return NaN;
  const s = [...values].sort((a, b) => a - b);
  if (s.length === 1) return s[0];
  const k = (s.length - 1) * (p / 100);
  const lo = Math.floor(k);
  const hi = Math.min(lo + 1, s.length - 1);
  return s[lo] + (s[hi] - s[lo]) * (k - lo);
}

function summarize(values) {
  if (!values.length) return {};
  const mean = values.reduce((a, b) => a + b, 0) / values.length;
  return {
    count: values.length,
    mean_ms: round(mean),
    p50_ms: round(percentile(values, 50)),
    p95_ms: round(percentile(values, 95)),
    max_ms: round(Math.max(...values)),
    min_ms: round(Math.min(...values)),
  };
}

function round(x) {
  return Math.round(x * 100) / 100;
}

function psQuery(pid) {
  // 返回 {workingSet, privateMemory, peakWorkingSet}（字节）；进程不存在返回 null
  const script = `$p = Get-Process -Id ${pid} -ErrorAction SilentlyContinue; if ($p) { [pscustomobject]@{ workingSet = $p.WorkingSet64; privateMemory = $p.PrivateMemorySize64; peakWorkingSet = $p.PeakWorkingSet64; cpuSeconds = $p.CPU } | ConvertTo-Json -Compress } else { 'null' }`;
  const r = spawnSync('powershell.exe', ['-NoProfile', '-Command', script], {
    encoding: 'utf8',
    timeout: 15000,
  });
  if (r.status !== 0 || !r.stdout) return null;
  try {
    return JSON.parse(r.stdout.trim());
  } catch {
    return null;
  }
}

function hostInfo() {
  const cpu = cpus();
  return {
    os: `${platform()} ${release()}`,
    cpu_model: (cpu[0] && cpu[0].model) || 'unknown',
    logical_cpus: cpu.length,
    total_mem_gb: round(Number((process.env.TOTAL_MEM_GB || 0)) || 0) || null,
    free_mem_gb_before: round(freemem() / 1024 ** 3),
    node: process.version,
  };
}

// ------------------------------------------------------------- llama-server --
class LlamaServer {
  constructor(opts) {
    this.opts = opts;
    this.proc = null;
    this.logLines = [];
    this.loadStart = 0;
    this.healthReadyAt = 0;
  }

  start() {
    const args = [
      '-m',
      this.opts.model,
      '-c',
      String(this.opts.ctx),
      '-t',
      String(this.opts.threads),
      '--port',
      String(this.opts.port),
      '--host',
      this.opts.host,
      '--no-webui',
    ];
    if (this.opts.gpuLayers > 0) args.push('-ngl', String(this.opts.gpuLayers));
    const logPath = this.opts.logFile;
    if (logPath) {
      mkdirSync(dirname(logPath), { recursive: true });
      appendFileSync(
        logPath,
        `\n===== llama-server start ${new Date().toISOString()} =====\ncmd: ${this.opts.exe} ${args.join(' ')}\n`,
        'utf8',
      );
    }
    this.loadStart = performance.now();
    this.proc = spawn(this.opts.exe, args, {
      cwd: dirname(this.opts.exe),
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const collect = (chunk) => {
      const text = chunk.toString('utf8');
      for (const line of text.split(/\r?\n/)) {
        if (line.trim()) this.logLines.push(line.trim());
      }
      if (logPath) {
        try {
          appendFileSync(logPath, text, 'utf8');
        } catch {
          /* 日志写入失败不影响测量 */
        }
      }
    };
    this.proc.stdout.on('data', collect);
    this.proc.stderr.on('data', collect);
    this.exitPromise = new Promise((res) => this.proc.on('exit', (code, sig) => res({ code, sig })));
    return this.loadStart;
  }

  async waitHealthy(timeoutMs = 180000) {
    const url = `http://${this.opts.host}:${this.opts.port}/health`;
    const deadline = performance.now() + timeoutMs;
    let lastErr = '';
    while (performance.now() < deadline) {
      if (this.proc.exitCode !== null) {
        throw new Error(`llama-server 提前退出（code=${this.proc.exitCode}）：${this.tailLog()}`);
      }
      try {
        const res = await fetch(url, { signal: AbortSignal.timeout(3000) });
        const text = await res.text();
        if (res.status === 200) {
          this.healthReadyAt = performance.now();
          return { ok: true, healthMs: this.healthReadyAt - this.loadStart, body: text };
        }
        lastErr = `HTTP ${res.status} ${text.slice(0, 100)}`;
      } catch (err) {
        lastErr = `${err.name}: ${err.message}`;
      }
      await sleep(100);
    }
    throw new Error(`等待 /health 超时（${timeoutMs}ms）：${lastErr}\n${this.tailLog()}`);
  }

  tailLog(n = 40) {
    return this.logLines.slice(-n).join('\n');
  }

  async chat(messages, maxTokens) {
    const url = `http://${this.opts.host}:${this.opts.port}/v1/chat/completions`;
    const payload = {
      model: 'hy-mt-1.5-1.8b-q4_k_m',
      messages,
      temperature: this.opts.temperature,
      top_p: this.opts.topP,
      top_k: this.opts.topK,
      repeat_penalty: this.opts.repeatPenalty,
      max_tokens: maxTokens,
      stream: false,
      cache_prompt: false,
    };
    const t0 = performance.now();
    const res = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
      signal: AbortSignal.timeout(this.opts.timeout),
    });
    const tHeaders = performance.now();
    const text = await res.text();
    const tEnd = performance.now();
    let obj = null;
    try {
      obj = JSON.parse(text);
    } catch {
      /* keep raw */
    }
    if (res.status !== 200) {
      const err = new Error(`HTTP ${res.status}: ${text.slice(0, 300)}`);
      err.status = res.status;
      throw err;
    }
    return {
      status: res.status,
      ttfb_ms: round(tHeaders - t0),
      total_ms: round(tEnd - t0),
      body: obj,
      raw: text,
    };
  }

  async stop() {
    if (!this.proc || this.proc.exitCode !== null) return { alreadyExited: true };
    this.proc.kill();
    const timer = new Promise((res) => setTimeout(() => res('timeout'), 5000));
    const exited = await Promise.race([this.exitPromise, timer]);
    if (exited === 'timeout') {
      spawnSync('taskkill', ['/PID', String(this.proc.pid), '/T', '/F'], { encoding: 'utf8' });
      await Promise.race([this.exitPromise, sleep(3000)]);
    }
    return { alreadyExited: false, exit: await Promise.race([this.exitPromise, sleep(1000).then(() => 'unknown')]) };
  }
}

// ------------------------------------------------------------------- main --
async function main() {
  const args = parseArgs(process.argv.slice(2));
  // 统一转绝对路径：llama-server 以自身目录为 cwd 启动，相对模型路径会加载失败
  const exeRaw = args.exe || findCandidateExe();
  const modelRaw = args.model || findCandidateModel();
  const exe = exeRaw ? resolve(exeRaw) : '';
  const model = modelRaw ? resolve(modelRaw) : '';
  if (!args.logFile) args.logFile = join(RAW_DIR, `llama_server_${dateStamp()}.log`);

  const precheck = {
    exe,
    exe_exists: Boolean(exe && existsSync(exe)),
    model,
    model_exists: Boolean(model && existsSync(model)),
    prompt_file: PROMPT_FILE,
    prompt_exists: existsSync(PROMPT_FILE),
  };
  if (args.selfCheck) {
    console.log(JSON.stringify(precheck, null, 2));
    return precheck.exe_exists && precheck.model_exists && precheck.prompt_exists ? EXIT_OK : EXIT_BLOCKED;
  }
  if (!precheck.exe_exists || !precheck.model_exists || !precheck.prompt_exists) {
    console.error('BLOCKED：本地 llama.cpp 基准缺少依赖。');
    console.error(`  llama-server.exe: ${exe || '(未找到)'} exists=${precheck.exe_exists}`);
    console.error(`  GGUF 模型:        ${model || '(未找到)'} exists=${precheck.model_exists}`);
    console.error('  GGUF 下载：https://modelscope.cn/models/Tencent-Hunyuan/HY-MT1.5-1.8B-GGUF/resolve/master/HY-MT1.5-1.8B-Q4_K_M.gguf');
    console.error('  llama-server：GitHub release（llama-bXXXXX-bin-win-cpu-x64.zip）或本机 llama.cpp 预编译；');
    console.error('  也可用 --exe/--model 或 LLAMA_SERVER_EXE/LLAMA_MODEL_GGUF 指定。');
    return EXIT_BLOCKED;
  }

  mkdirSync(RAW_DIR, { recursive: true });
  const text = args.text || SAMPLE_TEXT;
  const prompt = renderVariantPrompt(args.promptVariant, text);
  const messages = [{ role: 'user', content: prompt }];
  const server = new LlamaServer({
    exe,
    model,
    port: args.port,
    host: args.host,
    ctx: args.ctx,
    threads: args.threads,
    gpuLayers: args.gpuLayers,
    maxTokens: args.maxTokens,
    temperature: args.temperature,
    topP: args.topP,
    topK: args.topK,
    repeatPenalty: args.repeatPenalty,
    timeout: args.timeout,
    logFile: args.logFile,
  });

  console.log('='.repeat(74));
  console.log('S-05 本地翻译链路时延基准（llama.cpp llama-server）');
  console.log(`variant: ${args.promptVariant}  text_chars=${text.length}`);
  console.log(`exe:   ${exe}`);
  console.log(`model: ${model}`);
  console.log(`threads=${args.threads} ctx=${args.ctx} port=${args.port} ngl=${args.gpuLayers}`);
  console.log('='.repeat(74));

  let out = null;
  try {
    server.start();
    const health = await server.waitHealthy();
    const healthMs = health.healthMs;
    console.log(`[加载] 进程启动 → /health 就绪：${healthMs.toFixed(0)}ms（首次加载，另计）`);
    const memAfterLoad = psQuery(server.proc.pid);
    console.log(`[内存] 加载后 WorkingSet=${fmtBytes(memAfterLoad?.workingSet)} private=${fmtBytes(memAfterLoad?.privateMemory)}`);

    // 加载完成后再发首个请求，记录「模型已加载后的首个真实翻译」耗时（冷启动推理）
    const records = [];
    let failures = 0;
    let firstFailure = null;

    for (let i = 0; i < args.warmup; i++) {
      const rec = await runOnce(server, messages, args.maxTokens, `warmup-${i + 1}`);
      rec.warmup = true;
      records.push(rec);
    }
    for (let i = 0; i < args.runs; i++) {
      const rec = await runOnce(server, messages, args.maxTokens, `run-${i + 1}`);
      rec.warmup = false;
      records.push(rec);
      if (!rec.ok) {
        failures++;
        if (!firstFailure) firstFailure = rec;
      }
    }

    const memAfterRuns = psQuery(server.proc.pid);
    const measured = records.filter((r) => !r.warmup && r.ok);
    const totals = measured.map((r) => r.total_ms);
    const ttfb = measured.map((r) => r.ttfb_ms);
    const promptMs = measured.map((r) => r.server_prompt_ms).filter((v) => Number.isFinite(v));
    const decodeMs = measured.map((r) => r.server_predicted_ms).filter((v) => Number.isFinite(v));
    const tokPerSec = measured.map((r) => r.tok_per_sec).filter((v) => Number.isFinite(v));
    const completionTokens = measured.map((r) => r.completion_tokens).filter((v) => Number.isFinite(v));

    const summary = {
      measured_runs: measured.length,
      failures,
      failure_rate: args.runs ? round(failures / args.runs) : null,
      total_ms: summarize(totals),
      ttfb_ms: summarize(ttfb),
      server_prompt_ms: summarize(promptMs),
      server_predicted_ms: summarize(decodeMs),
      tok_per_sec: summarize(tokPerSec),
      completion_tokens: summarize(completionTokens),
      first_failure: firstFailure,
    };
    const p95 = summary.total_ms.p95_ms;
    const verdict = p95 <= THRESHOLD_MS ? 'DONE' : 'FAILED';

    out = {
      meta: {
        script: 'bench_local_translate.mjs',
        script_version: SCRIPT_VERSION,
        generated_at: new Date().toISOString(),
        mode: 'REAL_LOCAL_LLAMA_CPP',
        host: hostInfo(),
        repo_commit: gitCommit(),
        threshold_ms: THRESHOLD_MS,
        sample_len_chars: text.length,
        sample_text_sha256: createHash('sha256').update(text).digest('hex'),
        prompt_variant: args.promptVariant,
        prompt_file: 'resources/prompts/translate.md',
        prompt_sha256: sha256File(PROMPT_FILE),
        prompt_chars: prompt.length,
        exe,
        exe_version: exeVersion(exe),
        model_path: model,
        model_sha256: sha256File(model),
        model_size_bytes: existsSync(model) ? readFileSync(model).length : null,
        server_args: {
          ctx: args.ctx,
          threads: args.threads,
          port: args.port,
          host: args.host,
          gpu_layers: args.gpuLayers,
          max_tokens: args.maxTokens,
        },
        sampling: {
          temperature: args.temperature,
          top_p: args.topP,
          top_k: args.topK,
          repeat_penalty: args.repeatPenalty,
          max_tokens: args.maxTokens,
          note: 'product 默认 provider.temperature=0.2；本地引擎沿用该温度以保证可复现，基准按实际参数记录。',
        },
        health_ready_ms: round(healthMs),
        memory_after_load: memAfterLoad,
        memory_after_runs: memAfterRuns,
        idle_unload: {
          testable: false,
          reason:
            'idleUnloadSec=600 是 SnipAsk 应用层策略；llama-server 本身没有空闲卸载机制，' +
            '本机未运行应用，无法观测「600s 空闲后卸载」行为，仅记录模型常驻内存与进程未退出。',
        },
        server_log_tail: server.tailLog(30),
      },
      records,
      summary,
      verdict,
    };

    console.log('-'.repeat(74));
    console.log(
      `[客户端] 总耗时 P50 ${summary.total_ms.p50_ms}ms | P95 ${summary.total_ms.p95_ms}ms | max ${summary.total_ms.max_ms}ms`,
    );
    console.log(
      `[服务端] prefill P50 ${summary.server_prompt_ms.p50_ms}ms | decode P50 ${summary.server_predicted_ms.p50_ms}ms | ${summary.tok_per_sec.p50_ms} tok/s`,
    );
    console.log(`[内存] 峰值 WorkingSet=${fmtBytes(memAfterRuns?.peakWorkingSet)}（任务管理器/Get-Process）`);
    console.log(`[判定] ${verdict}（P95 ${p95}ms vs 阈值 ${THRESHOLD_MS}ms；首次加载 ${healthMs.toFixed(0)}ms 另计）`);
  } catch (err) {
    out = {
      meta: {
        script: 'bench_local_translate.mjs',
        generated_at: new Date().toISOString(),
        mode: 'REAL_LOCAL_LLAMA_CPP',
        exe,
        model,
        host: hostInfo(),
      },
      error: `${err.name}: ${err.message}`,
      server_log_tail: server.tailLog(60),
      verdict: 'BLOCKED',
    };
    console.error(`测量失败：${err.message}`);
  } finally {
    const stopInfo = await server.stop();
    if (out) out.meta.shutdown = stopInfo;
  }

  const outPath = args.out || join(RAW_DIR, `local_translate_${dateStamp()}.json`);
  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, JSON.stringify(out, null, 2), 'utf8');
  console.log(`原始数据：${outPath}`);
  console.log('-'.repeat(74));
  return out && out.verdict && out.verdict !== 'BLOCKED' ? EXIT_OK : EXIT_BLOCKED;
}

async function runOnce(server, messages, maxTokens, label) {
  try {
    const res = await server.chat(messages, maxTokens);
    const body = res.body || {};
    const choice = (body.choices || [{}])[0] || {};
    const content = (choice.message && choice.message.content) || '';
    const usage = body.usage || {};
    const timings = body.timings || {};
    return {
      run: label,
      ok: true,
      status: res.status,
      total_ms: res.total_ms,
      ttfb_ms: res.ttfb_ms,
      server_prompt_ms: numOrNaN(timings.prompt_ms),
      server_predicted_ms: numOrNaN(timings.predicted_ms),
      tok_per_sec: numOrNaN(timings.predicted_per_second),
      prompt_tokens: usage.prompt_tokens ?? null,
      completion_tokens: usage.completion_tokens ?? null,
      output_chars: content.length,
      output_cjk_ratio: Math.round(cjkRatio(content) * 1000) / 1000,
      looks_translated: content.length > 0 && cjkRatio(content) < 0.2,
      output_sample: content.slice(0, 400),
      output_sha256: createHash('sha256').update(content).digest('hex'),
    };
  } catch (err) {
    return {
      run: label,
      ok: false,
      error: `${err.name}: ${err.message}`,
      status: err.status ?? null,
      total_ms: null,
      ttfb_ms: null,
    };
  }
}

function numOrNaN(v) {
  return typeof v === 'number' ? v : NaN;
}

function fmtBytes(b) {
  if (typeof b !== 'number') return 'n/a';
  return `${(b / 1024 ** 3).toFixed(2)} GiB (${b} B)`;
}

function dateStamp() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
}

function gitCommit() {
  const r = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: REPO_ROOT, encoding: 'utf8', timeout: 10000 });
  return r.status === 0 ? r.stdout.trim() : 'unknown';
}

function exeVersion(exe) {
  try {
    const r = spawnSync(exe, ['--version'], { encoding: 'utf8', timeout: 20000, cwd: dirname(exe) });
    return ((r.stdout || '') + (r.stderr || '')).split(/\r?\n/).filter(Boolean).slice(0, 3).join(' | ');
  } catch {
    return 'unknown';
  }
}

main()
  .then((code) => process.exit(code))
  .catch((err) => {
    console.error(`脚本异常：${err.stack || err}`);
    process.exit(3);
  });
