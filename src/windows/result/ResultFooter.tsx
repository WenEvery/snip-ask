/** 结果窗底部：追问输入（Enter 发送 / Shift+Enter 换行）、重答、流式中取消 */

export interface ResultFooterProps {
  streaming: boolean;
  followup: string;
  sending: boolean;
  onFollowupChange: (value: string) => void;
  onSubmit: () => void;
  onRerun: () => void;
  onCancel: () => void;
}

export function ResultFooter(props: ResultFooterProps) {
  return (
    <footer className="result-footer">
      <div className="result-footer-actions">
        {props.streaming ? (
          <button type="button" className="result-btn" onClick={props.onCancel}>
            取消生成
          </button>
        ) : (
          <button type="button" className="result-btn" onClick={props.onRerun}>
            重答
          </button>
        )}
      </div>
      <div className="result-followup">
        <textarea
          className="result-input"
          rows={2}
          value={props.followup}
          placeholder="继续追问…（Enter 发送，Shift+Enter 换行）"
          disabled={props.sending}
          onChange={(event) => props.onFollowupChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              props.onSubmit();
            }
          }}
        />
        <button
          type="button"
          className="result-send"
          disabled={props.sending || !props.followup.trim()}
          onClick={props.onSubmit}
        >
          {props.sending ? "发送中…" : "发送"}
        </button>
      </div>
    </footer>
  );
}
