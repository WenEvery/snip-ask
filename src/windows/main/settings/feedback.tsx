/** 设置页反馈类局部组件（保存栏 / 二次确认 / 错误提示 / 状态标签） */

import { useState, type ReactNode } from "react";
import { t } from "@/i18n";
import { Button } from "@/components/ui";
import { fallbackLabel } from "./presets";
import type { UiError } from "./types";

export interface SaveBarProps {
  dirty: boolean;
  saving: boolean;
  label: string;
  onSave: () => void;
  disabled?: boolean;
}

export function SaveBar(props: SaveBarProps) {
  return (
    <div className="sa-savebar">
      <span className={`sa-savebar-state${props.dirty ? " is-dirty" : ""}`}>
        {props.dirty ? t("有未保存的修改") : t("本组设置已保存")}
      </span>
      <Button
        variant="primary"
        size="sm"
        disabled={!props.dirty || props.saving || props.disabled}
        onClick={props.onSave}
      >
        {props.saving ? t("保存中…") : t("应用{label}", { label: props.label })}
      </Button>
    </div>
  );
}

export interface ConfirmActionProps {
  children: ReactNode;
  description: string;
  confirmLabel?: string;
  onConfirm: () => void;
  disabled?: boolean;
}

/** 两段式危险操作确认（恢复默认 / 删除等） */
export function ConfirmAction(props: ConfirmActionProps) {
  const [confirming, setConfirming] = useState(false);
  if (!confirming) {
    return (
      <Button
        variant="danger"
        size="sm"
        disabled={props.disabled}
        onClick={() => setConfirming(true)}
      >
        {props.children}
      </Button>
    );
  }
  return (
    <span className="sa-confirm">
      <span className="sa-confirm-text">{props.description}</span>
      <Button
        variant="danger"
        size="sm"
        disabled={props.disabled}
        onClick={() => {
          setConfirming(false);
          props.onConfirm();
        }}
      >
        {props.confirmLabel ?? t("确认执行")}
      </Button>
      <Button variant="ghost" size="sm" onClick={() => setConfirming(false)}>
        {t("取消")}
      </Button>
    </span>
  );
}

export interface ErrorNoticeProps {
  error: UiError | null;
  onDismiss?: () => void;
}

/** 错误提示：message + code + 可折叠 detail + fallback 文案 */
export function ErrorNotice(props: ErrorNoticeProps) {
  const [open, setOpen] = useState(false);
  if (!props.error) return null;
  const { error } = props;
  return (
    <div className="sa-error" role="alert">
      <div className="sa-error-head">
        <strong className="sa-error-message">{error.message}</strong>
        <code className="sa-error-code">{error.code}</code>
        {error.retryable ? <span className="sa-error-retry">{t("可重试")}</span> : null}
        {error.detail ? (
          <button type="button" className="sa-linkbtn" onClick={() => setOpen(!open)}>
            {open ? t("收起详情") : t("查看详情")}
          </button>
        ) : null}
        {props.onDismiss ? (
          <button type="button" className="sa-linkbtn" onClick={props.onDismiss}>
            {t("关闭")}
          </button>
        ) : null}
      </div>
      {open && error.detail ? <pre className="sa-error-detail">{error.detail}</pre> : null}
      {error.fallback.length > 0 ? (
        <p className="sa-error-fallback">
          {t("可以试试：{list}", {
            list: error.fallback.map((item) => fallbackLabel(item)).join("、"),
          })}
        </p>
      ) : null}
    </div>
  );
}

/** 小型状态标签 */
export function StatusTag(props: {
  tone: "neutral" | "ok" | "warn" | "danger";
  children: ReactNode;
}) {
  return <span className={`sa-tag sa-tag-${props.tone}`}>{props.children}</span>;
}
