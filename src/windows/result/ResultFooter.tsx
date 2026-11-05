/**
 * 结果窗底部：追问输入（Enter 发送 / Shift+Enter 换行）、重答、清空线程、流式中取消。
 *
 * 流式/发送期间输入与发送按钮禁用（研究 §3.10-7：并发提交必须在 UI 侧挡住）。
 */

export interface ResultFooterProps {
  /** 是否有轮次正在流式生成。 */
  streaming: boolean;
  /** 输入区禁用（流式或发送中）。 */
  disabled: boolean;
  followup: string;
  sending: boolean;
  onFollowupChange: (value: string) => void;
  onSubmit: () => void;
  onRerun: () => void;
  onCancel: () => void;
  /** 清空当前线程（结束本次对话）。 */
  onClearThread: () => void;
}

export function ResultFooter(props: ResultFooterProps) {
  const placeholder = props.streaming
    ? "正在生成回答…（结束后可继续追问）"
    : "继续追问…（Enter 发送，Shift+Enter 换行）";
  return (
    <footer className="result-footer">
      <div className="result-footer-actions">
        {props.streaming ? (
          <button type="button" className="result-btn" onClick={props.onCancel}>
            取消生成
          </button>
        ) : (
          <>
            <button
              type="button"
              className="result-btn"
              disabled={props.disabled}
              onClick={props.onRerun}
            >
              重答
            </button>
            <button
              type="button"
              className="result-btn"
              disabled={props.disabled}
              title="结束这次对话，回到等待框选的状态"
              onClick={props.onClearThread}
            >
              结束对话
            </button>
          </>
        )}
      </div>
      <div className="result-followup">
        <textarea
          className="result-input"
          rows={2}
          value={props.followup}
          placeholder={placeholder}
          disabled={props.disabled}
          onChange={(event) => props.onFollowupChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              if (!props.disabled) props.onSubmit();
            }
          }}
        />
        <button
          type="button"
          className="result-send"
          disabled={props.disabled || !props.followup.trim()}
          onClick={props.onSubmit}
        >
          {props.sending ? "发送中…" : "发送"}
        </button>
      </div>
    </footer>
  );
}
