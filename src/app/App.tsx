/**
 * 窗口分发：按 `?window=`（Tauri 窗口 label）渲染 main / overlay / result。
 * 单入口（index.html），不引入 react-router。
 */

import { getWindowLabel } from "../shared/ipc";
import MainWindow from "../windows/main/MainWindow";
import OverlayWindow from "../windows/overlay/OverlayWindow";
import ResultWindow from "../windows/result/ResultWindow";
import "./app.css";

export default function App() {
  const label = getWindowLabel();
  switch (label) {
    case "main":
      return <MainWindow />;
    case "overlay":
      return <OverlayWindow />;
    case "result":
      return <ResultWindow />;
    default:
      return <UnknownWindow label={label} />;
  }
}

function UnknownWindow({ label }: { label: string }) {
  const href = typeof window !== "undefined" ? window.location.href : "";
  return (
    <div className="sa-unknown">
      <div className="sa-unknown__card sa-anim-slide">
        <h1 className="sa-unknown__title">未知窗口：{label || "(空)"}</h1>
        <p className="sa-unknown__desc">
          前端入口按 URL 参数 <code>?window=</code> 分发窗口，可用值如下：
        </p>
        <ul className="sa-unknown__list">
          <li>
            <code>?window=main</code> —— 设置页 + 首启引导
          </li>
          <li>
            <code>?window=overlay</code> —— 冻结层 + 框选 + 动作菜单
          </li>
          <li>
            <code>?window=result</code> —— 流式结果窗
          </li>
        </ul>
        <p className="sa-unknown__current">
          当前地址：<code>{href}</code>
        </p>
      </div>
    </div>
  );
}
