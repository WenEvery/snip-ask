/**
 * S-17 本地 HTTP mock（OpenAI 兼容 `/models`），用于在**真实 Tauri 应用**里制造
 * 401/402/429/500/超时/非 JSON 等网络与额度异常。
 *
 * 路由：`GET /c/<scenario>/models` —— scenario 决定响应：
 *   200 / 401 / 403 / 402 / 429 / 404 / 400 / 408 / 500 / 503  对应 HTTP 状态码
 *   200badjson  200 但响应体不是 JSON（验证 2xx 非 JSON 仍 ok）
 *   slow        3s 后返回 200（验证 latencyMs 真实测量，不触发 15s 超时）
 *   hang        不返回任何响应（客户端 15s 总超时会得到 SA-3005）
 *   empty       200 但 data 为空数组
 * 其它路径（含 `/health`）返回 200 文本，用于就绪探测。
 *
 * 请求头只记录「是否存在 / 方案 / UA / Accept」，不记录 Key 明文（即便测试用假 Key）。
 *
 * 用法：
 *   node scripts/spikes/s13_s17_s19_s20/mock_provider.mjs --port 39117 \
 *     --log .verify/spikes/S-13-S-17-S-19-S-20/raw/s17/mock-requests.jsonl
 *
 * 退出：Ctrl+C / SIGTERM / 父进程关闭管道；脚本自身不常驻（测完必须 kill）。
 */
import { appendFileSync, mkdirSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, resolve } from "node:path";

const argv = process.argv.slice(2);
function argOf(name, fallback) {
  const index = argv.indexOf(`--${name}`);
  return index >= 0 && argv[index + 1] ? argv[index + 1] : fallback;
}
const port = Number(argOf("port", "39117"));
const logPath = resolve(argOf("log", ".verify/spikes/S-13-S-17-S-19-S-20/raw/s17/mock-requests.jsonl"));
mkdirSync(dirname(logPath), { recursive: true });

const STATUS = {
  "200": 200,
  "401": 401,
  "403": 403,
  "402": 402,
  "429": 429,
  "404": 404,
  "400": 400,
  "408": 408,
  "500": 500,
  "503": 503,
};

function log(record) {
  appendFileSync(logPath, `${JSON.stringify(record)}\n`);
}

const server = createServer((req, res) => {
  const url = new URL(req.url ?? "/", `http://127.0.0.1:${port}`);
  const match = /^\/c\/([a-z0-9]+)\/models$/.exec(url.pathname);
  const authorization = String(req.headers.authorization ?? "");
  const record = {
    ts: new Date().toISOString(),
    method: req.method,
    path: url.pathname,
    scenario: match ? match[1] : null,
    headers: {
      authorizationPresent: authorization.length > 0,
      authorizationScheme: authorization.split(" ")[0] ?? null,
      // Key 本身不落盘；只记录长度（假 Key，无泄漏风险，仍按最小化原则）。
      authorizationLength: authorization.length,
      userAgent: String(req.headers["user-agent"] ?? ""),
      accept: String(req.headers.accept ?? ""),
    },
  };

  if (!match) {
    record.handled = "health";
    log(record);
    res.writeHead(200, { "content-type": "text/plain" });
    res.end("mock-provider-ok\n");
    return;
  }

  const scenario = match[1];
  record.handled = scenario;
  log(record);

  const sendJson = (status, body) => {
    res.writeHead(status, { "content-type": "application/json", "x-mock-scenario": scenario });
    res.end(JSON.stringify(body));
  };

  if (scenario in STATUS) {
    const status = STATUS[scenario];
    record.status = status;
    sendJson(
      status,
      status === 200
        ? { object: "list", data: [{ id: "mock-model-a", object: "model" }, { id: "mock-model-b", object: "model" }] }
        : { error: { message: `mock ${status}`, type: "mock_error", code: `mock_${status}` } },
    );
    return;
  }
  if (scenario === "200badjson") {
    record.status = 200;
    res.writeHead(200, { "content-type": "text/plain", "x-mock-scenario": scenario });
    res.end("this is not json");
    return;
  }
  if (scenario === "empty") {
    record.status = 200;
    sendJson(200, { object: "list", data: [] });
    return;
  }
  if (scenario === "slow") {
    record.status = 200;
    record.delayMs = 3000;
    setTimeout(() => sendJson(200, { object: "list", data: [{ id: "slow-model", object: "model" }] }), 3000);
    return;
  }
  if (scenario === "hang") {
    record.hung = true;
    // 不响应：让客户端连接 5s / 总 15s 超时生效；60s 后强制断开，避免悬挂 socket 残留。
    setTimeout(() => {
      try {
        res.destroy();
      } catch {
        /* ignore */
      }
    }, 60000).unref();
    return;
  }
  record.status = 404;
  sendJson(404, { error: { message: "unknown mock scenario", type: "mock_error" } });
});

server.listen(port, "127.0.0.1", () => {
  console.log(`MOCK_READY port=${port} log=${logPath}`);
});

function shutdown() {
  server.close(() => process.exit(0));
  setTimeout(() => process.exit(0), 1500).unref();
}
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
process.on("SIGHUP", shutdown);
