/** 结果窗顶栏：拖拽区、动作标题、轮次/模式/引擎徽标、AI 生成标注、复制与关闭 */

import type { Mode } from "@/shared/contract/types";
import { engineLabel, engineTitle } from "../shared/engineLabel";

export interface ResultHeaderProps {
  title: string;
  mode: Mode | null;
  engine: string | null;
  /** 是否展示「AI 生成」标注（复制文字是原文，不展示）。 */
  aiGenerated: boolean;
  /** 线程轮数（>1 时显示「第 N 轮」徽标）；0 表示尚未开始。 */
  round: number;
  browserPreview: boolean;
  canCopy: boolean;
  copied: boolean;
  onCopy: () => void;
  onClose: () => void;
}

const MODE_LABELS: Record<Mode, string> = { quick: "快速", deep: "深度" };

export function ResultHeader(props: ResultHeaderProps) {
  return (
    <header className="result-header" data-tauri-drag-region>
      <div className="result-header-main" data-tauri-drag-region>
        <h1 className="result-title">{props.title}</h1>
        {props.round > 1 ? <span className="result-chip">第 {props.round} 轮</span> : null}
        {props.mode ? <span className="result-chip">{MODE_LABELS[props.mode]}</span> : null}
        {props.engine ? (
          <span className="result-chip is-mono" title={engineTitle(props.engine)}>
            {engineLabel(props.engine)}
          </span>
        ) : null}
        {props.aiGenerated ? (
          <span className="result-ai-badge" title="内容由 AI 生成，请自行核实">
            AI 生成
          </span>
        ) : null}
        {props.browserPreview ? <span className="result-chip">预览</span> : null}
      </div>
      <div className="result-header-actions">
        <button
          type="button"
          className="result-iconbtn"
          disabled={!props.canCopy}
          onClick={props.onCopy}
          title="复制全文"
        >
          {props.copied ? "已复制" : "复制"}
        </button>
        <button
          type="button"
          className="result-iconbtn"
          onClick={props.onClose}
          title="关闭"
          aria-label="关闭"
        >
          ✕
        </button>
      </div>
    </header>
  );
}
