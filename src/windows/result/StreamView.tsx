/**
 * 增量渲染纯文本：保留换行/段落，支持极简内联格式（`**粗体**`、`` `代码` ``）。
 * 不引入 markdown 库；自动滚动由结果窗外壳负责。
 */

import type { ReactNode } from "react";

export interface StreamViewProps {
  text: string;
  reasoning: string;
  meta: string[];
  streaming: boolean;
}

export function StreamView(props: StreamViewProps) {
  const lines = props.text.split("\n");
  return (
    <div className="result-stream">
      {props.reasoning ? (
        <details className="result-reasoning">
          <summary>思考过程（{props.reasoning.length} 字）</summary>
          <pre className="result-reasoning-text">{props.reasoning}</pre>
        </details>
      ) : null}

      {props.meta.length > 0 ? (
        <div className="result-meta-lines">
          {props.meta.map((line, index) => (
            <p key={`${index}-${line.slice(0, 12)}`} className="result-meta-line">
              {line}
            </p>
          ))}
        </div>
      ) : null}

      {props.text ? (
        <div className="result-text">
          {lines.map((line, index) => (
            <p key={index} className="result-line">
              {renderInline(line)}
              {props.streaming && index === lines.length - 1 ? (
                <span className="result-cursor" />
              ) : null}
            </p>
          ))}
        </div>
      ) : props.streaming ? (
        <p className="result-line">
          <span className="result-cursor" />
        </p>
      ) : null}
    </div>
  );
}

/** 极简内联格式：**粗体** / `代码`，其余按纯文本渲染 */
function renderInline(line: string): ReactNode[] {
  if (!line) return [];
  const parts = line.split(/(\*\*[^*]+\*\*|`[^`]+`)/g);
  return parts
    .filter((part) => part !== "")
    .map((part, index) => {
      if (part.startsWith("**") && part.endsWith("**") && part.length > 4) {
        return <strong key={index}>{part.slice(2, -2)}</strong>;
      }
      if (part.startsWith("`") && part.endsWith("`") && part.length > 2) {
        return <code key={index}>{part.slice(1, -1)}</code>;
      }
      return <span key={index}>{part}</span>;
    });
}
