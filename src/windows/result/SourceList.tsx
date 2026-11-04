/** 检索来源列表：标题 / 站点 / 摘要，点击经 `open_external` 打开（仅 https 或 127.0.0.1） */

import type { SourceRef } from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { hostOf } from "./helpers";

export interface SourceListProps {
  sources: SourceRef[];
  onError?: (error: unknown) => void;
}

export function SourceList(props: SourceListProps) {
  if (props.sources.length === 0) return null;

  const open = async (url: string) => {
    try {
      await ipc.invoke<void>(COMMANDS.openExternal, { url });
    } catch (err) {
      props.onError?.(err);
    }
  };

  return (
    <section className="result-sources">
      <h2 className="result-sources-title">检索来源（{props.sources.length}）</h2>
      <ul className="result-source-list">
        {props.sources.map((source, index) => (
          <li key={`${source.url}-${index}`}>
            <button type="button" className="result-source" onClick={() => void open(source.url)} title={source.url}>
              <span className="result-source-title">{source.title || source.url}</span>
              <span className="result-source-meta">{source.siteName ?? hostOf(source.url)}</span>
              {source.snippet ? <span className="result-source-snippet">{source.snippet}</span> : null}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
