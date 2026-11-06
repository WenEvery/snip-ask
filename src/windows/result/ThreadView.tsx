/**
 * 线程式结果视图（契约 v4 / 研究 §3.6）：
 * - 每轮 = 「问」气泡 + 「答」（AI）气泡；
 * - 历史轮次默认折叠为单行摘要（点击展开），仅最后一轮展开渲染；
 * - 仅最后一轮可以处于流式；错误内联在对应轮次，不影响其它轮次。
 */

import type { ResultTurn } from "./thread";
import { summarize } from "./thread";
import { StreamView } from "./StreamView";
import { SourceList } from "./SourceList";
import { fallbackText, formatCost, formatLatency } from "./helpers";
import { engineLabel, engineTitle } from "../shared/engineLabel";

export interface ThreadViewProps {
  turns: ResultTurn[];
  /** 来源链接打开失败（沿用结果窗的错误展示）。 */
  onSourceError: (error: unknown) => void;
}

export function ThreadView(props: ThreadViewProps) {
  const lastIndex = props.turns.length - 1;
  return (
    <div className="result-thread">
      {props.turns.map((turn, index) => (
        <TurnBubble
          key={turn.task}
          turn={turn}
          latest={index === lastIndex}
          round={index + 1}
          onSourceError={props.onSourceError}
        />
      ))}
    </div>
  );
}

interface TurnBubbleProps {
  turn: ResultTurn;
  /** 是否是最后一轮（展开渲染 + 允许流式光标）。 */
  latest: boolean;
  round: number;
  onSourceError: (error: unknown) => void;
}

function TurnBubble(props: TurnBubbleProps) {
  const { turn } = props;
  const isCopy = turn.action === "copy";
  const body = <TurnBody turn={turn} latest={props.latest} onSourceError={props.onSourceError} />;
  const question = turn.question ? (
    <div className="result-turn-question">
      <span className="result-turn-role">问</span>
      <span className="result-turn-question-text">{turn.question}</span>
    </div>
  ) : null;

  if (props.latest) {
    return (
      <div className="result-turn is-latest">
        {question}
        <div className="result-turn-answer">
          <span className="result-turn-role is-ai">{isCopy ? "原文" : "答"}</span>
          {body}
        </div>
      </div>
    );
  }

  // 历史轮次：折叠为「第 N 轮 · 摘要」，点击展开完整回答。
  return (
    <details className="result-turn is-history">
      <summary className="result-turn-summary">
        <span className="result-turn-index">第 {props.round} 轮</span>
        <span className="result-turn-brief">
          {summarize(turn.question || turn.answer, 80) || "（无内容）"}
        </span>
        {turn.status === "error" ? <span className="result-turn-flag is-error">失败</span> : null}
        {turn.status === "cancelled" ? (
          <span className="result-turn-flag is-cancelled">已取消</span>
        ) : null}
      </summary>
      {question}
      <div className="result-turn-answer">
        <span className="result-turn-role is-ai">答</span>
        {body}
      </div>
    </details>
  );
}

function TurnBody(props: {
  turn: ResultTurn;
  latest: boolean;
  onSourceError: (error: unknown) => void;
}) {
  const { turn } = props;
  const isCopy = turn.action === "copy";
  const streaming = turn.status === "streaming";
  return (
    <>
      <StreamView
        text={turn.answer}
        reasoning={turn.reasoning}
        meta={turn.meta}
        streaming={streaming}
      />

      {turn.status === "error" && turn.error ? (
        <div className="result-error" role="alert">
          <div className="result-error-head">
            <strong className="result-error-message">{turn.error.message}</strong>
            <code className="result-error-code">{turn.error.code}</code>
            {turn.error.retryable ? <span className="result-hint">可重试</span> : null}
          </div>
          {turn.error.detail ? (
            <pre className="result-error-detail">{turn.error.detail}</pre>
          ) : null}
          {turn.error.fallback.length > 0 ? (
            <p className="result-error-fallback">可以试试：{fallbackText(turn.error.fallback)}</p>
          ) : null}
        </div>
      ) : null}

      {turn.status === "cancelled" ? <p className="result-hint">已取消本次生成。</p> : null}

      {turn.done && !isCopy ? (
        <div className="result-usage">
          <span>用时 {formatLatency(turn.done.latencyMs)}</span>
          <span>费用 {formatCost(turn.done.usage.costEstimate)}</span>
          <span title={engineTitle(turn.done.engine)}>{engineLabel(turn.done.engine)}</span>
        </div>
      ) : null}

      {turn.done && turn.done.sources.length > 0 ? (
        <SourceList sources={turn.done.sources} onError={props.onSourceError} />
      ) : null}
    </>
  );
}
